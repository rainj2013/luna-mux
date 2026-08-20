mod agent_adapters;
mod agent_command;
mod agent_hooks;
mod agent_profiles;
mod ai;
mod app_icon;
mod browser_runtime;
mod claude_code_adapter;
mod clipboard;
mod codex_shim;
mod commands;
mod composite_terminal_backend;
mod control_adapter;
mod control_approval;
mod control_contract;
mod control_service;
mod database;
mod desktop;
mod doctor;
mod legacy_agent_hook_cleanup;
mod local_pty_backend;
#[cfg(test)]
mod local_pty_probe;
mod luna_mcp;
mod luna_mcp_proxy;
mod models;
mod product;
mod runtime_env;
mod sessions;
mod shell_quoting;
mod ssh_config;
mod ssh_terminal_backend;
mod terminal_backend;
mod terminal_output;
mod terminal_runtime_contract;
mod transfers;
mod tunnels;
#[cfg(target_os = "windows")]
mod windows_app_icon;

use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use agent_hooks::{
    AgentHookService, ManagedAgentEvent, ManagedAgentEvidence, ManagedAgentStatus,
    ManagedAgentWaitingReason,
};
use browser_runtime::{
    BrowserRuntimeCreateRequest, BrowserRuntimeManager, BrowserRuntimeStatus, BrowserWarmupGate,
    warm_agent_browser_session,
};
use clipboard::system_clipboard_has_image_file;
use commands::*;
use composite_terminal_backend::CompositeTerminalBackend;
use control_adapter::AuthenticatedControlAdapter;
use control_service::{InProcessControlService, InProcessControlSideEffects};
use database::Database;
use local_pty_backend::InProcessLocalPtyTerminalBackend;
use luna_mcp::LunaMcpService;
use sessions::SessionManager;
use ssh_terminal_backend::InProcessSshTerminalBackend;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use tauri::Emitter;
use tauri::{Manager, RunEvent, Theme};
#[cfg(target_os = "macos")]
use tauri::{
    PhysicalPosition, WebviewUrl, WebviewWindowBuilder,
    window::{Effect, EffectState, EffectsBuilder},
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use terminal_backend::TerminalBackend;
use terminal_runtime_contract::{TerminalRuntimeEvent, TerminalRuntimeStatus};
use transfers::TransferManager;
use tunnels::TunnelManager;

pub use agent_hooks::try_run_hook_forwarder;
pub use browser_runtime::try_run_mcp_browser;
pub use doctor::try_run_agent_check;
pub use doctor::try_run_wsl_interop_probe;
pub use luna_mcp_proxy::try_run_luna_mcp_proxy;

#[cfg(target_os = "windows")]
fn disable_browser_accelerator_keys(window: &tauri::WebviewWindow) -> Result<(), String> {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
    use windows::core::Interface;

    window
        .with_webview(|webview| unsafe {
            let result = (|| -> windows::core::Result<()> {
                let core_webview = webview.controller().CoreWebView2()?;
                let settings = core_webview.Settings()?;
                let settings3 = settings.cast::<ICoreWebView2Settings3>()?;
                settings3.SetAreBrowserAcceleratorKeysEnabled(false)
            })();

            if let Err(error) = result {
                eprintln!("failed to disable WebView2 browser accelerator keys: {error}");
            }
        })
        .map_err(|error| error.to_string())
}

pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .on_menu_event(|app, event| desktop::handle_menu(app, event.id().as_ref()))
        .on_window_event(|window, event| {
            #[cfg(target_os = "macos")]
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (window, event);
        });
    #[cfg(target_os = "macos")]
    let builder = builder.menu(|app| desktop::menu(app, &models::NativeMenuLabels::default()));
    builder
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|error| error.to_string())?;
            runtime_env::cleanup_stale_runtime_dirs();
            match legacy_agent_hook_cleanup::remove_legacy_persistent_hooks() {
                Ok(Some(backup)) => eprintln!(
                    "Removed legacy Luna Mux Codex hooks; backup: {}",
                    backup.display()
                ),
                Ok(None) => {}
                Err(error) => eprintln!("Failed to remove legacy Luna Mux Codex hooks: {error}"),
            }
            let database = Arc::new(Database::open(
                &data_dir.join(product::DATABASE_FILE),
                product::CREDENTIAL_SERVICE,
            )?);
            let browser_runtimes = BrowserRuntimeManager::new(app.handle().clone(), &data_dir);
            let selected_theme = database.get_setting("uiTheme", models::UiTheme::default());
            let window = app
                .get_webview_window("main")
                .ok_or_else(|| "找不到主窗口".to_string())?;
            window
                .set_theme(match &selected_theme {
                    models::UiTheme::System => None,
                    models::UiTheme::Light => Some(Theme::Light),
                    models::UiTheme::Dark => Some(Theme::Dark),
                })
                .map_err(|error| error.to_string())?;
            #[cfg(target_os = "macos")]
            {
                let notification_window = WebviewWindowBuilder::new(
                    app,
                    "agent-notification",
                    WebviewUrl::App("index.html".into()),
                )
                .title("Luna Mux")
                .inner_size(380.0, 104.0)
                .resizable(false)
                .maximizable(false)
                .minimizable(false)
                .closable(false)
                .decorations(false)
                .transparent(true)
                .shadow(true)
                .effects(
                    EffectsBuilder::new()
                        .effect(Effect::Popover)
                        .state(EffectState::Active)
                        .radius(15.0)
                        .build(),
                )
                .always_on_top(true)
                .skip_taskbar(true)
                .visible_on_all_workspaces(true)
                .focused(false)
                .focusable(false)
                .visible(false)
                .build()
                .map_err(|error| error.to_string())?;
                notification_window
                    .set_theme(match &selected_theme {
                        models::UiTheme::System => None,
                        models::UiTheme::Light => Some(Theme::Light),
                        models::UiTheme::Dark => Some(Theme::Dark),
                    })
                    .map_err(|error| error.to_string())?;
            }
            #[cfg(target_os = "windows")]
            {
                window
                    .set_decorations(false)
                    .map_err(|error| error.to_string())?;
                disable_browser_accelerator_keys(&window)?;
            }
            let selected_icon = database.get_setting("appIcon", models::AppIconId::Luna);
            app_icon::apply_at_startup(app.handle(), &selected_icon)?;
            let sessions = SessionManager::new(app.handle().clone(), database.clone());
            let ssh_terminal_backend =
                InProcessSshTerminalBackend::new(database.clone(), sessions.clone());
            let local_pty_backend = InProcessLocalPtyTerminalBackend::new();
            let terminal_backend = CompositeTerminalBackend::new(
                ssh_terminal_backend.clone(),
                local_pty_backend.clone(),
            );
            let agent_hooks = AgentHookService::new();
            agent_hooks.start()?;
            let transfers =
                TransferManager::new(app.handle().clone(), database.clone(), sessions.clone());
            let tunnels = TunnelManager::new(app.handle().clone(), sessions.clone());
            let control_side_effects = InProcessControlSideEffects::new(
                window.clone(),
                database.clone(),
                transfers.clone(),
                tunnels.clone(),
                browser_runtimes.clone(),
                sessions.clone(),
            );
            let control = InProcessControlService::new(
                database.clone(),
                terminal_backend.clone(),
                control_side_effects,
                agent_hooks.clone(),
            );
            let control_adapter = AuthenticatedControlAdapter::new(control.clone());
            let luna_mcp = LunaMcpService::new(
                control_adapter.clone(),
                database.clone(),
                agent_hooks.clone(),
                terminal_backend.clone(),
            );
            control.set_luna_mcp(luna_mcp.clone());
            luna_mcp.start()?;
            let browser_ensure_db = database.clone();
            let browser_ensure_runtimes = browser_runtimes.clone();
            let browser_ensure_mcp = luna_mcp.clone();
            let browser_warmup_gate = BrowserWarmupGate::default();
            agent_hooks.set_browser_ensure(Arc::new(move |mux_session_id| {
                let database = browser_ensure_db.clone();
                let browser_runtimes = browser_ensure_runtimes.clone();
                let luna_mcp = browser_ensure_mcp.clone();
                let warmup_gate = browser_warmup_gate.clone();
                Box::pin(async move {
                    let running = browser_runtimes
                        .list()?
                        .into_iter()
                        .filter(|runtime| {
                            runtime.mux_session_id == mux_session_id
                                && runtime.status == BrowserRuntimeStatus::Running
                        })
                        .collect::<Vec<_>>();
                    if running.len() == 1 {
                        let runtime = &running[0];
                        return warmup_gate
                            .warm_once(&mux_session_id, &runtime.id, || {
                                warm_agent_browser_session(&mux_session_id, runtime.cdp_port)
                            })
                            .await;
                    }
                    if running.len() > 1 {
                        return Err(
                            "当前 Session 同时运行了多个 Browser Resource，请先只保留一个"
                                .into(),
                        );
                    }
                    let local_resources = database
                        .list_browser_resources(Some(&mux_session_id))?
                        .into_iter()
                        .filter(|resource| resource.source_pane_id.is_empty())
                        .collect::<Vec<_>>();
                    let resource = match local_resources.as_slice() {
                        [resource] => resource,
                        [] => {
                            return Err(
                                "当前 Session 没有可自动启动的本地 Browser Resource，请先创建一个"
                                    .into(),
                            );
                        }
                        _ => {
                            return Err(
                                "当前 Session 有多个本地 Browser Resource，无法自动选择，请先手动启动其中一个"
                                    .into(),
                            );
                        }
                    };
                    let runtime = browser_runtimes
                        .create(BrowserRuntimeCreateRequest {
                            mux_session_id: mux_session_id.clone(),
                            browser_resource_id: resource.id.clone(),
                            url: String::new(),
                            temporary_profile: resource.temporary_profile,
                        })
                        .await?;
                    if let Err(error) = luna_mcp.refresh_target_resource("browser", &resource.id) {
                        let _ = browser_runtimes.close(&runtime.id).await;
                        return Err(error);
                    }
                    warmup_gate
                        .warm_once(&mux_session_id, &runtime.id, || {
                            warm_agent_browser_session(&mux_session_id, runtime.cdp_port)
                        })
                        .await
                })
            }));
            let agent_app_handle = app.handle().clone();
            let agent_luna_mcp = luna_mcp.clone();
            let agent_notification_db = database.clone();
            let agent_notification_window = window.clone();
            let agent_notification_focus = Arc::new(Mutex::new(AgentNotificationFocus::default()));
            let event_notification_focus = agent_notification_focus.clone();
            agent_hooks.set_event_sink(Arc::new(move |event| {
                let _ = agent_luna_mcp.refresh_source_pane(&event.context.pane_id);
                if should_send_agent_desktop_notification(&event) {
                    let focused_pane_is_visible = agent_notification_window
                        .is_focused()
                        .unwrap_or(false)
                        && event_notification_focus
                            .lock()
                            .ok()
                            .is_some_and(|focus| {
                                focus.terminal_visible
                                    && focus.mux_session_id.as_deref()
                                        == Some(event.context.mux_session_id.as_str())
                                    && focus.pane_id.as_deref()
                                        == Some(event.context.pane_id.as_str())
                            });
                    if !focused_pane_is_visible {
                        let language = agent_notification_db
                            .get_setting("language", "zh-CN".to_string());
                        let pane_title = agent_notification_db
                            .list_mux_panes(Some(&event.context.mux_session_id))
                            .ok()
                            .and_then(|panes| {
                                panes
                                    .into_iter()
                                    .find(|pane| pane.id == event.context.pane_id)
                                    .map(|pane| pane.title)
                            })
                            .unwrap_or_else(|| product::DISPLAY_NAME.to_string());
                        let body = agent_desktop_notification_body(&event, &language);
                        show_agent_desktop_notification(
                            &agent_app_handle,
                            &event,
                            pane_title,
                            body.into(),
                        );
                    }
                }
                let _ = tauri::Emitter::emit(&agent_app_handle, "managed-agent:event", event);
            }));
            let control_events = control.event_buffer();
            let runtime_agent_hooks = agent_hooks.clone();
            let runtime_luna_mcp = luna_mcp.clone();
            let app_handle = app.handle().clone();
            terminal_backend.set_event_sink(Arc::new(move |event| {
                if let TerminalRuntimeEvent::Status(status) = &event
                    && let Some(mux_session_id) = status
                        .runtime
                        .managed_agent
                        .as_ref()
                        .map(|context| context.mux_session_id.as_str())
                        .or_else(|| {
                            status
                                .runtime
                                .context
                                .as_ref()
                                .map(|context| context.mux_session_id.as_str())
                        })
                {
                    let _ = runtime_luna_mcp.refresh_session(mux_session_id);
                }
                if let TerminalRuntimeEvent::Exit(exit) = &event {
                    runtime_agent_hooks.revoke_runtime(&exit.runtime_id);
                    runtime_luna_mcp.revoke_runtime(&exit.runtime_id);
                    agent_adapters::cleanup(&exit.runtime_id);
                }
                if let TerminalRuntimeEvent::Output(output) = &event {
                    runtime_agent_hooks.record_terminal_output(output);
                }
                control_events.record_runtime_event(event.clone());
                let _ = tauri::Emitter::emit(&app_handle, "terminal-runtime:event", event);
            }));
            app.manage(AppState {
                db: database,
                sessions,
                ssh_terminal_backend,
                local_pty_backend,
                terminal_backend,
                control,
                control_adapter,
                luna_mcp,
                agent_hooks,
                transfers,
                tunnels,
                browser_runtimes,
                agent_notification_focus,
                ai_diagnostics: ai::AiDiagnostics::default(),
                allowed_imports: Mutex::new(HashSet::new()),
                pending_archive_imports: Mutex::new(std::collections::HashMap::new()),
                pending_luna_remote_imports: Mutex::new(std::collections::HashMap::new()),
                exit_cleanup_started: AtomicBool::new(false),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            platform,
            system_open_external,
            system_clipboard_has_image_file,
            browser_chrome_discover,
            browser_runtime_create,
            browser_runtimes_list,
            browser_runtime_close,
            browser_runtime_navigate,
            browser_runtime_focus_external,
            browser_runtime_resize,
            browser_runtime_mouse,
            browser_runtime_key,
            bookmarks_list,
            mux_sessions_list,
            mux_sessions_save,
            mux_sessions_remove,
            mux_panes_list,
            mux_panes_save,
            mux_panes_remove,
            browser_resources_list,
            browser_resources_save,
            browser_resources_remove,
            bookmarks_save,
            bookmarks_reorder,
            bookmarks_move_to_group,
            bookmark_groups_list,
            bookmark_groups_create,
            bookmark_groups_rename,
            bookmark_groups_delete,
            bookmark_groups_reorder,
            bookmarks_duplicate,
            bookmarks_remove,
            bookmarks_forget_credential,
            bookmarks_preview_ssh_config,
            bookmarks_import_ssh_config,
            bookmarks_export_archive,
            bookmarks_preview_archive,
            bookmarks_import_archive,
            bookmarks_discover_luna_remote_sources,
            bookmarks_preview_luna_remote,
            bookmarks_choose_luna_remote_database,
            bookmarks_import_luna_remote,
            sessions_connect,
            sessions_disconnect,
            sessions_write,
            sessions_resize,
            sessions_flow,
            sessions_host_key_decision,
            terminal_runtime_create,
            terminal_targets_list,
            terminal_runtimes_list,
            terminal_runtime_read_output,
            terminal_runtime_write,
            terminal_runtime_resize,
            terminal_runtime_flow,
            terminal_runtime_interrupt,
            terminal_runtime_close,
            managed_agent_profiles_list,
            managed_agent_profile_availability,
            managed_agents_events,
            managed_agents_set_notification_focus,
            managed_agents_activate_notification,
            control_catalog,
            control_invoke,
            control_read_events,
            control_approval_resolve,
            control_audit_list,
            control_audit_clear,
            files_home,
            files_parent_local,
            files_remote_home,
            files_list_local,
            files_list_remote,
            files_create_directory,
            files_rename,
            files_remove,
            files_preview,
            files_get_favorites,
            files_set_favorites,
            files_choose_local_directory,
            files_choose_private_key,
            transfers_list,
            transfers_enqueue,
            transfers_cancel,
            transfers_retry,
            transfers_resolve_conflict,
            transfers_clear_completed,
            deployments_list,
            deployments_save,
            deployments_remove,
            deployments_preview,
            deployments_execute,
            tunnels_list_profiles,
            tunnels_save_profile,
            tunnels_remove_profile,
            tunnels_list,
            tunnels_start,
            browser_tunnel_start,
            tunnels_stop,
            diagnostics_run,
            diagnostics_repair,
            diagnostics_export,
            state_get_sidebar_collapsed,
            state_set_sidebar_collapsed,
            state_get_collapsed_bookmark_groups,
            state_set_collapsed_bookmark_groups,
            state_get_sidebar_width,
            state_set_sidebar_width,
            settings_get_ui_theme,
            settings_save_ui_theme,
            settings_get_language,
            settings_get_remote_agent_integration_enabled,
            settings_apply_language,
            settings_save_language,
            settings_save_remote_agent_integration_enabled,
            settings_get_terminal,
            settings_save_terminal,
            settings_list_system_fonts,
            settings_choose_terminal_background,
            settings_load_terminal_background,
            settings_get_app_icons,
            settings_set_app_icon,
            ai_settings_get,
            ai_settings_save,
            ai_settings_delete_key,
            ai_settings_test,
            ai_command_generate,
            ai_command_analyze,
            ai_command_history_list,
            ai_command_history_clear,
            ai_diagnostics_get,
            ai_diagnostics_clear
        ])
        .build(tauri::generate_context!())
        .unwrap_or_else(|error| panic!("failed to build {}: {error}", product::DISPLAY_NAME))
        .run(|app, event| match event {
            RunEvent::ExitRequested { api, code, .. } => {
                let state = app.state::<AppState>();
                if state.exit_cleanup_started.load(Ordering::Acquire) {
                    return;
                }
                api.prevent_exit();
                let active_runtime_count = state
                    .terminal_backend
                    .list()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|runtime| {
                        matches!(
                            runtime.status,
                            TerminalRuntimeStatus::Starting
                                | TerminalRuntimeStatus::Connecting
                                | TerminalRuntimeStatus::Running
                        )
                    })
                    .count()
                    + state
                        .browser_runtimes
                        .list()
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|runtime| {
                            matches!(
                                runtime.status,
                                browser_runtime::BrowserRuntimeStatus::Starting
                                    | browser_runtime::BrowserRuntimeStatus::Running
                            )
                        })
                        .count();
                if active_runtime_count == 0 {
                    begin_exit_cleanup(app, code.unwrap_or(0));
                    return;
                }
                let language = state.db.get_setting("language", "zh-CN".to_string());
                let (title, message, confirm, cancel) =
                    exit_confirmation_copy(&language, active_runtime_count);
                let handle = app.clone();
                app.dialog()
                    .message(message)
                    .title(title)
                    .kind(MessageDialogKind::Warning)
                    .buttons(MessageDialogButtons::OkCancelCustom(confirm, cancel))
                    .show(move |confirmed| {
                        if confirmed {
                            begin_exit_cleanup(&handle, code.unwrap_or(0));
                        }
                    });
            }
            #[cfg(target_os = "macos")]
            RunEvent::Exit => {
                app.state::<AppState>()
                    .browser_runtimes
                    .force_cleanup_managed_processes();
            }
            #[cfg(target_os = "macos")]
            RunEvent::Reopen { .. } => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
            _ => {}
        });
}

fn exit_confirmation_copy(
    language: &str,
    active_runtime_count: usize,
) -> (String, String, String, String) {
    if language.starts_with("zh") {
        (
            format!("退出 {}？", product::DISPLAY_NAME),
            format!("仍有 {active_runtime_count} 个终端进程正在运行。退出会关闭这些进程。"),
            "退出并关闭进程".into(),
            "取消".into(),
        )
    } else {
        (
            format!("Quit {}?", product::DISPLAY_NAME),
            format!(
                "{active_runtime_count} terminal process(es) are still running. Quitting will close them."
            ),
            "Quit and close processes".into(),
            "Cancel".into(),
        )
    }
}

fn should_send_agent_desktop_notification(event: &ManagedAgentEvent) -> bool {
    event.evidence == ManagedAgentEvidence::StructuredHook
        && matches!(
            event.status,
            ManagedAgentStatus::Waiting | ManagedAgentStatus::Completed | ManagedAgentStatus::Error
        )
        && !matches!(
            event.hook_event_name.as_str(),
            "SessionEnd" | "RuntimeExit" | "AgentProcessExit"
        )
}

fn agent_desktop_notification_body(event: &ManagedAgentEvent, language: &str) -> &'static str {
    let chinese = language.starts_with("zh");
    match (&event.status, &event.waiting_reason, chinese) {
        (ManagedAgentStatus::Waiting, Some(ManagedAgentWaitingReason::Permission), true) => {
            "Agent 正在等待权限确认"
        }
        (ManagedAgentStatus::Waiting, Some(ManagedAgentWaitingReason::Permission), false) => {
            "Agent is waiting for permission"
        }
        (ManagedAgentStatus::Waiting, _, true) => "Agent 正在等待输入",
        (ManagedAgentStatus::Waiting, _, false) => "Agent is waiting for input",
        (ManagedAgentStatus::Completed, _, true) => "Agent 已完成任务",
        (ManagedAgentStatus::Completed, _, false) => "Agent completed its task",
        (ManagedAgentStatus::Error, _, true) => "Agent 运行出错",
        (ManagedAgentStatus::Error, _, false) => "Agent encountered an error",
        _ => {
            if chinese {
                "Agent 状态已更新"
            } else {
                "Agent status changed"
            }
        }
    }
}

#[cfg(target_os = "macos")]
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentDesktopNotification<'a> {
    title: &'a str,
    body: &'a str,
    status: &'a ManagedAgentStatus,
    mux_session_id: &'a str,
    pane_id: &'a str,
    sequence: u64,
}

#[cfg(target_os = "macos")]
fn show_agent_desktop_notification(
    app: &tauri::AppHandle,
    event: &ManagedAgentEvent,
    title: String,
    body: String,
) {
    let Some(notification) = app.get_webview_window("agent-notification") else {
        eprintln!("failed to find Agent notification window");
        return;
    };
    let payload = AgentDesktopNotification {
        title: &title,
        body: &body,
        status: &event.status,
        mux_session_id: &event.context.mux_session_id,
        pane_id: &event.context.pane_id,
        sequence: event.sequence,
    };
    if let Some(main) = app.get_webview_window("main")
        && let Ok(Some(monitor)) = main.current_monitor()
        && let Ok(size) = notification.outer_size()
    {
        let scale = monitor.scale_factor();
        let margin = (16.0 * scale).round() as i32;
        let top_offset = (28.0 * scale).round() as i32;
        let x = monitor.position().x + monitor.size().width as i32 - size.width as i32 - margin;
        let y = monitor.position().y + top_offset;
        let _ = notification.set_position(PhysicalPosition::new(x, y));
    }
    if let Err(error) = notification.emit("managed-agent:desktop-notification", payload) {
        eprintln!("failed to update Agent notification window: {error}");
        return;
    }
    if let Err(error) = notification.show() {
        eprintln!("failed to show Agent notification window: {error}");
    }
}

#[cfg(target_os = "windows")]
fn show_agent_desktop_notification(
    app: &tauri::AppHandle,
    event: &ManagedAgentEvent,
    title: String,
    body: String,
) {
    let app = app.clone();
    let mux_session_id = event.context.mux_session_id.clone();
    let pane_id = event.context.pane_id.clone();
    let sequence = event.sequence;
    std::thread::spawn(move || {
        let mut notification = notify_rust::Notification::new();
        notification
            .summary(&title)
            .body(&body)
            .sound_name("default");
        match notification.show() {
            Ok(handle) => {
                let response_result = handle.wait_for_response(
                    move |response: &notify_rust::NotificationResponse| {
                        eprintln!("Agent system notification response: {response:?}");
                        if should_activate_windows_agent_notification(response)
                            && let Err(error) = activate_agent_notification(
                                &app,
                                &mux_session_id,
                                &pane_id,
                                sequence,
                            )
                        {
                            eprintln!("failed to activate Agent notification target: {error}");
                        }
                    },
                );
                if let Err(error) = response_result {
                    eprintln!("failed to wait for Agent system notification response: {error}");
                }
            }
            Err(error) => eprintln!("failed to deliver Agent system notification: {error}"),
        }
    });
}

#[cfg(target_os = "windows")]
fn should_activate_windows_agent_notification(
    response: &notify_rust::NotificationResponse,
) -> bool {
    use notify_rust::NotificationResponse;

    match response {
        NotificationResponse::Default
        | NotificationResponse::Action(_)
        | NotificationResponse::Reply(_) => true,
        NotificationResponse::Closed(_) => false,
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn show_agent_desktop_notification(
    _app: &tauri::AppHandle,
    _event: &ManagedAgentEvent,
    _title: String,
    _body: String,
) {
}

fn activate_agent_notification(
    app: &tauri::AppHandle,
    mux_session_id: &str,
    pane_id: &str,
    sequence: u64,
) -> Result<(), String> {
    let main = app
        .get_webview_window("main")
        .ok_or_else(|| "找不到主窗口".to_string())?;
    main.show().map_err(|error| error.to_string())?;
    main.unminimize().map_err(|error| error.to_string())?;
    main.set_focus().map_err(|error| error.to_string())?;
    main.emit(
        "managed-agent:activate-pane",
        serde_json::json!({
            "muxSessionId": mux_session_id,
            "paneId": pane_id,
            "sequence": sequence,
        }),
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
fn managed_agents_activate_notification(
    app: tauri::AppHandle,
    mux_session_id: String,
    pane_id: String,
    sequence: u64,
) -> Result<(), String> {
    activate_agent_notification(&app, &mux_session_id, &pane_id, sequence)
}

fn begin_exit_cleanup(app: &tauri::AppHandle, code: i32) {
    let state = app.state::<AppState>();
    if state.exit_cleanup_started.swap(true, Ordering::AcqRel) {
        return;
    }
    let sessions = state.sessions.clone();
    let local_pty_backend = state.local_pty_backend.clone();
    let tunnels = state.tunnels.clone();
    let luna_mcp = state.luna_mcp.clone();
    let browser_runtimes = state.browser_runtimes.clone();
    let ids = sessions
        .list()
        .into_iter()
        .map(|item| item.id)
        .collect::<Vec<_>>();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        luna_mcp.shutdown();
        let non_browser_cleanup = async {
            for id in ids {
                tunnels.stop_session(&id).await;
            }
            sessions.disconnect_all().await;
            local_pty_backend.close_all().await;
        };
        let cleanup = async {
            tokio::join!(browser_runtimes.close_all(), non_browser_cleanup);
        };
        let _ = tokio::time::timeout(Duration::from_secs(5), cleanup).await;
        browser_runtimes.force_cleanup_managed_processes();
        app.exit(code);
    });
}

#[cfg(test)]
mod exit_tests {
    use super::{
        agent_desktop_notification_body, exit_confirmation_copy,
        should_send_agent_desktop_notification,
    };
    use crate::{
        agent_hooks::{
            ManagedAgentEvent, ManagedAgentEvidence, ManagedAgentStatus, ManagedAgentWaitingReason,
        },
        terminal_runtime_contract::TerminalManagedAgentContext,
    };

    #[test]
    fn exit_confirmation_is_localized() {
        let zh = exit_confirmation_copy("zh-CN", 2);
        assert!(zh.1.contains('2'));
        assert!(zh.2.contains("关闭进程"));
        let en = exit_confirmation_copy("en", 3);
        assert!(en.1.contains('3'));
        assert_eq!(en.3, "Cancel");
    }

    fn agent_event(hook: &str, status: ManagedAgentStatus) -> ManagedAgentEvent {
        ManagedAgentEvent {
            sequence: 1,
            timestamp: "2026-08-16T00:00:00Z".into(),
            context: TerminalManagedAgentContext {
                mux_session_id: "session-1".into(),
                pane_id: "pane-1".into(),
                runtime_id: "runtime-1".into(),
                agent_id: "agent-1".into(),
                launch_profile_id: "codex.default".into(),
            },
            adapter_id: "codex".into(),
            agent_session_id: None,
            agent_turn_id: None,
            hook_event_name: hook.into(),
            status,
            waiting_reason: None,
            evidence: ManagedAgentEvidence::StructuredHook,
        }
    }

    #[test]
    fn agent_desktop_notifications_cover_attention_but_not_process_teardown() {
        let mut permission = agent_event("PermissionRequest", ManagedAgentStatus::Waiting);
        permission.waiting_reason = Some(ManagedAgentWaitingReason::Permission);
        assert!(should_send_agent_desktop_notification(&permission));
        assert_eq!(
            agent_desktop_notification_body(&permission, "zh-CN"),
            "Agent 正在等待权限确认"
        );

        let completed = agent_event("Stop", ManagedAgentStatus::Completed);
        assert!(should_send_agent_desktop_notification(&completed));
        assert_eq!(
            agent_desktop_notification_body(&completed, "en"),
            "Agent completed its task"
        );

        let ended = agent_event("SessionEnd", ManagedAgentStatus::Completed);
        assert!(!should_send_agent_desktop_notification(&ended));

        let mut heuristic = agent_event("TerminalBell", ManagedAgentStatus::Waiting);
        heuristic.evidence = ManagedAgentEvidence::TerminalHeuristic;
        assert!(!should_send_agent_desktop_notification(&heuristic));
    }
}
