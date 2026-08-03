use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::AtomicBool},
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
    ai, app_icon, database::Database, models::*, sessions::SessionManager, ssh_config,
    transfers::TransferManager, tunnels::TunnelManager,
};

pub struct AppState {
    pub db: Arc<Database>,
    pub sessions: Arc<SessionManager>,
    pub transfers: Arc<TransferManager>,
    pub tunnels: Arc<TunnelManager>,
    pub ai_diagnostics: ai::AiDiagnostics,
    pub allowed_imports: Mutex<HashSet<PathBuf>>,
    pub exit_cleanup_started: AtomicBool,
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
pub fn sessions_list(state: State<AppState>) -> Vec<SessionSummary> {
    state.sessions.list()
}
#[tauri::command]
pub fn sessions_connect(
    state: State<AppState>,
    input: ConnectInput,
) -> Result<SessionSummary, String> {
    state.sessions.connect(input)
}
#[tauri::command]
pub async fn sessions_disconnect(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.tunnels.stop_session(&id).await;
    state.sessions.disconnect(&id).await
}
#[tauri::command]
pub async fn sessions_write(
    state: State<'_, AppState>,
    id: String,
    data: String,
) -> Result<(), String> {
    state.sessions.write(&id, data).await
}
#[tauri::command]
pub async fn sessions_resize(
    state: State<'_, AppState>,
    id: String,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    state.sessions.resize(&id, cols, rows).await
}
#[tauri::command]
pub fn sessions_flow(state: State<AppState>, id: String, paused: bool) -> Result<(), String> {
    state.sessions.flow(&id, paused)
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
pub fn state_get_open_bookmarks(state: State<AppState>) -> Vec<String> {
    state.db.get_setting("openBookmarks", vec![])
}
#[tauri::command]
pub fn state_set_open_bookmarks(state: State<AppState>, ids: Vec<String>) -> Result<(), String> {
    state.db.set_setting("openBookmarks", &ids)
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
    state
        .db
        .get_setting("terminalSettings", TerminalSettings::default())
}
#[tauri::command]
pub fn settings_save_terminal(
    state: State<AppState>,
    mut settings: TerminalSettings,
) -> Result<TerminalSettings, String> {
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
    let selected = state.db.get_setting("appIcon", AppIconId::Ssh);
    let definitions = [
        (AppIconId::Ssh, "SSH 终端"),
        (AppIconId::Classic, "蓝色传输"),
        (AppIconId::Neon, "霓虹深色"),
        (AppIconId::Light, "清新传输"),
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
    if profile.name.is_empty() || profile.bind_address.is_empty() {
        return Err("请输入有效的端口转发名称和监听地址".into());
    }
    if profile.forward_type != PortForwardType::Dynamic
        && (profile.target_host.is_empty() || profile.target_port == 0)
    {
        return Err("请输入有效的目标地址和端口".into());
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
        "ssh-client-diagnostics-{}.json",
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
    let value = json!({"generatedAt":Utc::now().to_rfc3339(),"application":{"name":"SSH Client","version":env!("CARGO_PKG_VERSION"),"platform":platform(),"arch":std::env::consts::ARCH,"runtime":"Tauri 2"},"connections":connections,"sessions":state.sessions.list(),"transfers":{"total":transfers.len(),"byStatus":by_status,"activeBytes":active_bytes},"portForwardProfileCount":profiles(&state).len()});
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
