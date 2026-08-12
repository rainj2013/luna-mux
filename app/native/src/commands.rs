use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, atomic::AtomicBool},
    time::UNIX_EPOCH,
};

use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::Utc;
use serde_json::json;
use tauri::{AppHandle, State, Theme, WebviewWindow};
use tauri_plugin_dialog::{DialogExt, FilePath};
use tauri_plugin_opener::OpenerExt;
use uuid::Uuid;

use crate::{
    ai, app_icon,
    control_contract::{ControlCatalog, ControlEventReadResult, ControlRequest, ControlResponse},
    control_service::{self, InProcessControlService, LunaControlService},
    database::Database,
    models::*,
    product,
    sessions::SessionManager,
    ssh_config,
    ssh_terminal_backend::InProcessSshTerminalBackend,
    terminal_backend::TerminalBackend,
    terminal_runtime_contract::{
        TerminalRuntime, TerminalRuntimeAuthentication, TerminalRuntimeCreateRequest,
        TerminalRuntimeOutputReadResult, TerminalTarget,
    },
    transfers::TransferManager,
    tunnels::TunnelManager,
};

const LUNA_REMOTE_CONNECTION_ARCHIVE_FORMAT: &str = "luna-remote-connections";
const LEGACY_CONNECTION_ARCHIVE_FORMAT: &str = "ssh-client-connections";
const LUNA_REMOTE_IDENTIFIER: &str = "com.local.lunaremote";
const LUNA_REMOTE_DATABASE_FILE: &str = "luna-remote.db";
const LUNA_REMOTE_CREDENTIAL_SERVICE: &str = "com.local.lunaremote.credentials";
const LUNA_REMOTE_LEGACY_CREDENTIAL_SERVICE: &str = "com.local.sshclient.credentials";

pub struct AppState {
    pub db: Arc<Database>,
    pub sessions: Arc<SessionManager>,
    pub ssh_terminal_backend: Arc<InProcessSshTerminalBackend>,
    pub control: Arc<InProcessControlService>,
    pub transfers: Arc<TransferManager>,
    pub tunnels: Arc<TunnelManager>,
    pub ai_diagnostics: ai::AiDiagnostics,
    pub allowed_imports: Mutex<HashSet<PathBuf>>,
    pub pending_archive_imports: Mutex<HashMap<String, BookmarkArchive>>,
    pub pending_luna_remote_imports: Mutex<HashMap<String, LunaRemoteSnapshot>>,
    pub exit_cleanup_started: AtomicBool,
}

/// Tauri is the trusted local UI adapter. External Agent/CLI adapters must
/// authenticate independently and inject their own caller identity.
#[tauri::command]
pub fn control_catalog(state: State<AppState>) -> ControlCatalog {
    state.control.catalog(&control_service::ui_caller())
}

#[tauri::command]
pub async fn control_invoke(
    state: State<'_, AppState>,
    request: ControlRequest,
) -> Result<ControlResponse, serde_json::Value> {
    state
        .control
        .invoke(&control_service::ui_caller(), request)
        .await
        .map_err(|error| serde_json::to_value(error).unwrap_or_else(|_| serde_json::json!({"code":"internal","message":"控制请求失败","retryable":false})))
}

#[tauri::command]
pub fn control_read_events(
    state: State<AppState>,
    from_sequence: u64,
    limit: usize,
) -> Result<ControlEventReadResult, serde_json::Value> {
    state
        .control
        .read_events(&control_service::ui_caller(), from_sequence, limit)
        .map_err(|error| serde_json::to_value(error).unwrap_or_else(|_| serde_json::json!({"code":"internal","message":"事件读取失败","retryable":false})))
}

fn selected_path(value: Option<FilePath>) -> Option<String> {
    value?
        .into_path()
        .ok()
        .map(|path| path.to_string_lossy().into_owned())
}
fn require_session(session_id: Option<String>) -> Result<String, String> {
    session_id
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "SSH 会话未连接".into())
}

#[tauri::command]
pub fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "win32"
    } else {
        "linux"
    }
}
#[tauri::command]
pub fn system_open_external(app: AppHandle, value: String) -> Result<(), String> {
    let url = url::Url::parse(&value).map_err(|_| "链接格式不正确".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("只允许打开 HTTP 或 HTTPS 链接".into());
    }
    app.opener()
        .open_url(url.to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn sessions_connect(
    state: State<'_, AppState>,
    input: ConnectInput,
) -> Result<SessionSummary, String> {
    let target_id = InProcessSshTerminalBackend::target_id(&input.bookmark_id);
    if !input.new_session {
        if let Some(runtime) = state.ssh_terminal_backend.find_active_by_target(&target_id) {
            return InProcessSshTerminalBackend::legacy_summary(&runtime);
        }
    }
    let runtime = state
        .ssh_terminal_backend
        .create(TerminalRuntimeCreateRequest {
            target_id,
            title: None,
            cwd: None,
            command: None,
            authentication: Some(TerminalRuntimeAuthentication::Ssh {
                credential: input.credential,
                remember_credential: input.remember_credential,
                jump_credential: input.jump_credential,
                remember_jump_credential: input.remember_jump_credential,
            }),
            cols: 100,
            rows: 30,
        })
        .await?;
    InProcessSshTerminalBackend::legacy_summary(&runtime)
}

#[tauri::command]
pub async fn terminal_runtime_create(
    state: State<'_, AppState>,
    request: TerminalRuntimeCreateRequest,
) -> Result<TerminalRuntime, String> {
    state.ssh_terminal_backend.create(request).await
}

#[tauri::command]
pub fn terminal_targets_list(state: State<AppState>) -> Result<Vec<TerminalTarget>, String> {
    state.ssh_terminal_backend.targets()
}

#[tauri::command]
pub fn terminal_runtimes_list(state: State<AppState>) -> Result<Vec<TerminalRuntime>, String> {
    state.ssh_terminal_backend.list()
}

#[tauri::command]
pub fn terminal_runtime_read_output(
    state: State<AppState>,
    runtime_id: String,
    from_cursor: u64,
    max_bytes: usize,
) -> Result<TerminalRuntimeOutputReadResult, String> {
    state
        .ssh_terminal_backend
        .read_output(&runtime_id, from_cursor, max_bytes)
}

#[tauri::command]
pub async fn terminal_runtime_write(
    state: State<'_, AppState>,
    runtime_id: String,
    data: String,
) -> Result<(), String> {
    state.ssh_terminal_backend.write(&runtime_id, &data).await
}

#[tauri::command]
pub async fn terminal_runtime_resize(
    state: State<'_, AppState>,
    runtime_id: String,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    state
        .ssh_terminal_backend
        .resize(&runtime_id, cols, rows)
        .await
}

#[tauri::command]
pub fn terminal_runtime_flow(
    state: State<AppState>,
    runtime_id: String,
    paused: bool,
) -> Result<(), String> {
    state
        .ssh_terminal_backend
        .set_output_paused(&runtime_id, paused)
}

#[tauri::command]
pub async fn terminal_runtime_interrupt(
    state: State<'_, AppState>,
    runtime_id: String,
) -> Result<(), String> {
    state.ssh_terminal_backend.interrupt(&runtime_id).await
}

#[tauri::command]
pub async fn terminal_runtime_close(
    state: State<'_, AppState>,
    runtime_id: String,
) -> Result<(), String> {
    state.ssh_terminal_backend.close(&runtime_id).await
}
#[tauri::command]
pub async fn sessions_disconnect(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.tunnels.stop_session(&id).await;
    state.ssh_terminal_backend.close(&id).await
}
#[tauri::command]
pub async fn sessions_write(
    state: State<'_, AppState>,
    id: String,
    data: String,
) -> Result<(), String> {
    state.ssh_terminal_backend.write(&id, &data).await
}
#[tauri::command]
pub async fn sessions_resize(
    state: State<'_, AppState>,
    id: String,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    state.ssh_terminal_backend.resize(&id, cols, rows).await
}
#[tauri::command]
pub fn sessions_flow(state: State<AppState>, id: String, paused: bool) -> Result<(), String> {
    state.ssh_terminal_backend.set_output_paused(&id, paused)
}
#[tauri::command]
pub fn sessions_host_key_decision(state: State<AppState>, id: String, accept: bool) {
    state.sessions.host_key_decision(&id, accept);
}

#[tauri::command]
pub fn bookmarks_list(state: State<AppState>) -> Result<Vec<Bookmark>, String> {
    state.db.list_bookmarks()
}
#[tauri::command]
pub fn bookmarks_save(state: State<AppState>, input: BookmarkInput) -> Result<Bookmark, String> {
    state.db.save_bookmark(input)
}
#[tauri::command]
pub fn bookmarks_reorder(
    state: State<AppState>,
    ids: Vec<String>,
) -> Result<Vec<Bookmark>, String> {
    state.db.reorder_bookmarks(&ids)
}
#[tauri::command]
pub fn bookmarks_move_to_group(
    state: State<AppState>,
    id: String,
    group_name: String,
) -> Result<Vec<Bookmark>, String> {
    state.db.move_bookmark_to_group(&id, &group_name)
}
#[tauri::command]
pub fn bookmark_groups_list(state: State<AppState>) -> Result<Vec<String>, String> {
    state.db.list_bookmark_groups()
}
#[tauri::command]
pub fn bookmark_groups_create(state: State<AppState>, name: String) -> Result<Vec<String>, String> {
    state.db.create_bookmark_group(&name)
}
#[tauri::command]
pub fn bookmark_groups_rename(
    state: State<AppState>,
    old_name: String,
    new_name: String,
) -> Result<Vec<String>, String> {
    state.db.rename_bookmark_group(&old_name, &new_name)
}
#[tauri::command]
pub fn bookmark_groups_delete(
    state: State<AppState>,
    name: String,
) -> Result<BookmarkGroupDeleteResult, String> {
    state.db.delete_bookmark_group(&name)
}
#[tauri::command]
pub fn bookmark_groups_reorder(
    state: State<AppState>,
    groups: Vec<String>,
) -> Result<Vec<String>, String> {
    state.db.reorder_bookmark_groups(&groups)
}
#[tauri::command]
pub fn bookmarks_duplicate(state: State<AppState>, id: String) -> Result<Bookmark, String> {
    let source = state
        .db
        .get_bookmark(&id)?
        .ok_or_else(|| "要复制的连接不存在".to_string())?;
    let existing_names = state
        .db
        .list_bookmarks()?
        .into_iter()
        .map(|bookmark| bookmark.name)
        .collect::<HashSet<_>>();
    let base_name = format!("{} 副本", source.name);
    let mut name = base_name.clone();
    let mut suffix = 2;
    while existing_names.contains(&name) {
        name = format!("{base_name} {suffix}");
        suffix += 1;
    }
    let duplicate = state.db.save_bookmark(BookmarkInput {
        id: None,
        name,
        host: source.host,
        port: source.port,
        username: source.username,
        auth_type: source.auth_type,
        private_key_path: source.private_key_path,
        jump_bookmark_id: source.jump_bookmark_id,
        group_name: source.group_name,
        favorite: false,
        keepalive_enabled: source.keepalive_enabled,
        keepalive_interval_seconds: source.keepalive_interval_seconds,
        keepalive_count_max: source.keepalive_count_max,
        note: source.note,
    })?;
    let mut ids = state
        .db
        .list_bookmarks()?
        .into_iter()
        .map(|bookmark| bookmark.id)
        .filter(|bookmark_id| bookmark_id != &duplicate.id)
        .collect::<Vec<_>>();
    let position = ids
        .iter()
        .position(|bookmark_id| bookmark_id == &id)
        .map(|index| index + 1)
        .unwrap_or(ids.len());
    ids.insert(position, duplicate.id.clone());
    state.db.reorder_bookmarks(&ids)?;
    state
        .db
        .get_bookmark(&duplicate.id)?
        .ok_or_else(|| "复制连接失败".to_string())
}
#[tauri::command]
pub fn bookmarks_remove(state: State<AppState>, id: String) -> Result<(), String> {
    if let Some(dependent) = state
        .db
        .list_bookmarks()?
        .into_iter()
        .find(|item| item.jump_bookmark_id == id)
    {
        return Err(format!(
            "连接“{}”正在使用此跳板机，请先修改该连接",
            dependent.name
        ));
    }
    state.db.remove_bookmark(&id)
}
#[tauri::command]
pub fn bookmarks_forget_credential(state: State<AppState>, id: String) -> Result<(), String> {
    state.db.forget_credential(&id)
}

#[tauri::command]
pub async fn bookmarks_preview_ssh_config(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<SshConfigPreview>, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("导入 OpenSSH Config")
        .set_file_name(ssh_config::default_path().to_string_lossy())
        .pick_file(move |path| {
            let _ = sender.send(path);
        });
    let path = selected_path(receiver.await.ok().flatten());
    let Some(path) = path else {
        return Ok(None);
    };
    let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > 5 * 1024 * 1024 {
        return Err("SSH Config 必须是小于 5 MB 的普通文件".into());
    }
    let entries = ssh_config::parse(
        &fs::read_to_string(&path).map_err(|e| e.to_string())?,
        &dirs::home_dir().unwrap_or_default(),
    );
    state
        .allowed_imports
        .lock()
        .map_err(|_| "导入状态锁已损坏")?
        .insert(PathBuf::from(&path));
    Ok(Some(SshConfigPreview { path, entries }))
}

#[tauri::command]
pub fn bookmarks_import_ssh_config(
    state: State<AppState>,
    path: String,
    aliases: Vec<String>,
) -> Result<Vec<Bookmark>, String> {
    if !state
        .allowed_imports
        .lock()
        .map_err(|_| "导入状态锁已损坏")?
        .remove(Path::new(&path))
    {
        return Err("请重新选择 SSH Config 文件".into());
    }
    let selected: HashSet<String> = aliases
        .into_iter()
        .map(|value| value.to_ascii_lowercase())
        .collect();
    let entries: Vec<_> = ssh_config::parse(
        &fs::read_to_string(&path).map_err(|e| e.to_string())?,
        &dirs::home_dir().unwrap_or_default(),
    )
    .into_iter()
    .filter(|entry| selected.contains(&entry.alias.to_ascii_lowercase()))
    .collect();
    let existing = state.db.list_bookmarks()?;
    let mut imported = HashMap::<String, Bookmark>::new();
    for entry in &entries {
        let current = existing
            .iter()
            .find(|item| item.name.eq_ignore_ascii_case(&entry.alias));
        let saved = state.db.save_bookmark(BookmarkInput {
            id: current.map(|item| item.id.clone()),
            name: entry.name.clone(),
            host: entry.host.clone(),
            port: entry.port,
            username: entry.username.clone(),
            auth_type: if entry.private_key_path.is_empty() {
                AuthType::Agent
            } else {
                AuthType::PrivateKey
            },
            private_key_path: entry.private_key_path.clone(),
            jump_bookmark_id: String::new(),
            group_name: current
                .map(|item| item.group_name.clone())
                .unwrap_or_else(|| "SSH Config".into()),
            favorite: current.is_some_and(|item| item.favorite),
            keepalive_enabled: true,
            keepalive_interval_seconds: 15,
            keepalive_count_max: 3,
            note: current.map(|item| item.note.clone()).unwrap_or_default(),
        })?;
        imported.insert(entry.alias.to_ascii_lowercase(), saved);
    }
    for entry in &entries {
        if entry.proxy_jump_alias.is_empty() {
            continue;
        }
        let Some(saved) = imported.get(&entry.alias.to_ascii_lowercase()).cloned() else {
            continue;
        };
        let jump = imported
            .get(&entry.proxy_jump_alias.to_ascii_lowercase())
            .or_else(|| {
                existing
                    .iter()
                    .find(|item| item.name.eq_ignore_ascii_case(&entry.proxy_jump_alias))
            });
        if let Some(jump) =
            jump.filter(|jump| jump.id != saved.id && jump.jump_bookmark_id.is_empty())
        {
            let updated = state.db.save_bookmark(BookmarkInput {
                id: Some(saved.id.clone()),
                name: saved.name,
                host: saved.host,
                port: saved.port,
                username: saved.username,
                auth_type: saved.auth_type,
                private_key_path: saved.private_key_path,
                jump_bookmark_id: jump.id.clone(),
                group_name: saved.group_name,
                favorite: saved.favorite,
                keepalive_enabled: saved.keepalive_enabled,
                keepalive_interval_seconds: saved.keepalive_interval_seconds,
                keepalive_count_max: saved.keepalive_count_max,
                note: saved.note,
            })?;
            imported.insert(entry.alias.to_ascii_lowercase(), updated);
        }
    }
    Ok(imported.into_values().collect())
}

#[tauri::command]
pub fn files_home() -> String {
    dirs::home_dir()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}
#[tauri::command]
pub fn files_parent_local(path: String) -> String {
    Path::new(&path)
        .parent()
        .unwrap_or(Path::new(&path))
        .to_string_lossy()
        .into_owned()
}
#[tauri::command]
pub fn files_list_local(path: String) -> Result<Vec<DirectoryEntry>, String> {
    fs::read_dir(path)
        .map_err(|e| e.to_string())?
        .map(|entry| {
            let entry = entry.map_err(|e| e.to_string())?;
            let metadata = fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
            let kind = if metadata.file_type().is_symlink() {
                EntryKind::Symlink
            } else if metadata.is_dir() {
                EntryKind::Directory
            } else if metadata.is_file() {
                EntryKind::File
            } else {
                EntryKind::Other
            };
            Ok(DirectoryEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: entry.path().to_string_lossy().into_owned(),
                kind,
                size: metadata.is_file().then_some(metadata.len()),
                modified_at: metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map(|value| value.as_millis() as i64),
            })
        })
        .collect()
}
#[tauri::command]
pub async fn files_remote_home(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<String, String> {
    state.sessions.remote_home(&session_id).await
}
#[tauri::command]
pub async fn files_list_remote(
    state: State<'_, AppState>,
    session_id: String,
    path: String,
) -> Result<Vec<DirectoryEntry>, String> {
    state.sessions.list_remote(&session_id, path).await
}
#[tauri::command]
pub async fn files_create_directory(
    state: State<'_, AppState>,
    remote: bool,
    session_id: Option<String>,
    path: String,
) -> Result<(), String> {
    if remote {
        return state
            .sessions
            .create_remote_directory(&require_session(session_id)?, path)
            .await;
    }
    fs::create_dir(&path).map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn files_rename(
    state: State<'_, AppState>,
    remote: bool,
    session_id: Option<String>,
    from: String,
    to: String,
) -> Result<(), String> {
    if remote {
        return state
            .sessions
            .rename_remote(&require_session(session_id)?, from, to)
            .await;
    }
    fs::rename(from, to).map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn files_remove(
    state: State<'_, AppState>,
    remote: bool,
    session_id: Option<String>,
    paths: Vec<String>,
) -> Result<(), String> {
    if remote {
        return state
            .sessions
            .remove_remote(&require_session(session_id)?, paths)
            .await;
    }
    for path in paths {
        let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if metadata.is_dir() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        }
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn preview_file(path: &Path, position: PreviewPosition) -> Result<FilePreview, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    let length = size.min(1024 * 1024) as usize;
    if matches!(position, PreviewPosition::End) {
        file.seek(SeekFrom::Start(size.saturating_sub(length as u64)))
            .map_err(|e| e.to_string())?;
    }
    let mut data = vec![0; length];
    file.read_exact(&mut data).map_err(|e| e.to_string())?;
    let sample = &data[..data.len().min(8192)];
    let controls = sample
        .iter()
        .filter(|byte| **byte == 0 || **byte < 9 || (**byte > 13 && **byte < 32))
        .count();
    let binary = !sample.is_empty() && controls as f64 / sample.len() as f64 > 0.01;
    Ok(FilePreview {
        content: if binary {
            String::new()
        } else {
            String::from_utf8_lossy(&data).into_owned()
        },
        size,
        truncated: size > 1024 * 1024,
        position,
        binary,
    })
}

#[tauri::command]
pub async fn files_preview(
    state: State<'_, AppState>,
    remote: bool,
    session_id: Option<String>,
    path: String,
    position: PreviewPosition,
) -> Result<FilePreview, String> {
    if remote {
        return state
            .sessions
            .preview_remote(&require_session(session_id)?, path, position)
            .await;
    }
    preview_file(Path::new(&path), position)
}
#[tauri::command]
pub fn files_get_favorites(state: State<AppState>, bookmark_id: String) -> FavoritePaths {
    state.db.get_setting(
        &format!("fileFavorites:{bookmark_id}"),
        FavoritePaths {
            local: vec![],
            remote: vec![],
        },
    )
}
#[tauri::command]
pub fn files_set_favorites(
    state: State<AppState>,
    bookmark_id: String,
    mut value: FavoritePaths,
) -> Result<(), String> {
    value.local.sort();
    value.local.dedup();
    value.local.truncate(30);
    value.remote.sort();
    value.remote.dedup();
    value.remote.truncate(30);
    state
        .db
        .set_setting(&format!("fileFavorites:{bookmark_id}"), &value)
}
#[tauri::command]
pub async fn files_choose_local_directory(app: AppHandle) -> Option<String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("选择本地部署目录")
        .pick_folder(move |path| {
            let _ = sender.send(path);
        });
    selected_path(receiver.await.ok().flatten())
}
#[tauri::command]
pub async fn files_choose_private_key(app: AppHandle) -> Option<String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("选择 SSH 私钥")
        .pick_file(move |path| {
            let _ = sender.send(path);
        });
    selected_path(receiver.await.ok().flatten())
}

#[tauri::command]
pub fn state_get_sidebar_collapsed(state: State<AppState>) -> bool {
    state.db.get_setting("sidebarCollapsed", false)
}
#[tauri::command]
pub fn state_set_sidebar_collapsed(state: State<AppState>, collapsed: bool) -> Result<(), String> {
    state.db.set_setting("sidebarCollapsed", &collapsed)
}

#[tauri::command]
pub async fn bookmarks_export_archive(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let default = format!(
        "{}-connections-{}.json",
        product::PRODUCT_KEY,
        Utc::now().format("%Y-%m-%d")
    );
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("导出连接备份")
        .set_file_name(default)
        .add_filter(format!("{} 连接备份", product::DISPLAY_NAME), &["json"])
        .save_file(move |path| {
            let _ = sender.send(path);
        });
    let Some(path) = selected_path(receiver.await.ok().flatten()) else {
        return Ok(None);
    };
    let connections = state
        .db
        .list_bookmarks()?
        .into_iter()
        .map(|item| BookmarkArchiveEntry {
            id: item.id,
            name: item.name,
            host: item.host,
            port: item.port,
            username: item.username,
            auth_type: item.auth_type,
            private_key_path: item.private_key_path,
            jump_bookmark_id: item.jump_bookmark_id,
            group_name: item.group_name,
            favorite: item.favorite,
            keepalive_enabled: item.keepalive_enabled,
            keepalive_interval_seconds: item.keepalive_interval_seconds,
            keepalive_count_max: item.keepalive_count_max,
            note: item.note,
        })
        .collect();
    let archive = BookmarkArchive {
        format: product::CONNECTION_ARCHIVE_FORMAT.into(),
        version: 1,
        exported_at: Utc::now().to_rfc3339(),
        groups: state.db.list_bookmark_groups()?,
        connections,
    };
    fs::write(
        &path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&archive).map_err(|e| e.to_string())?
        ),
    )
    .map_err(|e| e.to_string())?;
    Ok(Some(path))
}

fn validate_bookmark_archive(archive: &BookmarkArchive) -> Result<BookmarkArchiveSource, String> {
    let source = match archive.format.as_str() {
        product::CONNECTION_ARCHIVE_FORMAT => BookmarkArchiveSource::LunaMux,
        LUNA_REMOTE_CONNECTION_ARCHIVE_FORMAT => BookmarkArchiveSource::LunaRemote,
        LEGACY_CONNECTION_ARCHIVE_FORMAT => BookmarkArchiveSource::Legacy,
        _ => return Err("不支持此连接备份格式或版本".into()),
    };
    if archive.version != 1 {
        return Err("不支持此连接备份格式或版本".into());
    }
    if archive.connections.len() > 10_000 || archive.groups.len() > 1_000 {
        return Err("连接备份包含的连接或分组数量过多".into());
    }
    let archive_ids = archive
        .connections
        .iter()
        .map(|item| item.id.as_str())
        .collect::<HashSet<_>>();
    if archive_ids.len() != archive.connections.len() {
        return Err("连接备份中存在重复的连接 ID".into());
    }
    if archive.connections.iter().any(|item| {
        item.id.trim().is_empty()
            || item.name.trim().is_empty()
            || item.host.trim().is_empty()
            || item.username.trim().is_empty()
            || item.port == 0
            || item.jump_bookmark_id == item.id
            || (!item.jump_bookmark_id.is_empty()
                && !archive_ids.contains(item.jump_bookmark_id.as_str()))
    }) {
        return Err("连接备份中存在无效的连接或跳板机引用".into());
    }
    let entries_by_id = archive
        .connections
        .iter()
        .map(|entry| (entry.id.as_str(), entry))
        .collect::<HashMap<_, _>>();
    if archive.connections.iter().any(|entry| {
        !entry.jump_bookmark_id.is_empty()
            && entries_by_id
                .get(entry.jump_bookmark_id.as_str())
                .is_some_and(|jump| !jump.jump_bookmark_id.is_empty())
    }) {
        return Err("连接备份包含多层跳板机，当前仅支持一层跳板机".into());
    }
    Ok(source)
}

#[tauri::command]
pub async fn bookmarks_preview_archive(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<BookmarkArchivePreview>, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("导入连接备份")
        .add_filter(format!("{} 连接备份", product::DISPLAY_NAME), &["json"])
        .pick_file(move |path| {
            let _ = sender.send(path);
        });
    let Some(path) = selected_path(receiver.await.ok().flatten()) else {
        return Ok(None);
    };
    let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > 10 * 1024 * 1024 {
        return Err("连接备份必须是小于 10 MB 的普通 JSON 文件".into());
    }
    let archive: BookmarkArchive =
        serde_json::from_str(&fs::read_to_string(&path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("连接备份格式无效：{e}"))?;
    let source = validate_bookmark_archive(&archive)?;
    let preview_id = Uuid::new_v4().to_string();
    let preview = BookmarkArchivePreview {
        preview_id: preview_id.clone(),
        path,
        source,
        exported_at: archive.exported_at.clone(),
        groups: archive.groups.clone(),
        connections: archive.connections.clone(),
        credentials_included: false,
    };
    let mut pending = state
        .pending_archive_imports
        .lock()
        .map_err(|_| "导入状态锁已损坏")?;
    pending.clear();
    pending.insert(preview_id, archive);
    Ok(Some(preview))
}

#[tauri::command]
pub fn bookmarks_import_archive(
    state: State<AppState>,
    preview_id: String,
    connection_ids: Vec<String>,
    groups: Vec<String>,
) -> Result<BookmarkArchiveImportResult, String> {
    let archive = state
        .pending_archive_imports
        .lock()
        .map_err(|_| "导入状态锁已损坏")?
        .get(&preview_id)
        .cloned()
        .ok_or_else(|| "导入预览已失效，请重新选择连接备份".to_string())?;
    validate_bookmark_archive(&archive)?;
    let selected_ids = connection_ids.into_iter().collect::<HashSet<_>>();
    let archive_ids = archive
        .connections
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<HashSet<_>>();
    if selected_ids.len() > archive.connections.len()
        || selected_ids
            .iter()
            .any(|id| !archive_ids.contains(id.as_str()))
    {
        return Err("导入选择包含未知连接，请重新预览".into());
    }
    let entries = archive
        .connections
        .iter()
        .filter(|entry| selected_ids.contains(&entry.id))
        .cloned()
        .collect::<Vec<_>>();
    if let Some(entry) = entries.iter().find(|entry| {
        !entry.jump_bookmark_id.is_empty() && !selected_ids.contains(&entry.jump_bookmark_id)
    }) {
        return Err(format!("连接“{}”依赖的跳板机未被选择", entry.name));
    }
    let allowed_groups = archive
        .groups
        .iter()
        .chain(archive.connections.iter().map(|entry| &entry.group_name))
        .map(|group| group.trim())
        .filter(|group| !group.is_empty())
        .collect::<HashSet<_>>();
    let mut requested_groups = Vec::new();
    for group in groups {
        let group = group.trim().to_string();
        if !group.is_empty() && !requested_groups.contains(&group) {
            requested_groups.push(group);
        }
    }
    if requested_groups
        .iter()
        .any(|group| !allowed_groups.contains(group.as_str()))
    {
        return Err("导入选择包含未知分组，请重新预览".into());
    }
    if entries.is_empty() && requested_groups.is_empty() {
        return Err("请至少选择一个连接或空分组".into());
    }
    let result = state
        .db
        .import_bookmark_archive(&entries, &requested_groups)?;
    state
        .pending_archive_imports
        .lock()
        .map_err(|_| "导入状态锁已损坏")?
        .remove(&preview_id);
    Ok(result)
}

fn default_luna_remote_database_path() -> Option<PathBuf> {
    dirs::data_dir().map(|path| {
        path.join(LUNA_REMOTE_IDENTIFIER)
            .join(LUNA_REMOTE_DATABASE_FILE)
    })
}

#[tauri::command]
pub fn bookmarks_discover_luna_remote_sources() -> Vec<LunaRemoteSource> {
    default_luna_remote_database_path()
        .filter(|path| path.is_file())
        .and_then(|path| {
            Database::read_luna_remote_snapshot(&path)
                .ok()
                .map(|snapshot| LunaRemoteSource {
                    path: snapshot.path,
                    source_modified_at: snapshot.source_modified_at,
                })
        })
        .into_iter()
        .collect()
}

fn store_luna_remote_preview(
    state: &AppState,
    snapshot: LunaRemoteSnapshot,
) -> Result<LunaRemoteImportPreview, String> {
    let preview_id = Uuid::new_v4().to_string();
    let preview = LunaRemoteImportPreview {
        preview_id: preview_id.clone(),
        path: snapshot.path.clone(),
        source_modified_at: snapshot.source_modified_at.clone(),
        groups: snapshot.groups.clone(),
        connections: snapshot.connections.clone(),
        known_hosts: snapshot.known_hosts.clone(),
        setting_keys: snapshot
            .settings
            .iter()
            .map(|entry| entry.key.clone())
            .collect(),
        forwarding_profiles: snapshot.forwarding_profiles.clone(),
        credential_connection_ids: snapshot.credential_connection_ids.clone(),
    };
    let mut pending = state
        .pending_luna_remote_imports
        .lock()
        .map_err(|_| "Luna Remote 导入状态锁已损坏")?;
    pending.clear();
    pending.insert(preview_id, snapshot);
    Ok(preview)
}

#[tauri::command]
pub fn bookmarks_preview_luna_remote(
    state: State<AppState>,
    path: String,
) -> Result<LunaRemoteImportPreview, String> {
    let path = PathBuf::from(path);
    let snapshot = Database::read_luna_remote_snapshot(&path)?;
    store_luna_remote_preview(&state, snapshot)
}

#[tauri::command]
pub async fn bookmarks_choose_luna_remote_database(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<LunaRemoteImportPreview>, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("选择 Luna Remote 数据库")
        .add_filter("Luna Remote 数据库", &["db"])
        .pick_file(move |path| {
            let _ = sender.send(path);
        });
    let Some(path) = selected_path(receiver.await.ok().flatten()) else {
        return Ok(None);
    };
    let snapshot = Database::read_luna_remote_snapshot(Path::new(&path))?;
    store_luna_remote_preview(&state, snapshot).map(Some)
}

#[tauri::command]
pub fn bookmarks_import_luna_remote(
    state: State<AppState>,
    selection: LunaRemoteImportSelection,
) -> Result<LunaRemoteImportResult, String> {
    let snapshot = state
        .pending_luna_remote_imports
        .lock()
        .map_err(|_| "Luna Remote 导入状态锁已损坏")?
        .get(&selection.preview_id)
        .cloned()
        .ok_or_else(|| "Luna Remote 导入预览已失效，请重新选择数据源".to_string())?;
    let selected_ids = selection.connection_ids.into_iter().collect::<HashSet<_>>();
    let source_ids = snapshot
        .connections
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<HashSet<_>>();
    if selected_ids.len() > snapshot.connections.len()
        || selected_ids
            .iter()
            .any(|id| !source_ids.contains(id.as_str()))
    {
        return Err("导入选择包含未知的 Luna Remote 连接，请重新预览".into());
    }
    let entries = snapshot
        .connections
        .iter()
        .filter(|entry| selected_ids.contains(&entry.id))
        .cloned()
        .collect::<Vec<_>>();
    if let Some(entry) = entries.iter().find(|entry| {
        !entry.jump_bookmark_id.is_empty() && !selected_ids.contains(&entry.jump_bookmark_id)
    }) {
        return Err(format!("连接“{}”依赖的跳板机未被选择", entry.name));
    }
    let allowed_groups = snapshot
        .groups
        .iter()
        .chain(snapshot.connections.iter().map(|entry| &entry.group_name))
        .map(|group| group.trim())
        .filter(|group| !group.is_empty())
        .collect::<HashSet<_>>();
    let mut requested_groups = Vec::new();
    for group in selection.groups {
        let group = group.trim().to_string();
        if !group.is_empty() && !requested_groups.contains(&group) {
            requested_groups.push(group);
        }
    }
    if requested_groups
        .iter()
        .any(|group| !allowed_groups.contains(group.as_str()))
    {
        return Err("导入选择包含未知的 Luna Remote 分组，请重新预览".into());
    }
    let has_selected_forwarding_profiles = selection.import_forwarding_profiles
        && snapshot
            .forwarding_profiles
            .iter()
            .any(|profile| selected_ids.contains(&profile.bookmark_id));
    if entries.is_empty()
        && requested_groups.is_empty()
        && !selection.import_host_keys
        && !selection.import_settings
        && !has_selected_forwarding_profiles
    {
        return Err("请至少选择一种要导入的数据".into());
    }
    if selection.import_credentials && entries.is_empty() {
        return Err("导入凭据前必须选择对应连接".into());
    }
    let result = state.db.import_luna_remote_snapshot(
        &snapshot,
        &entries,
        &requested_groups,
        selection.import_host_keys,
        selection.import_settings,
        has_selected_forwarding_profiles,
        selection.import_credentials,
        &[
            LUNA_REMOTE_CREDENTIAL_SERVICE,
            LUNA_REMOTE_LEGACY_CREDENTIAL_SERVICE,
        ],
    )?;
    state
        .pending_luna_remote_imports
        .lock()
        .map_err(|_| "Luna Remote 导入状态锁已损坏")?
        .remove(&selection.preview_id);
    Ok(result)
}
#[tauri::command]
pub fn state_get_collapsed_bookmark_groups(state: State<AppState>) -> Vec<String> {
    state.db.get_setting("collapsedBookmarkGroups", vec![])
}
#[tauri::command]
pub fn state_set_collapsed_bookmark_groups(
    state: State<AppState>,
    groups: Vec<String>,
) -> Result<(), String> {
    state.db.set_setting("collapsedBookmarkGroups", &groups)
}
#[tauri::command]
pub fn state_get_sidebar_width(state: State<AppState>) -> u32 {
    state
        .db
        .get_setting::<u32>("sidebarWidth", 260)
        .clamp(200, 480)
}
#[tauri::command]
pub fn state_set_sidebar_width(state: State<AppState>, width: u32) -> Result<(), String> {
    state.db.set_setting("sidebarWidth", &width.clamp(200, 480))
}

#[tauri::command]
pub fn settings_get_ui_theme(state: State<AppState>) -> UiTheme {
    state.db.get_setting("uiTheme", UiTheme::System)
}

#[tauri::command]
pub fn settings_get_language(state: State<AppState>) -> String {
    state.db.get_setting("language", "zh-CN".to_string())
}

#[tauri::command]
pub fn settings_apply_language(app: AppHandle, menu: NativeMenuLabels) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    app.set_menu(desktop::menu(&app, &menu).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    #[cfg(not(target_os = "macos"))]
    let _ = (app, menu);
    Ok(())
}

#[tauri::command]
pub fn settings_save_language(
    app: AppHandle,
    state: State<AppState>,
    language: String,
    menu: NativeMenuLabels,
) -> Result<String, String> {
    state.db.set_setting("language", &language)?;
    settings_apply_language(app, menu)?;
    Ok(language)
}

#[tauri::command]
pub fn settings_save_ui_theme(
    window: WebviewWindow,
    state: State<AppState>,
    theme: UiTheme,
) -> Result<UiTheme, String> {
    state.db.set_setting("uiTheme", &theme)?;
    window
        .set_theme(match theme {
            UiTheme::System => None,
            UiTheme::Light => Some(Theme::Light),
            UiTheme::Dark => Some(Theme::Dark),
        })
        .map_err(|e| e.to_string())?;
    Ok(theme)
}
#[tauri::command]
pub fn settings_get_terminal(state: State<AppState>) -> TerminalSettings {
    let mut settings = state
        .db
        .get_setting("terminalSettings", TerminalSettings::default());
    if settings.font_family == "Cascadia Mono, SFMono-Regular, Consolas, monospace" {
        settings.font_family = TerminalSettings::default().font_family;
    }
    settings
}
#[tauri::command]
pub fn settings_save_terminal(
    state: State<AppState>,
    mut settings: TerminalSettings,
) -> Result<TerminalSettings, String> {
    if settings.font_family.trim().is_empty() {
        settings.font_family = TerminalSettings::default().font_family;
    }
    settings.font_size = settings.font_size.clamp(10, 32);
    settings.background_opacity = settings.background_opacity.clamp(0.1, 1.0);
    if !settings.background_image_path.is_empty()
        && !Path::new(&settings.background_image_path).is_file()
    {
        settings.background_image_path.clear();
    }
    state.db.set_setting("terminalSettings", &settings)?;
    Ok(settings)
}

fn system_monospace_fonts() -> &'static Vec<String> {
    static FONTS: OnceLock<Vec<String>> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut database = fontdb::Database::new();
        database.load_system_fonts();
        database
            .faces()
            .filter(|face| face.monospaced)
            .filter_map(|face| face.families.first().map(|(name, _)| name.trim()))
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    })
}

#[tauri::command]
pub async fn settings_list_system_fonts() -> Vec<String> {
    tauri::async_runtime::spawn_blocking(|| system_monospace_fonts().clone())
        .await
        .unwrap_or_default()
}

#[tauri::command]
pub fn ai_settings_get(state: State<AppState>) -> AiSettings {
    ai::get_settings(&state.db)
}

#[tauri::command]
pub fn ai_settings_save(
    state: State<AppState>,
    settings: AiSettingsInput,
) -> Result<AiSettings, String> {
    ai::save_settings(&state.db, settings)
}

#[tauri::command]
pub fn ai_settings_delete_key(state: State<AppState>) -> AiSettings {
    ai::delete_api_key(&state.db)
}

#[tauri::command]
pub async fn ai_settings_test(
    state: State<'_, AppState>,
    settings: AiSettingsInput,
) -> Result<(), String> {
    ai::test_settings(&state.db, &state.ai_diagnostics, settings).await
}

#[tauri::command]
pub async fn ai_command_generate(
    state: State<'_, AppState>,
    request: AiGenerateRequest,
) -> Result<AiCommandSuggestion, String> {
    ai::generate_command(&state.db, &state.ai_diagnostics, request).await
}

#[tauri::command]
pub fn ai_command_analyze(command: String) -> Result<AiRiskAssessment, String> {
    ai::analyze_command(&command)
}

#[tauri::command]
pub fn ai_command_history_list(state: State<AppState>) -> Vec<AiCommandHistoryEntry> {
    state.db.list_ai_command_history()
}

#[tauri::command]
pub fn ai_command_history_clear(state: State<AppState>) -> Result<(), String> {
    state.db.clear_ai_command_history()
}

#[tauri::command]
pub fn ai_diagnostics_get(state: State<AppState>) -> Option<AiRawExchange> {
    state.ai_diagnostics.get()
}

#[tauri::command]
pub fn ai_diagnostics_clear(state: State<AppState>) {
    state.ai_diagnostics.clear();
}
#[tauri::command]
pub async fn settings_choose_terminal_background(app: AppHandle) -> Option<String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("选择终端背景图片")
        .add_filter("图片", &["png", "jpg", "jpeg", "webp", "bmp", "gif"])
        .pick_file(move |path| {
            let _ = sender.send(path);
        });
    selected_path(receiver.await.ok().flatten())
}
#[tauri::command]
pub fn settings_load_terminal_background(path: String) -> Result<String, String> {
    let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > 25 * 1024 * 1024 {
        return Err("背景图片必须是不超过 25 MB 的文件".into());
    }
    image::ImageReader::open(&path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?;
    let mime = match Path::new(&path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        _ => "image/jpeg",
    };
    Ok(format!(
        "data:{mime};base64,{}",
        STANDARD.encode(fs::read(path).map_err(|e| e.to_string())?)
    ))
}

#[tauri::command]
pub fn settings_get_app_icons(state: State<AppState>) -> Result<AppIconSettings, String> {
    let selected = state.db.get_setting("appIcon", AppIconId::Luna);
    let definitions = [
        (AppIconId::Luna, "Luna"),
        (AppIconId::Graphite, "Graphite"),
        (AppIconId::Signal, "Signal"),
        (AppIconId::Light, "Light"),
    ];
    let options = definitions
        .into_iter()
        .map(|(id, name)| AppIconOption {
            data_url: format!(
                "data:image/png;base64,{}",
                STANDARD.encode(app_icon::bytes(&id))
            ),
            id,
            name: name.into(),
        })
        .collect();
    Ok(AppIconSettings { selected, options })
}
#[tauri::command]
pub async fn settings_set_app_icon(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    icon: AppIconId,
) -> Result<AppIconId, String> {
    app_icon::apply(&app, &window, &icon).await?;
    state.db.set_setting("appIcon", &icon)?;
    Ok(icon)
}

fn deployments(state: &AppState) -> Vec<DeploymentProfile> {
    state.db.get_setting("deploymentProfiles", vec![])
}
#[tauri::command]
pub fn deployments_list(state: State<AppState>, bookmark_id: String) -> Vec<DeploymentProfile> {
    deployments(&state)
        .into_iter()
        .filter(|item| item.bookmark_id == bookmark_id)
        .collect()
}
#[tauri::command]
pub fn deployments_save(
    state: State<AppState>,
    mut profile: DeploymentProfile,
) -> Result<DeploymentProfile, String> {
    if state.db.get_bookmark(&profile.bookmark_id)?.is_none() {
        return Err("连接不存在".into());
    }
    if profile.id.is_empty() {
        profile.id = Uuid::new_v4().to_string();
    }
    profile.name = profile.name.trim().into();
    profile.local_directory = profile.local_directory.trim().into();
    profile.remote_directory = profile
        .remote_directory
        .trim()
        .trim_end_matches('/')
        .to_string();
    if profile.remote_directory.is_empty() {
        profile.remote_directory = "/".into();
    }
    if profile.name.is_empty()
        || profile.local_directory.is_empty()
        || !profile.remote_directory.starts_with('/')
    {
        return Err("部署名称、本地目录和远端绝对路径不能为空".into());
    }
    let mut items = deployments(&state);
    items.retain(|item| item.id != profile.id);
    items.insert(0, profile.clone());
    state.db.set_setting("deploymentProfiles", &items)?;
    Ok(profile)
}
#[tauri::command]
pub fn deployments_remove(state: State<AppState>, id: String) -> Result<(), String> {
    let mut items = deployments(&state);
    items.retain(|item| item.id != id);
    state.db.set_setting("deploymentProfiles", &items)
}

async fn deployment_diff(
    state: &AppState,
    profile: &DeploymentProfile,
    session_id: &str,
) -> Result<Vec<DeploymentDiffEntry>, String> {
    if state.sessions.bookmark_id(session_id)? != profile.bookmark_id {
        return Err("部署配置与当前连接不匹配".into());
    }
    let root = PathBuf::from(&profile.local_directory);
    if !root.is_dir() {
        return Err("本地部署目录不存在".into());
    }
    let mut local = HashMap::<String, (String, u64, i64)>::new();
    for entry in walkdir::WalkDir::new(&root).follow_links(false) {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_type().is_symlink() {
            return Err(format!("暂不支持部署符号链接：{}", entry.path().display()));
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let metadata = entry.metadata().map_err(|e| e.to_string())?;
        let relative = entry
            .path()
            .strip_prefix(&root)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let modified = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .map(|value| value.as_millis() as i64)
            .unwrap_or(0);
        local.insert(
            relative,
            (
                entry.path().to_string_lossy().into_owned(),
                metadata.len(),
                modified,
            ),
        );
    }
    let mut remote = state
        .sessions
        .remote_tree(session_id, &profile.remote_directory)
        .await?;
    let mut result = Vec::new();
    for (relative, (path, size, modified)) in local {
        let remote_entry = remote.remove(&relative);
        let remote_path = if profile.remote_directory == "/" {
            format!("/{relative}")
        } else {
            format!("{}/{relative}", profile.remote_directory)
        };
        let same = remote_entry.as_ref().is_some_and(|entry| {
            entry.size == Some(size)
                && entry
                    .modified_at
                    .is_some_and(|value| (value - modified).abs() <= 2000)
        });
        result.push(DeploymentDiffEntry {
            relative_path: relative,
            local_path: Some(path),
            remote_path,
            status: if remote_entry.is_none() {
                DeploymentDiffStatus::New
            } else if same {
                DeploymentDiffStatus::Same
            } else {
                DeploymentDiffStatus::Changed
            },
            size: Some(size),
        });
    }
    for (relative, entry) in remote {
        result.push(DeploymentDiffEntry {
            relative_path: relative,
            local_path: None,
            remote_path: entry.path,
            status: DeploymentDiffStatus::RemoteOnly,
            size: entry.size,
        });
    }
    result.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    Ok(result)
}
#[tauri::command]
pub async fn deployments_preview(
    state: State<'_, AppState>,
    id: String,
    session_id: String,
) -> Result<Vec<DeploymentDiffEntry>, String> {
    let profile = deployments(&state)
        .into_iter()
        .find(|item| item.id == id)
        .ok_or_else(|| "部署配置不存在".to_string())?;
    deployment_diff(&state, &profile, &session_id).await
}
#[tauri::command]
pub async fn deployments_execute(
    state: State<'_, AppState>,
    id: String,
    session_id: String,
) -> Result<Vec<TransferTask>, String> {
    let profile = deployments(&state)
        .into_iter()
        .find(|item| item.id == id)
        .ok_or_else(|| "部署配置不存在".to_string())?;
    let diff = deployment_diff(&state, &profile, &session_id).await?;
    let mut groups = HashMap::<String, Vec<String>>::new();
    for entry in &diff {
        if entry.local_path.is_none()
            || !matches!(
                entry.status,
                DeploymentDiffStatus::New | DeploymentDiffStatus::Changed
            )
        {
            continue;
        }
        let parent = entry
            .remote_path
            .rsplit_once('/')
            .map(|value| if value.0.is_empty() { "/" } else { value.0 })
            .unwrap_or(".")
            .to_string();
        groups
            .entry(parent)
            .or_default()
            .push(entry.local_path.clone().unwrap());
    }
    let mut tasks = Vec::new();
    for (destination_directory, source_paths) in groups {
        tasks.extend(state.transfers.enqueue(TransferRequest {
            session_id: session_id.clone(),
            direction: TransferDirection::Upload,
            source_paths,
            destination_directory,
        })?);
    }
    let extraneous = diff
        .into_iter()
        .filter(|entry| matches!(entry.status, DeploymentDiffStatus::RemoteOnly))
        .map(|entry| entry.remote_path)
        .collect::<Vec<_>>();
    if profile.delete_extraneous && !extraneous.is_empty() {
        let manager = state.transfers.clone();
        let sessions = state.sessions.clone();
        let ids = tasks.iter().map(|task| task.id.clone()).collect::<Vec<_>>();
        let session = session_id.clone();
        tauri::async_runtime::spawn(async move {
            if manager.when_settled(&ids).await {
                let _ = sessions.remove_remote(&session, extraneous).await;
            }
        });
    }
    Ok(tasks)
}

fn profiles(state: &AppState) -> Vec<PortForwardProfile> {
    state.db.get_setting("portForwardProfiles", vec![])
}

fn generated_port_forward_name(profile: &PortForwardProfile) -> String {
    let bind_port = if profile.bind_port == 0 {
        "自动端口".to_string()
    } else {
        profile.bind_port.to_string()
    };
    let bind = format!("{}:{bind_port}", profile.bind_address);
    match &profile.forward_type {
        PortForwardType::Local => format!(
            "本地 {bind} → {}:{}",
            profile.target_host, profile.target_port
        ),
        PortForwardType::Remote => format!(
            "远端 {bind} → {}:{}",
            profile.target_host, profile.target_port
        ),
        PortForwardType::Dynamic => format!("SOCKS5 {bind}"),
    }
}

#[tauri::command]
pub fn tunnels_list_profiles(
    state: State<AppState>,
    bookmark_id: String,
) -> Vec<PortForwardProfile> {
    profiles(&state)
        .into_iter()
        .filter(|item| item.bookmark_id == bookmark_id)
        .collect()
}
#[tauri::command]
pub fn tunnels_save_profile(
    state: State<AppState>,
    mut profile: PortForwardProfile,
) -> Result<PortForwardProfile, String> {
    if profile.id.is_empty() {
        profile.id = Uuid::new_v4().to_string();
    }
    profile.name = profile.name.trim().into();
    profile.bind_address = profile.bind_address.trim().into();
    profile.target_host = profile.target_host.trim().into();
    if profile.bind_address.is_empty() {
        return Err("请输入有效的监听地址".into());
    }
    if profile.forward_type != PortForwardType::Dynamic
        && (profile.target_host.is_empty() || profile.target_port == 0)
    {
        return Err("请输入有效的目标地址和端口".into());
    }
    if profile.name.is_empty() {
        profile.name = generated_port_forward_name(&profile);
    }
    let mut items = profiles(&state);
    items.retain(|item| item.id != profile.id);
    items.insert(0, profile.clone());
    state.db.set_setting("portForwardProfiles", &items)?;
    Ok(profile)
}
#[tauri::command]
pub fn tunnels_remove_profile(state: State<AppState>, id: String) -> Result<(), String> {
    let mut items = profiles(&state);
    items.retain(|item| item.id != id);
    state.db.set_setting("portForwardProfiles", &items)
}
#[tauri::command]
pub fn tunnels_list(state: State<AppState>, session_id: String) -> Vec<TunnelSummary> {
    state.tunnels.list(&session_id)
}
#[tauri::command]
pub async fn tunnels_start(
    state: State<'_, AppState>,
    session_id: String,
    profile_id: String,
) -> Result<TunnelSummary, String> {
    let profile = profiles(&state)
        .into_iter()
        .find(|item| item.id == profile_id)
        .ok_or_else(|| "端口转发配置不存在".to_string())?;
    state.tunnels.start(session_id, profile).await
}
#[tauri::command]
pub async fn tunnels_stop(
    state: State<'_, AppState>,
    session_id: String,
    tunnel_id: String,
) -> Result<(), String> {
    state.tunnels.stop(&session_id, &tunnel_id).await
}

#[tauri::command]
pub fn transfers_list(state: State<AppState>) -> Result<Vec<TransferTask>, String> {
    state.transfers.list()
}
#[tauri::command]
pub fn transfers_enqueue(
    state: State<AppState>,
    request: TransferRequest,
) -> Result<Vec<TransferTask>, String> {
    state.transfers.enqueue(request)
}
#[tauri::command]
pub fn transfers_cancel(state: State<AppState>, id: String) {
    state.transfers.cancel(&id)
}
#[tauri::command]
pub fn transfers_retry(
    state: State<AppState>,
    id: String,
    session_id: String,
) -> Result<(), String> {
    state.transfers.retry(&id, session_id)
}
#[tauri::command]
pub fn transfers_resolve_conflict(
    state: State<AppState>,
    id: String,
    resolution: ConflictResolution,
    apply_to_batch: bool,
) {
    state
        .transfers
        .resolve_conflict(&id, resolution, apply_to_batch)
}
#[tauri::command]
pub fn transfers_clear_completed(state: State<AppState>) -> Result<(), String> {
    state.transfers.clear_completed()
}

#[tauri::command]
pub async fn diagnostics_export(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let default = format!(
        "{}-diagnostics-{}.json",
        product::PRODUCT_KEY,
        Utc::now().format("%Y-%m-%dT%H-%M-%S")
    );
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("导出诊断信息")
        .set_file_name(default)
        .add_filter("JSON", &["json"])
        .save_file(move |path| {
            let _ = sender.send(path);
        });
    let Some(path) = selected_path(receiver.await.ok().flatten()) else {
        return Ok(None);
    };
    let bookmarks = state.db.list_bookmarks()?;
    let transfers = state.db.list_transfers()?;
    let connections=bookmarks.into_iter().map(|item|json!({"id":item.id,"name":item.name,"host":item.host,"port":item.port,"username":item.username,"authType":item.auth_type,"groupName":item.group_name,"favorite":item.favorite,"usesJumpHost":!item.jump_bookmark_id.is_empty(),"keepaliveEnabled":item.keepalive_enabled,"keepaliveIntervalSeconds":item.keepalive_interval_seconds,"keepaliveCountMax":item.keepalive_count_max,"hasSavedCredential":item.has_saved_credential,"lastConnectedAt":item.last_connected_at})).collect::<Vec<_>>();
    let mut by_status = HashMap::<String, usize>::new();
    for task in &transfers {
        *by_status
            .entry(task.status.as_str().to_string())
            .or_default() += 1;
    }
    let active_bytes = transfers
        .iter()
        .filter(|task| {
            matches!(
                task.status,
                TransferStatus::Queued
                    | TransferStatus::Scanning
                    | TransferStatus::Running
                    | TransferStatus::Conflict
            )
        })
        .map(|task| task.bytes_transferred)
        .sum::<u64>();
    let value = json!({"generatedAt":Utc::now().to_rfc3339(),"application":{"name":product::DISPLAY_NAME,"version":env!("CARGO_PKG_VERSION"),"platform":platform(),"arch":std::env::consts::ARCH,"runtime":"Tauri 2"},"connections":connections,"sessions":state.sessions.list(),"transfers":{"total":transfers.len(),"byStatus":by_status,"activeBytes":active_bytes},"portForwardProfileCount":profiles(&state).len()});
    fs::write(
        &path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
        ),
    )
    .map_err(|e| e.to_string())?;
    Ok(Some(path))
}
