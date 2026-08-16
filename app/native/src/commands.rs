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
use tauri::{AppHandle, Emitter, Manager, State, Theme, WebviewWindow};
use tauri_plugin_dialog::{DialogExt, FilePath};
use tauri_plugin_opener::OpenerExt;
use uuid::Uuid;

use crate::{
    agent_adapters::{self, ManagedAgentLaunch},
    agent_hooks::{AgentHookService, ManagedAgentEvent},
    agent_profiles::{self, AgentLaunchProfile, AgentProfileAvailability},
    ai, app_icon,
    browser_runtime::{
        BrowserKeyEvent, BrowserMouseEvent, BrowserRuntime, BrowserRuntimeCreateRequest,
        BrowserRuntimeManager, ChromeInstallation,
    },
    composite_terminal_backend::CompositeTerminalBackend,
    control_adapter::AuthenticatedControlAdapter,
    control_contract::{
        ControlApproval, ControlCatalog, ControlEventReadResult, ControlRequest, ControlResponse,
    },
    control_service::{self, InProcessControlService, LunaControlService},
    database::Database,
    desktop,
    local_pty_backend::InProcessLocalPtyTerminalBackend,
    luna_mcp::{LunaMcpService, MCP_AUTHORIZATION_ENV},
    models::*,
    product,
    sessions::SessionManager,
    ssh_config,
    ssh_terminal_backend::InProcessSshTerminalBackend,
    terminal_backend::TerminalBackend,
    terminal_runtime_contract::{
        TerminalRuntime, TerminalRuntimeAuthentication, TerminalRuntimeContext,
        TerminalRuntimeCreateRequest, TerminalRuntimeOutputReadResult, TerminalTarget,
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
    pub local_pty_backend: Arc<InProcessLocalPtyTerminalBackend>,
    pub terminal_backend: Arc<CompositeTerminalBackend>,
    pub control: Arc<InProcessControlService>,
    pub control_adapter: Arc<AuthenticatedControlAdapter>,
    pub luna_mcp: Arc<LunaMcpService>,
    pub agent_hooks: Arc<AgentHookService>,
    pub transfers: Arc<TransferManager>,
    pub tunnels: Arc<TunnelManager>,
    pub browser_runtimes: Arc<BrowserRuntimeManager>,
    pub agent_notification_focus: Arc<Mutex<AgentNotificationFocus>>,
    pub ai_diagnostics: ai::AiDiagnostics,
    pub allowed_imports: Mutex<HashSet<PathBuf>>,
    pub pending_archive_imports: Mutex<HashMap<String, BookmarkArchive>>,
    pub pending_luna_remote_imports: Mutex<HashMap<String, LunaRemoteSnapshot>>,
    pub exit_cleanup_started: AtomicBool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentNotificationFocus {
    pub mux_session_id: Option<String>,
    pub pane_id: Option<String>,
    pub terminal_visible: bool,
}

#[tauri::command]
pub fn browser_chrome_discover(state: State<AppState>) -> Option<ChromeInstallation> {
    state.browser_runtimes.discover_chrome()
}

#[tauri::command]
pub async fn browser_runtime_create(
    state: State<'_, AppState>,
    request: BrowserRuntimeCreateRequest,
) -> Result<BrowserRuntime, String> {
    let browser_resource_id = request.browser_resource_id.clone();
    let resource = state
        .db
        .list_browser_resources(None)?
        .into_iter()
        .find(|resource| resource.id == browser_resource_id)
        .ok_or_else(|| "浏览器资源不存在".to_string())?;
    if resource.mux_session_id != request.mux_session_id {
        return Err("浏览器资源与会话不匹配".into());
    }
    let runtime = state.browser_runtimes.create(request).await?;
    if let Err(error) = state
        .luna_mcp
        .refresh_target_resource("browser", &browser_resource_id)
    {
        let _ = state.browser_runtimes.close(&runtime.id).await;
        return Err(error);
    }
    Ok(runtime)
}

#[tauri::command]
pub fn browser_runtimes_list(state: State<AppState>) -> Result<Vec<BrowserRuntime>, String> {
    state.browser_runtimes.list()
}

#[tauri::command]
pub async fn browser_runtime_close(
    state: State<'_, AppState>,
    runtime_id: String,
) -> Result<(), String> {
    let browser_resource_id = state
        .browser_runtimes
        .list()?
        .into_iter()
        .find(|runtime| runtime.id == runtime_id)
        .map(|runtime| runtime.browser_resource_id);
    state.browser_runtimes.close(&runtime_id).await?;
    if let Some(browser_resource_id) = browser_resource_id {
        if let Err(error) = state
            .luna_mcp
            .refresh_target_resource("browser", &browser_resource_id)
        {
            eprintln!("failed to refresh Browser resource grants after close: {error}");
        }
    }
    Ok(())
}

#[tauri::command]
pub fn browser_runtime_navigate(
    state: State<AppState>,
    runtime_id: String,
    url: String,
) -> Result<(), String> {
    state.browser_runtimes.navigate(&runtime_id, &url)
}

#[tauri::command]
pub fn browser_runtime_focus_external(
    state: State<AppState>,
    runtime_id: String,
) -> Result<(), String> {
    state.browser_runtimes.focus_external_window(&runtime_id)
}

#[tauri::command]
pub fn browser_runtime_resize(
    state: State<AppState>,
    runtime_id: String,
    width: u32,
    height: u32,
) -> Result<(), String> {
    state.browser_runtimes.resize(&runtime_id, width, height)
}

#[tauri::command]
pub fn browser_runtime_mouse(
    state: State<AppState>,
    runtime_id: String,
    event: BrowserMouseEvent,
) -> Result<(), String> {
    state.browser_runtimes.mouse(&runtime_id, event)
}

#[tauri::command]
pub fn browser_runtime_key(
    state: State<AppState>,
    runtime_id: String,
    event: BrowserKeyEvent,
) -> Result<(), String> {
    state.browser_runtimes.key(&runtime_id, event)
}

/// Tauri is the trusted local UI adapter. External Agent/CLI adapters must
/// authenticate independently and inject their own caller identity.
#[tauri::command]
pub fn control_catalog(state: State<AppState>) -> ControlCatalog {
    let _adapter = &state.control_adapter;
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

/// Only the trusted desktop UI can resolve an approval. External transports
/// receive approval IDs through the event stream but cannot call this IPC API.
#[tauri::command]
pub fn control_approval_resolve(
    state: State<AppState>,
    approval_id: String,
    approved: bool,
) -> Result<ControlApproval, serde_json::Value> {
    state
        .control
        .resolve_approval(&approval_id, approved)
        .map_err(|error| serde_json::to_value(error).unwrap_or_else(|_| serde_json::json!({"code":"internal","message":"审批处理失败","retryable":false})))
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
            runtime_id: None,
            context: None,
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
            managed_agent: None,
            launch_environment: Default::default(),
            cols: 100,
            rows: 30,
        })
        .await?;
    InProcessSshTerminalBackend::legacy_summary(&runtime)
}

#[tauri::command]
pub async fn terminal_runtime_create(
    state: State<'_, AppState>,
    mut request: TerminalRuntimeCreateRequest,
) -> Result<TerminalRuntime, String> {
    let is_remote = request.target_id.starts_with("ssh-bookmark:");
    let remote_agent_integration_enabled =
        !is_remote || state.db.get_setting("remoteAgentIntegrationEnabled", false);
    if is_remote && request.managed_agent.is_some() && !remote_agent_integration_enabled {
        return Err(
            "远程 Agent 集成当前已关闭。请先在“设置 > SSH > 远程 Agent 集成”中了解风险并启用；普通 SSH 终端不受影响。"
                .into(),
        );
    }
    let mut issued_hook_token = None;
    let mut issued_mcp_token = None;
    let mut remote_agent_launch = None;
    let mut remote_manual_launch = None;
    let mut integration_warning = None;
    if let Some(context) = request.context.clone() {
        validate_runtime_context(&request, &context)?;
        request
            .launch_environment
            .insert("LUNA_MUX_SESSION_ID".into(), context.mux_session_id.clone());
        request
            .launch_environment
            .insert("LUNA_MUX_PANE_ID".into(), context.pane_id.clone());
        request
            .launch_environment
            .insert("LUNA_MUX_RUNTIME_ID".into(), context.runtime_id.clone());
        let browser_cdp_port = if !is_remote || remote_agent_integration_enabled {
            request.launch_environment.insert(
                "LUNA_MUX_BROWSER_REGISTRY_PATH".into(),
                state.browser_runtimes.registry_path(),
            );
            let port = state
                .browser_runtimes
                .session_cdp_port(&context.mux_session_id)?;
            request
                .launch_environment
                .insert("LUNA_MUX_BROWSER_CDP_PORT".into(), port.to_string());
            Some(port)
        } else {
            None
        };
        if request.managed_agent.is_none() && remote_agent_integration_enabled {
            let endpoint = state.agent_hooks.endpoint()?;
            let token = state.agent_hooks.issue_bootstrap_token(context.clone())?;
            let mcp_endpoint = state.luna_mcp.endpoint()?;
            let mcp_token = state
                .luna_mcp
                .issue_runtime_context_token(request.context.as_ref().expect("terminal context"))?;
            if request.target_id.starts_with("ssh-bookmark:") {
                remote_manual_launch = Some((
                    endpoint,
                    token.clone(),
                    mcp_endpoint,
                    mcp_token.clone(),
                    context,
                    browser_cdp_port.expect("remote integration Browser port"),
                ));
            } else {
                #[cfg(windows)]
                let wsl_bootstrap = if request.target_id.starts_with("local:wsl:") {
                    match crate::codex_shim::install_wsl_manual_bootstrap(
                        &context,
                        &request.target_id,
                        &mcp_endpoint,
                    ) {
                        Ok(command) => Some(command),
                        Err(error) => {
                            state.agent_hooks.revoke_token(&token);
                            state.luna_mcp.revoke_token(&mcp_token);
                            return Err(error);
                        }
                    }
                } else {
                    None
                };
                #[cfg(not(windows))]
                let wsl_bootstrap: Option<String> = None;
                request
                    .launch_environment
                    .insert("LUNA_MUX_HOOK_ENDPOINT".into(), endpoint);
                request
                    .launch_environment
                    .insert("LUNA_MUX_HOOK_AUTHORIZATION".into(), token.clone());
                request
                    .launch_environment
                    .insert("LUNA_MUX_MCP_ENDPOINT".into(), mcp_endpoint);
                request
                    .launch_environment
                    .insert(MCP_AUTHORIZATION_ENV.into(), mcp_token.clone());
                if let Some(bootstrap) = wsl_bootstrap {
                    request.command = Some(match request.command.take() {
                        Some(command) if !command.trim().is_empty() => {
                            format!("{bootstrap}; {command}")
                        }
                        _ => bootstrap,
                    });
                }
            }
            issued_hook_token = Some(token);
            issued_mcp_token = Some(mcp_token);
        }
    }
    if let Some(agent) = request.managed_agent.clone() {
        validate_managed_agent_context(&request, &agent)?;
        let agent_process_id = agent.agent_id.clone();
        let profile = agent_profiles::resolve(&agent.launch_profile_id)?;
        let endpoint = state.agent_hooks.endpoint()?;
        let mcp_endpoint = state.luna_mcp.endpoint()?;
        let token = state.agent_hooks.issue_token(agent)?;
        issued_hook_token = Some(token.clone());
        let mcp_token = match state.luna_mcp.issue_runtime_token(
            request
                .managed_agent
                .as_ref()
                .expect("managed agent context"),
        ) {
            Ok(token) => token,
            Err(error) => {
                state.agent_hooks.revoke_token(&token);
                return Err(error);
            }
        };
        issued_mcp_token = Some(mcp_token.clone());
        request
            .launch_environment
            .insert("LUNA_MUX_AGENT_ADAPTER".into(), profile.adapter.clone());
        request
            .launch_environment
            .insert("LUNA_MUX_AGENT_PROCESS_ID".into(), agent_process_id);
        if request.target_id.starts_with("ssh-bookmark:") {
            let context = request
                .managed_agent
                .clone()
                .expect("managed agent context");
            request.command = None;
            remote_agent_launch =
                Some((endpoint, token, mcp_endpoint, mcp_token, profile, context));
        } else {
            request
                .launch_environment
                .insert("LUNA_MUX_HOOK_ENDPOINT".into(), endpoint.clone());
            request
                .launch_environment
                .insert("LUNA_MUX_MCP_ENDPOINT".into(), mcp_endpoint.clone());
            // Native local shells load a runtime-scoped codex/claude shim below.
            // Keep the PTY's initial input short: injecting the full configuration
            // command through a canonical TTY can exceed MAX_CANON, truncate a
            // quoted argument, and leave zsh at its `quote>` continuation prompt.
            // WSL does not load those native shims, so it still needs the inline
            // managed command.
            let launch_command = if uses_runtime_agent_shim(&request.target_id) {
                profile.command.trim().to_string()
            } else {
                match agent_adapters::managed_command(&ManagedAgentLaunch {
                    profile: &profile,
                    target_id: &request.target_id,
                    hook_endpoint: &endpoint,
                    mcp_endpoint: &mcp_endpoint,
                    context: request
                        .managed_agent
                        .as_ref()
                        .expect("managed agent context"),
                    inject_inline_hooks: true,
                    hook_command: None,
                    browser_command: None,
                    browser_credentials_file: None,
                    existing_developer_instructions: None,
                }) {
                    Ok(command) => command,
                    Err(error) => {
                        if let Some(token) = issued_hook_token.take() {
                            state.agent_hooks.revoke_token(&token);
                        }
                        if let Some(token) = issued_mcp_token.take() {
                            state.luna_mcp.revoke_token(&token);
                        }
                        return Err(error);
                    }
                }
            };
            request
                .launch_environment
                .insert("LUNA_MUX_HOOK_ENDPOINT".into(), endpoint);
            request
                .launch_environment
                .insert("LUNA_MUX_HOOK_AUTHORIZATION".into(), token);
            request
                .launch_environment
                .insert(MCP_AUTHORIZATION_ENV.into(), mcp_token);
            request.command = Some(launch_command);
        }
    }
    match state.terminal_backend.create(request).await {
        Ok(runtime) => {
            if let Some((local_endpoint, token, local_mcp_endpoint, mcp_token, profile, context)) =
                remote_agent_launch
            {
                let setup = async {
                    wait_for_runtime(&*state.terminal_backend, &runtime.runtime_id).await?;
                    let requires_hook_forwarder =
                        agent_adapters::requires_remote_hook_forwarder(&profile.adapter)?;
                    state
                        .sessions
                        .verify_remote_agent_requirements(
                            &runtime.runtime_id,
                            &profile.command,
                            true,
                        )
                        .await?;
                    let local_port = hook_endpoint_port(&local_endpoint)?;
                    let local_mcp_port = mcp_endpoint_port(&local_mcp_endpoint)?;
                    let remote_port = state
                        .sessions
                        .start_loopback_reverse_forward(&runtime.runtime_id, local_port)
                        .await?;
                    let remote_mcp_port = state
                        .sessions
                        .start_loopback_reverse_forward(&runtime.runtime_id, local_mcp_port)
                        .await?;
                    let remote_endpoint = format!("http://127.0.0.1:{remote_port}/v1/hooks");
                    let remote_mcp_endpoint = format!("http://127.0.0.1:{remote_mcp_port}/mcp");
                    let browser_bridge_token = format!("lmxbm_{}", Uuid::new_v4().simple());
                    let browser_cdp_port = state
                        .browser_runtimes
                        .session_cdp_port(&context.mux_session_id)?;
                    let remote_browser_port = state
                        .sessions
                        .start_browser_mcp_reverse_forward(
                            &runtime.runtime_id,
                            context.mux_session_id.clone(),
                            browser_cdp_port,
                            browser_bridge_token.clone(),
                        )
                        .await?;
                    let browser_proxy = state
                        .sessions
                        .install_browser_mcp_proxy(&runtime.runtime_id)
                        .await?;
                    let browser_credentials = state
                        .sessions
                        .write_browser_bridge_credentials(
                            &runtime.runtime_id,
                            &runtime.runtime_id,
                            remote_browser_port,
                            &browser_bridge_token,
                        )
                        .await?;
                    let environment_file = state
                        .sessions
                        .write_agent_environment_file(
                            &runtime.runtime_id,
                            &runtime.runtime_id,
                            &remote_endpoint,
                            &token,
                            &mcp_token,
                            Some(&browser_credentials),
                        )
                        .await?;
                    let remote_hook_command = if requires_hook_forwarder {
                        let forwarder = state
                            .sessions
                            .install_agent_hook_forwarder(&runtime.runtime_id)
                            .await?;
                        Some(format!("python3 {}", posix_shell_quote(&forwarder)))
                    } else {
                        None
                    };
                    let existing_developer_instructions = if profile.adapter == "codex" {
                        state
                            .sessions
                            .remote_codex_developer_instructions(&runtime.runtime_id)
                            .await
                    } else {
                        None
                    };
                    let launch_command = agent_adapters::managed_command(&ManagedAgentLaunch {
                        profile: &profile,
                        target_id: &runtime.target_id,
                        hook_endpoint: &remote_endpoint,
                        mcp_endpoint: &remote_mcp_endpoint,
                        context: &context,
                        inject_inline_hooks: true,
                        hook_command: remote_hook_command.as_deref(),
                        browser_command: Some(&browser_proxy),
                        browser_credentials_file: Some(&browser_credentials),
                        existing_developer_instructions: existing_developer_instructions.as_deref(),
                    })?;
                    let command = remote_managed_agent_command(
                        &context,
                        &profile.adapter,
                        &environment_file,
                        &launch_command,
                    );
                    let write_result = state
                        .terminal_backend
                        .write(&runtime.runtime_id, &format!("{command}\r"))
                        .await;
                    if write_result.is_err() {
                        state
                            .sessions
                            .remove_remote_file(&runtime.runtime_id, &environment_file)
                            .await;
                    }
                    write_result
                }
                .await;
                if let Err(error) = setup {
                    state.agent_hooks.revoke_token(&token);
                    state.luna_mcp.revoke_token(&mcp_token);
                    let _ = state.terminal_backend.close(&runtime.runtime_id).await;
                    return Err(format!("远端 Agent 集成初始化失败：{error}"));
                }
            }
            if let Some((
                local_endpoint,
                token,
                local_mcp_endpoint,
                mcp_token,
                context,
                browser_cdp_port,
            )) = remote_manual_launch
            {
                let setup = setup_remote_manual_agent_shims(
                    &state,
                    &runtime,
                    &context,
                    &local_endpoint,
                    &token,
                    &local_mcp_endpoint,
                    &mcp_token,
                    browser_cdp_port,
                )
                .await;
                if let Err(error) = setup {
                    integration_warning = Some(format!("远端 Agent 集成未启用：{error}"));
                    eprintln!(
                        "Unable to install remote Agent shims for Runtime {}: {error}",
                        runtime.runtime_id
                    );
                    if let Err(cleanup_error) = state
                        .sessions
                        .cleanup_remote_agent_runtime_for_session(&runtime.runtime_id)
                        .await
                    {
                        eprintln!(
                            "Unable to roll back remote Agent Runtime files for {}: {cleanup_error}",
                            runtime.runtime_id
                        );
                    }
                    state.agent_hooks.revoke_token(&token);
                    state.luna_mcp.revoke_token(&mcp_token);
                }
            }
            let mut current = state
                .terminal_backend
                .list()?
                .into_iter()
                .find(|current| current.runtime_id == runtime.runtime_id)
                .unwrap_or(runtime);
            if let Some(warning) = integration_warning {
                current.error = Some(warning);
            }
            Ok(current)
        }
        Err(error) => {
            if let Some(token) = issued_hook_token {
                state.agent_hooks.revoke_token(&token);
            }
            if let Some(token) = issued_mcp_token {
                state.luna_mcp.revoke_token(&token);
            }
            Err(error)
        }
    }
}

async fn wait_for_runtime(backend: &dyn TerminalBackend, runtime_id: &str) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let runtime = backend
            .list()?
            .into_iter()
            .find(|runtime| runtime.runtime_id == runtime_id)
            .ok_or_else(|| "SSH Runtime 在启动期间消失".to_string())?;
        match runtime.status {
            crate::terminal_runtime_contract::TerminalRuntimeStatus::Running => return Ok(()),
            crate::terminal_runtime_contract::TerminalRuntimeStatus::Error
            | crate::terminal_runtime_contract::TerminalRuntimeStatus::Exited => {
                return Err(runtime
                    .error
                    .unwrap_or_else(|| "SSH Runtime 启动失败".into()));
            }
            _ => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("等待 SSH Runtime 就绪超时".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn setup_remote_manual_agent_shims(
    state: &AppState,
    runtime: &TerminalRuntime,
    context: &TerminalRuntimeContext,
    local_hook_endpoint: &str,
    hook_token: &str,
    local_mcp_endpoint: &str,
    mcp_token: &str,
    browser_cdp_port: u16,
) -> Result<(), String> {
    wait_for_runtime(&*state.terminal_backend, &runtime.runtime_id).await?;
    if state
        .sessions
        .remote_command_path(&runtime.runtime_id, "python3")
        .await
        .is_none()
    {
        return Err("远端缺少 python3，无法安装 Agent Hook 与 Browser MCP 代理".into());
    }
    let (codex, claude) = tokio::join!(
        state
            .sessions
            .remote_command_path(&runtime.runtime_id, "codex"),
        state
            .sessions
            .remote_command_path(&runtime.runtime_id, "claude")
    );
    if codex.is_none() && claude.is_none() {
        return Err("远端没有可用的 codex 或 claude 命令，已跳过 Agent 注入".into());
    }
    let has_codex = codex.is_some();
    let has_claude = claude.is_some();
    let existing_developer_instructions = if codex.is_some() {
        state
            .sessions
            .remote_codex_developer_instructions(&runtime.runtime_id)
            .await
    } else {
        None
    };
    let hook_forwarder = state
        .sessions
        .install_agent_hook_forwarder(&runtime.runtime_id)
        .await?;
    let browser_proxy = state
        .sessions
        .install_browser_mcp_proxy(&runtime.runtime_id)
        .await?;

    let hook_port = state
        .sessions
        .start_loopback_reverse_forward(
            &runtime.runtime_id,
            hook_endpoint_port(local_hook_endpoint)?,
        )
        .await?;
    let mcp_port = match state
        .sessions
        .start_loopback_reverse_forward(&runtime.runtime_id, mcp_endpoint_port(local_mcp_endpoint)?)
        .await
    {
        Ok(port) => port,
        Err(error) => {
            let _ = state
                .sessions
                .cancel_remote_forward(&runtime.runtime_id, "127.0.0.1".into(), hook_port)
                .await;
            return Err(error);
        }
    };
    let browser_token = format!("lmxbm_{}", Uuid::new_v4().simple());
    let browser_port = match state
        .sessions
        .start_browser_mcp_reverse_forward(
            &runtime.runtime_id,
            context.mux_session_id.clone(),
            browser_cdp_port,
            browser_token.clone(),
        )
        .await
    {
        Ok(port) => port,
        Err(error) => {
            for port in [hook_port, mcp_port] {
                let _ = state
                    .sessions
                    .cancel_remote_forward(&runtime.runtime_id, "127.0.0.1".into(), port)
                    .await;
            }
            return Err(error);
        }
    };
    let mut created_browser_credentials = None;
    let setup = async {
        let browser_credentials = state
            .sessions
            .write_browser_bridge_credentials(
                &runtime.runtime_id,
                &runtime.runtime_id,
                browser_port,
                &browser_token,
            )
            .await?;
        created_browser_credentials = Some(browser_credentials.clone());
        let remote_hook_endpoint = format!("http://127.0.0.1:{hook_port}/v1/hooks");
        let remote_mcp_endpoint = format!("http://127.0.0.1:{mcp_port}/mcp");
        let hook_command = format!("python3 {}", posix_shell_quote(&hook_forwarder));
        let managed_context = crate::terminal_runtime_contract::TerminalManagedAgentContext {
            mux_session_id: context.mux_session_id.clone(),
            pane_id: context.pane_id.clone(),
            runtime_id: context.runtime_id.clone(),
            agent_id: format!("runtime:{}", context.runtime_id),
            launch_profile_id: "manual.remote".into(),
        };
        let mut shim_bin = None;
        for (adapter, real_command, profile_id, label) in [
            ("codex", codex, "codex.default", "Codex"),
            ("claude-code", claude, "claude-code.default", "Claude Code"),
        ] {
            let Some(real_command) = real_command else {
                continue;
            };
            let profile = AgentLaunchProfile {
                id: profile_id.into(),
                label: label.into(),
                adapter: adapter.into(),
                command: posix_shell_quote(&real_command),
                built_in: true,
            };
            let launch = agent_adapters::managed_command(&ManagedAgentLaunch {
                profile: &profile,
                target_id: &runtime.target_id,
                hook_endpoint: &remote_hook_endpoint,
                mcp_endpoint: &remote_mcp_endpoint,
                context: &managed_context,
                inject_inline_hooks: true,
                hook_command: Some(&hook_command),
                browser_command: Some(&browser_proxy),
                browser_credentials_file: Some(&browser_credentials),
                existing_developer_instructions: existing_developer_instructions.as_deref(),
            })?;
            let script = remote_manual_agent_script(adapter, &hook_command, &launch);
            let name = if adapter == "codex" {
                "codex"
            } else {
                "claude"
            };
            let (bin, _) = state
                .sessions
                .install_remote_runtime_shim(
                    &runtime.runtime_id,
                    &runtime.runtime_id,
                    name,
                    &script,
                )
                .await?;
            shim_bin = Some(bin);
        }
        let shim_bin = shim_bin.ok_or_else(|| "没有生成远端 Agent shim".to_string())?;
        let environment_file = state
            .sessions
            .write_agent_environment_file(
                &runtime.runtime_id,
                &runtime.runtime_id,
                &remote_hook_endpoint,
                hook_token,
                mcp_token,
                Some(&browser_credentials),
            )
            .await?;
        let bootstrap = remote_manual_shell_bootstrap(
            context,
            &environment_file,
            &shim_bin,
            has_codex,
            has_claude,
        );
        let write_result = state
            .terminal_backend
            .write(&runtime.runtime_id, &format!("{bootstrap}\r"))
            .await;
        if write_result.is_err() {
            state
                .sessions
                .remove_remote_file(&runtime.runtime_id, &environment_file)
                .await;
        }
        write_result
    }
    .await;
    if setup.is_err() {
        if let Some(path) = created_browser_credentials.as_deref() {
            state
                .sessions
                .remove_remote_file(&runtime.runtime_id, path)
                .await;
        }
        for port in [hook_port, mcp_port, browser_port] {
            let _ = state
                .sessions
                .cancel_remote_forward(&runtime.runtime_id, "127.0.0.1".into(), port)
                .await;
        }
    }
    setup
}

fn remote_manual_agent_script(adapter_id: &str, hook_command: &str, launch: &str) -> String {
    let payload_adapter = if adapter_id == "claude-code" {
        "claude-code"
    } else {
        "codex"
    };
    format!(
        "#!/bin/sh\n\
LUNA_MUX_AGENT_PROCESS_ID=\"$$-$(date +%s)\"\n\
LUNA_MUX_AGENT_ADAPTER={}\n\
export LUNA_MUX_AGENT_PROCESS_ID LUNA_MUX_AGENT_ADAPTER\n\
printf '%s' '{}' | {} >/dev/null 2>&1 || true\n\
{} \"$@\"\n\
luna_mux_agent_exit_code=$?\n\
printf '%s' '{}' | {} >/dev/null 2>&1 || true\n\
exit \"$luna_mux_agent_exit_code\"\n",
        posix_shell_quote(payload_adapter),
        format!(
            "{{\"hook_event_name\":\"AgentProcessStart\",\"agent_adapter\":\"{payload_adapter}\"}}"
        ),
        hook_command,
        launch,
        format!(
            "{{\"hook_event_name\":\"AgentProcessExit\",\"agent_adapter\":\"{payload_adapter}\"}}"
        ),
        hook_command,
    )
}

fn remote_manual_shell_bootstrap(
    context: &TerminalRuntimeContext,
    environment_file: &str,
    shim_bin: &str,
    has_codex: bool,
    has_claude: bool,
) -> String {
    let mut command = format!(
        "set -a; . {}; rm -f -- {}; set +a; LUNA_MUX_SESSION_ID={}; LUNA_MUX_PANE_ID={}; LUNA_MUX_RUNTIME_ID={}; export LUNA_MUX_SESSION_ID LUNA_MUX_PANE_ID LUNA_MUX_RUNTIME_ID; PATH={}:\"$PATH\"; export PATH; hash -r 2>/dev/null || true",
        posix_shell_quote(environment_file),
        posix_shell_quote(environment_file),
        posix_shell_quote(&context.mux_session_id),
        posix_shell_quote(&context.pane_id),
        posix_shell_quote(&context.runtime_id),
        posix_shell_quote(shim_bin),
    );
    for (name, enabled) in [("codex", has_codex), ("claude", has_claude)] {
        if enabled {
            command.push_str(&format!(
                "; unalias {name} 2>/dev/null || true; {name}() {{ {} \"$@\"; }}",
                posix_shell_quote(&format!("{shim_bin}/{name}"))
            ));
        }
    }
    command
}

fn hook_endpoint_port(endpoint: &str) -> Result<u16, String> {
    loopback_endpoint_port(endpoint, "/v1/hooks", "Hook 接收器")
}

fn mcp_endpoint_port(endpoint: &str) -> Result<u16, String> {
    loopback_endpoint_port(endpoint, "/mcp", "MCP 服务")
}

fn loopback_endpoint_port(endpoint: &str, path: &str, label: &str) -> Result<u16, String> {
    let url = url::Url::parse(endpoint).map_err(|error| error.to_string())?;
    if url.scheme() != "http" || url.host_str() != Some("127.0.0.1") || url.path() != path {
        return Err(format!("{label}必须是 127.0.0.1 上的 HTTP {path}"));
    }
    url.port().ok_or_else(|| format!("{label}缺少本地端口"))
}

fn remote_managed_agent_command(
    agent: &crate::terminal_runtime_contract::TerminalManagedAgentContext,
    adapter_id: &str,
    environment_file: &str,
    command: &str,
) -> String {
    let identity = [
        ("LUNA_MUX_SESSION_ID", agent.mux_session_id.as_str()),
        ("LUNA_MUX_PANE_ID", agent.pane_id.as_str()),
        ("LUNA_MUX_RUNTIME_ID", agent.runtime_id.as_str()),
        ("LUNA_MUX_AGENT_ID", agent.agent_id.as_str()),
        ("LUNA_MUX_AGENT_ADAPTER", adapter_id),
        ("LUNA_MUX_AGENT_PROCESS_ID", agent.agent_id.as_str()),
        (
            "LUNA_MUX_LAUNCH_PROFILE_ID",
            agent.launch_profile_id.as_str(),
        ),
    ]
    .into_iter()
    .map(|(key, value)| format!("{key}={}", posix_shell_quote(value)))
    .collect::<Vec<_>>()
    .join(" ");
    let environment_file = posix_shell_quote(environment_file);
    format!(
        "(set -a; . {environment_file}; rm -f -- {environment_file}; set +a; {identity} {command})"
    )
}

fn validate_managed_agent_context(
    request: &TerminalRuntimeCreateRequest,
    agent: &crate::terminal_runtime_contract::TerminalManagedAgentContext,
) -> Result<(), String> {
    let runtime_id = request
        .runtime_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "受管 Agent 必须预分配 runtimeId".to_string())?;
    if runtime_id != agent.runtime_id {
        return Err("受管 Agent runtimeId 与启动请求不一致".into());
    }
    if [
        agent.mux_session_id.as_str(),
        agent.pane_id.as_str(),
        agent.agent_id.as_str(),
        agent.launch_profile_id.as_str(),
    ]
    .into_iter()
    .any(|value| value.trim().is_empty())
    {
        return Err("受管 Agent 的 Session、Pane、Agent 和 Launch Profile 标识不能为空".into());
    }
    Ok(())
}

fn uses_runtime_agent_shim(target_id: &str) -> bool {
    target_id.starts_with("local:") && !target_id.starts_with("local:wsl:")
}

fn validate_runtime_context(
    request: &TerminalRuntimeCreateRequest,
    context: &TerminalRuntimeContext,
) -> Result<(), String> {
    let runtime_id = request
        .runtime_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "终端上下文必须预分配 runtimeId".to_string())?;
    if runtime_id != context.runtime_id {
        return Err("终端上下文 runtimeId 与启动请求不一致".into());
    }
    if [context.mux_session_id.as_str(), context.pane_id.as_str()]
        .into_iter()
        .any(|value| value.trim().is_empty())
    {
        return Err("终端上下文的 Session 和 Pane 标识不能为空".into());
    }
    Ok(())
}

#[cfg(test)]
fn codex_command_with_hook(command: &str, target_id: &str) -> Result<String, String> {
    codex_managed_command(command, target_id, true, "http://127.0.0.1:43128/mcp")
}

#[cfg(test)]
fn codex_managed_command(
    command: &str,
    target_id: &str,
    inject_inline_hooks: bool,
    mcp_endpoint: &str,
) -> Result<String, String> {
    codex_managed_command_with_hook(command, target_id, inject_inline_hooks, None, mcp_endpoint)
}

#[cfg(test)]
fn codex_managed_command_with_hook(
    command: &str,
    target_id: &str,
    inject_inline_hooks: bool,
    hook_command: Option<&str>,
    mcp_endpoint: &str,
) -> Result<String, String> {
    crate::codex_shim::managed_command(
        command,
        target_id,
        inject_inline_hooks,
        hook_command,
        mcp_endpoint,
        None,
        None,
        "session-1",
        None,
    )
}

#[cfg(test)]
fn hook_executable_for_target(executable: &Path, target_id: &str) -> Result<String, String> {
    crate::codex_shim::hook_executable_for_target(executable, target_id)
}

fn posix_shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod managed_agent_launch_tests {
    use super::{
        codex_command_with_hook, codex_managed_command_with_hook, hook_endpoint_port,
        hook_executable_for_target, mcp_endpoint_port, remote_managed_agent_command,
        remote_manual_agent_script, remote_manual_shell_bootstrap, uses_runtime_agent_shim,
        validate_managed_agent_context,
    };
    use crate::terminal_runtime_contract::{
        TerminalManagedAgentContext, TerminalRuntimeContext, TerminalRuntimeCreateRequest,
    };
    use std::path::Path;

    #[test]
    fn native_local_agents_use_short_runtime_shim_commands() {
        assert!(uses_runtime_agent_shim("local:macos-shell"));
        assert!(uses_runtime_agent_shim("local:powershell"));
        assert!(!uses_runtime_agent_shim("local:wsl:Ubuntu"));
        assert!(!uses_runtime_agent_shim("ssh-bookmark:server-1"));
    }

    #[test]
    fn codex_hook_configuration_covers_lifecycle_without_embedding_credentials() {
        let command = codex_command_with_hook("codex", "local:powershell").unwrap();
        for event in [
            "SessionStart",
            "PermissionRequest",
            "PostToolUse",
            "Stop",
            "SessionEnd",
        ] {
            assert!(command.contains(&format!("hooks.{event}=")));
        }
        assert!(command.contains(" hook"));
        assert!(!command.contains("LUNA_MUX_HOOK_AUTHORIZATION"));
        assert!(!command.contains("lmxh_"));
        assert!(!command.contains("lmx_"));
        assert!(command.contains("features.network_proxy=true"));
        assert!(command.contains("127.0.0.1"));
        assert!(!command.contains("mcp_servers.chrome_devtools.command"));
        assert!(!command.contains("mcp_servers.chrome_devtools.args"));
        assert!(!command.contains("features.plugins=false"));
        assert!(!command.contains("mcp_servers.node_repl.enabled=false"));
    }

    #[test]
    fn wsl_hook_path_uses_the_mounted_windows_drive() {
        if !cfg!(windows) {
            return;
        }
        let path = hook_executable_for_target(
            Path::new(r"D:\Program Files\Luna Mux\luna-mux.exe"),
            "local:wsl:Ubuntu",
        )
        .unwrap();
        assert_eq!(path, "/mnt/d/Program Files/Luna Mux/luna-mux.exe");
        let command = codex_managed_command_with_hook(
            "codex",
            "local:wsl:Ubuntu",
            true,
            None,
            "http://127.0.0.1:43128/mcp",
        )
        .unwrap();
        assert!(command.contains("mcp_servers.luna_mux.command="));
        assert!(command.contains("mcp_servers.luna_mux.args=['mcp','luna']"));
        assert!(!command.contains("mcp_servers.luna_mux.url="));
    }

    #[test]
    fn remote_hook_uses_posix_command_and_loopback_reverse_endpoint() {
        let command = codex_managed_command_with_hook(
            "codex",
            "ssh-bookmark:server-1",
            true,
            Some("python3 '/home/user/.luna-mux/bin/hook_forwarder.py'"),
            "http://127.0.0.1:43128/mcp",
        )
        .unwrap();
        assert!(command.contains("hook_forwarder.py"));
        assert!(command.contains("python3"));
        assert!(command.contains("hooks.SessionStart="));
        assert!(!command.contains("commandWindows"));
        assert_eq!(
            hook_endpoint_port("http://127.0.0.1:43127/v1/hooks").unwrap(),
            43127
        );
        assert!(hook_endpoint_port("http://localhost:43127/v1/hooks").is_err());
        assert_eq!(
            mcp_endpoint_port("http://127.0.0.1:43128/mcp").unwrap(),
            43128
        );
        assert!(!command.contains("lmx_"));
    }

    #[test]
    fn remote_agent_environment_file_is_consumed_without_echoing_secrets() {
        let context = TerminalManagedAgentContext {
            mux_session_id: "session-1".into(),
            pane_id: "pane-1".into(),
            runtime_id: "runtime-1".into(),
            agent_id: "agent-1".into(),
            launch_profile_id: "codex.default".into(),
        };
        let command = remote_managed_agent_command(
            &context,
            "codex",
            "/home/user/.luna-mux/runtime/agent-secret.env",
            "codex",
        );
        assert!(command.contains(". '/home/user/.luna-mux/runtime/agent-secret.env'"));
        assert!(command.contains("rm -f -- '/home/user/.luna-mux/runtime/agent-secret.env'"));
        assert!(!command.contains("LUNA_MUX_HOOK_AUTHORIZATION"));
        assert!(!command.contains("lmxh_"));
        assert!(!command.contains("lmx_control-secret"));
        assert!(command.contains("LUNA_MUX_AGENT_ADAPTER='codex'"));
        assert!(command.contains("LUNA_MUX_AGENT_PROCESS_ID='agent-1'"));
        assert!(command.ends_with(" codex)"));
    }

    #[test]
    fn remote_manual_shims_preserve_arguments_and_emit_process_lifecycle() {
        let script = remote_manual_agent_script(
            "claude-code",
            "python3 '/home/user/.luna-mux/bin/hook_forwarder.py'",
            "'/usr/local/bin/claude' --settings '{}'",
        );
        assert!(script.starts_with("#!/bin/sh"));
        assert!(script.contains("AgentProcessStart"));
        assert!(script.contains("AgentProcessExit"));
        assert!(script.contains("\"agent_adapter\":\"claude-code\""));
        assert!(script.contains("'/usr/local/bin/claude' --settings '{}' \"$@\""));
    }

    #[test]
    fn remote_shell_bootstrap_scopes_both_agent_commands_to_the_runtime_path() {
        let context = TerminalRuntimeContext {
            mux_session_id: "session-1".into(),
            pane_id: "pane-1".into(),
            runtime_id: "runtime-1".into(),
        };
        let command = remote_manual_shell_bootstrap(
            &context,
            "/home/user/.luna-mux/runtime/agent.env",
            "/home/user/.luna-mux/runtime/runtime-1/bin",
            true,
            true,
        );
        assert!(command.contains(". '/home/user/.luna-mux/runtime/agent.env'"));
        assert!(command.contains("rm -f -- '/home/user/.luna-mux/runtime/agent.env'"));
        assert!(command.contains("PATH='/home/user/.luna-mux/runtime/runtime-1/bin':\"$PATH\""));
        assert!(command.contains("LUNA_MUX_SESSION_ID='session-1'"));
        assert!(
            command
                .contains("codex() { '/home/user/.luna-mux/runtime/runtime-1/bin/codex' \"$@\"; }")
        );
        assert!(
            command.contains(
                "claude() { '/home/user/.luna-mux/runtime/runtime-1/bin/claude' \"$@\"; }"
            )
        );
        assert!(!command.contains("lmxbm_"));
        assert!(!command.contains("LUNA_MUX_HOOK_AUTHORIZATION="));
    }

    #[test]
    #[cfg(windows)]
    fn installed_codex_accepts_generated_hook_configuration() {
        if !std::process::Command::new("where.exe")
            .arg("codex.cmd")
            .status()
            .is_ok_and(|status| status.success())
        {
            return;
        }
        let command = format!(
            "{} --version",
            codex_command_with_hook("codex.cmd", "local:powershell").unwrap()
        );
        let output = std::process::Command::new("powershell.exe")
            .args(["-NoLogo", "-NoProfile", "-Command", &command])
            .output()
            .expect("run installed Codex CLI");
        assert!(
            output.status.success(),
            "Codex rejected generated hooks: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("codex-cli"));
    }

    #[test]
    fn managed_agent_identity_must_be_complete_and_match_the_runtime() {
        let mut request = TerminalRuntimeCreateRequest {
            runtime_id: Some("runtime-1".into()),
            context: None,
            target_id: "local:powershell".into(),
            title: None,
            cwd: None,
            command: None,
            authentication: None,
            managed_agent: None,
            launch_environment: Default::default(),
            cols: 100,
            rows: 30,
        };
        let mut agent = TerminalManagedAgentContext {
            mux_session_id: "session-1".into(),
            pane_id: "pane-1".into(),
            runtime_id: "runtime-1".into(),
            agent_id: "agent-1".into(),
            launch_profile_id: "codex.default".into(),
        };
        validate_managed_agent_context(&request, &agent).unwrap();
        agent.runtime_id = "runtime-2".into();
        assert!(validate_managed_agent_context(&request, &agent).is_err());
        agent.runtime_id = "runtime-1".into();
        agent.pane_id.clear();
        assert!(validate_managed_agent_context(&request, &agent).is_err());
        request.runtime_id = None;
        assert!(validate_managed_agent_context(&request, &agent).is_err());
    }
}

#[tauri::command]
pub fn managed_agent_profiles_list() -> Vec<AgentLaunchProfile> {
    agent_profiles::profiles()
}

#[tauri::command]
pub async fn managed_agent_profile_availability(
    profile_id: String,
    target_id: String,
) -> Result<AgentProfileAvailability, String> {
    agent_profiles::availability(&profile_id, &target_id).await
}

#[tauri::command]
pub fn managed_agents_events(state: State<AppState>) -> Vec<ManagedAgentEvent> {
    state.agent_hooks.events()
}

#[tauri::command]
pub fn managed_agents_set_notification_focus(
    state: State<AppState>,
    mux_session_id: Option<String>,
    pane_id: Option<String>,
    terminal_visible: bool,
) {
    *state
        .agent_notification_focus
        .lock()
        .expect("agent notification focus lock") = AgentNotificationFocus {
        mux_session_id,
        pane_id,
        terminal_visible,
    };
}

#[tauri::command]
pub fn control_audit_list(
    state: State<AppState>,
    limit: usize,
) -> Result<Vec<ControlAuditRecord>, String> {
    state.db.list_control_audit(limit)
}

#[tauri::command]
pub fn control_audit_clear(state: State<AppState>) -> Result<usize, String> {
    state.db.clear_control_audit()
}

#[tauri::command]
pub async fn terminal_targets_list(
    state: State<'_, AppState>,
) -> Result<Vec<TerminalTarget>, String> {
    let backend = state.terminal_backend.clone();
    tauri::async_runtime::spawn_blocking(move || backend.targets())
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn terminal_runtimes_list(state: State<AppState>) -> Result<Vec<TerminalRuntime>, String> {
    state.terminal_backend.list()
}

#[tauri::command]
pub fn terminal_runtime_read_output(
    state: State<AppState>,
    runtime_id: String,
    from_cursor: u64,
    max_bytes: usize,
) -> Result<TerminalRuntimeOutputReadResult, String> {
    state
        .terminal_backend
        .read_output(&runtime_id, from_cursor, max_bytes)
}

#[tauri::command]
pub async fn terminal_runtime_write(
    state: State<'_, AppState>,
    runtime_id: String,
    data: String,
) -> Result<(), String> {
    state.terminal_backend.write(&runtime_id, &data).await
}

#[tauri::command]
pub async fn terminal_runtime_resize(
    state: State<'_, AppState>,
    runtime_id: String,
    cols: u32,
    rows: u32,
) -> Result<(), String> {
    state.terminal_backend.resize(&runtime_id, cols, rows).await
}

#[tauri::command]
pub fn terminal_runtime_flow(
    state: State<AppState>,
    runtime_id: String,
    paused: bool,
) -> Result<(), String> {
    state
        .terminal_backend
        .set_output_paused(&runtime_id, paused)
}

#[tauri::command]
pub async fn terminal_runtime_interrupt(
    state: State<'_, AppState>,
    runtime_id: String,
) -> Result<(), String> {
    state.terminal_backend.interrupt(&runtime_id).await
}

#[tauri::command]
pub async fn terminal_runtime_close(
    state: State<'_, AppState>,
    runtime_id: String,
) -> Result<(), String> {
    state.terminal_backend.close(&runtime_id).await
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
pub fn mux_sessions_list(state: State<AppState>) -> Result<Vec<MuxSession>, String> {
    state.db.list_mux_sessions()
}

#[tauri::command]
pub fn mux_sessions_save(
    state: State<AppState>,
    input: MuxSessionInput,
) -> Result<MuxSession, String> {
    let session = state.db.save_mux_session(input)?;
    state.db.ensure_session_browser_resource(&session.id)?;
    Ok(session)
}

#[tauri::command]
pub fn mux_sessions_remove(state: State<AppState>, id: String) -> Result<(), String> {
    state.db.delete_mux_session(&id)
}

#[tauri::command]
pub fn mux_panes_list(
    state: State<AppState>,
    mux_session_id: Option<String>,
) -> Result<Vec<MuxPane>, String> {
    state.db.list_mux_panes(mux_session_id.as_deref())
}

#[tauri::command]
pub fn mux_panes_save(state: State<AppState>, input: MuxPaneInput) -> Result<MuxPane, String> {
    state.db.save_mux_pane(input)
}

#[tauri::command]
pub fn mux_panes_remove(state: State<AppState>, id: String) -> Result<(), String> {
    state.db.delete_mux_pane(&id)
}

#[tauri::command]
pub fn browser_resources_list(
    state: State<AppState>,
    mux_session_id: Option<String>,
) -> Result<Vec<BrowserResource>, String> {
    state.db.list_browser_resources(mux_session_id.as_deref())
}

#[tauri::command]
pub fn browser_resources_save(
    state: State<AppState>,
    input: BrowserResourceInput,
) -> Result<BrowserResource, String> {
    state.db.save_browser_resource(input)
}

#[tauri::command]
pub async fn browser_resources_remove(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let runtime_ids = state
        .browser_runtimes
        .list()?
        .into_iter()
        .filter(|runtime| runtime.browser_resource_id == id)
        .map(|runtime| runtime.id)
        .collect::<Vec<_>>();
    for runtime_id in runtime_ids {
        state.browser_runtimes.close(&runtime_id).await?;
    }
    state.db.delete_browser_resource(&id)?;
    state.luna_mcp.refresh_target_resource("browser", &id)
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
    state.db.get_setting("uiTheme", UiTheme::default())
}

#[tauri::command]
pub fn settings_get_language(state: State<AppState>) -> String {
    state.db.get_setting("language", "zh-CN".to_string())
}

#[tauri::command]
pub fn settings_get_remote_agent_integration_enabled(state: State<AppState>) -> bool {
    state.db.get_setting("remoteAgentIntegrationEnabled", false)
}

#[tauri::command]
pub fn settings_save_remote_agent_integration_enabled(
    state: State<AppState>,
    enabled: bool,
) -> Result<bool, String> {
    state
        .db
        .set_setting("remoteAgentIntegrationEnabled", &enabled)?;
    Ok(enabled)
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
    save_ui_theme(&window, &state.db, theme)
}

pub(crate) fn save_ui_theme(
    window: &WebviewWindow,
    database: &Database,
    theme: UiTheme,
) -> Result<UiTheme, String> {
    database.set_setting("uiTheme", &theme)?;
    window
        .set_theme(match &theme {
            UiTheme::System => None,
            UiTheme::Light => Some(Theme::Light),
            UiTheme::Dark => Some(Theme::Dark),
        })
        .map_err(|e| e.to_string())?;
    window
        .app_handle()
        .emit("ui-theme:changed", theme.clone())
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
    settings: TerminalSettings,
) -> Result<TerminalSettings, String> {
    save_terminal_settings(&state.db, None, settings)
}

pub(crate) fn save_terminal_settings(
    database: &Database,
    window: Option<&WebviewWindow>,
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
    database.set_setting("terminalSettings", &settings)?;
    if let Some(window) = window {
        window
            .emit("terminal-settings:changed", settings.clone())
            .map_err(|error| error.to_string())?;
    }
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

fn browser_tunnel_route(
    remote_url: &str,
    local_port: u16,
) -> Result<(String, String, u16), String> {
    let value = remote_url.trim();
    let value = if value.contains("://") {
        value.to_string()
    } else {
        format!("http://{value}")
    };
    let mut url = url::Url::parse(&value).map_err(|error| format!("远端服务 URL 无效: {error}"))?;
    if url.scheme() != "http" {
        return Err("首版 SSH Browser 转发只支持 http URL".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("远端服务 URL 不能包含用户名或密码".into());
    }
    let target_host = url
        .host_str()
        .filter(|host| !host.is_empty())
        .ok_or_else(|| "远端服务 URL 缺少主机名".to_string())?
        .to_string();
    let target_port = url
        .port_or_known_default()
        .ok_or_else(|| "远端服务 URL 缺少有效端口".to_string())?;
    url.set_host(Some("127.0.0.1"))
        .map_err(|_| "无法生成 Browser 本地转发 URL".to_string())?;
    url.set_port(Some(local_port))
        .map_err(|_| "无法设置 Browser 本地转发端口".to_string())?;
    Ok((url.to_string(), target_host, target_port))
}

#[tauri::command]
pub async fn browser_tunnel_start(
    state: State<'_, AppState>,
    session_id: String,
    browser_resource_id: String,
    source_pane_id: String,
    remote_url: String,
) -> Result<BrowserTunnel, String> {
    let browser_resource_id = browser_resource_id.trim();
    let source_pane_id = source_pane_id.trim();
    if browser_resource_id.is_empty() || browser_resource_id.len() > 128 {
        return Err("浏览器资源 ID 无效".into());
    }
    if source_pane_id.is_empty() || source_pane_id.len() > 128 {
        return Err("来源 SSH Pane ID 无效".into());
    }
    let (_, target_host, target_port) = browser_tunnel_route(&remote_url, 1)?;
    let bookmark_id = state.sessions.bookmark_id(&session_id)?;
    let panes = state.db.list_mux_panes(None)?;
    let source_pane = panes
        .iter()
        .find(|pane| pane.id == source_pane_id)
        .ok_or_else(|| "来源 SSH Pane 不存在".to_string())?;
    if source_pane.kind != MuxPaneKind::Terminal || source_pane.bookmark_id != bookmark_id {
        return Err("来源 Pane 与当前 SSH Runtime 不匹配".into());
    }
    let browser_resource = state
        .db
        .list_browser_resources(None)?
        .iter()
        .find(|resource| resource.id == browser_resource_id)
        .cloned()
        .ok_or_else(|| "浏览器资源不存在".to_string())?;
    if browser_resource.mux_session_id != source_pane.mux_session_id
        || browser_resource.source_pane_id != source_pane.id
    {
        return Err("浏览器资源与来源 SSH Pane 不匹配".into());
    }
    let profile = PortForwardProfile {
        id: format!("browser-resource:{browser_resource_id}"),
        bookmark_id,
        name: browser_resource.name,
        forward_type: PortForwardType::Local,
        bind_address: "127.0.0.1".into(),
        bind_port: 0,
        target_host,
        target_port,
    };
    let tunnel = state.tunnels.start(session_id, profile).await?;
    let (local_url, _, _) = browser_tunnel_route(&remote_url, tunnel.bind_port)?;
    Ok(BrowserTunnel { tunnel, local_url })
}
#[tauri::command]
pub async fn tunnels_stop(
    state: State<'_, AppState>,
    session_id: String,
    tunnel_id: String,
) -> Result<(), String> {
    state.tunnels.stop(&session_id, &tunnel_id).await
}

#[cfg(test)]
mod browser_tunnel_tests {
    use super::browser_tunnel_route;

    #[test]
    fn rewrites_remote_http_url_to_an_assigned_loopback_port() {
        let (local, host, port) =
            browser_tunnel_route("http://127.0.0.1:3000/app?q=1#view", 49152).unwrap();
        assert_eq!(local, "http://127.0.0.1:49152/app?q=1#view");
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 3000);
    }

    #[test]
    fn rejects_unsupported_or_credential_bearing_remote_urls() {
        assert!(browser_tunnel_route("https://localhost:3000", 49152).is_err());
        assert!(browser_tunnel_route("http://user:secret@localhost:3000", 49152).is_err());
    }
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
