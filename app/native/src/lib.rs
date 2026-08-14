mod agent_adapter_install;
mod agent_adapters;
mod agent_hooks;
mod agent_profiles;
mod ai;
mod app_icon;
mod browser_runtime;
mod claude_code_adapter;
mod codex_shim;
mod commands;
mod composite_terminal_backend;
mod control_adapter;
mod control_approval;
mod control_contract;
mod control_service;
mod database;
mod desktop;
mod local_pty_backend;
#[cfg(test)]
mod local_pty_probe;
mod luna_mcp;
mod models;
mod product;
mod sessions;
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

use agent_hooks::AgentHookService;
use browser_runtime::{
    BrowserRuntimeCreateRequest, BrowserRuntimeManager, BrowserRuntimeStatus,
    warm_agent_browser_session,
};
use commands::*;
use composite_terminal_backend::CompositeTerminalBackend;
use control_adapter::AuthenticatedControlAdapter;
use control_service::{InProcessControlService, InProcessControlSideEffects};
use database::Database;
use local_pty_backend::InProcessLocalPtyTerminalBackend;
use luna_mcp::LunaMcpService;
use sessions::SessionManager;
use ssh_terminal_backend::InProcessSshTerminalBackend;
use tauri::{Manager, RunEvent, Theme};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use terminal_backend::TerminalBackend;
use terminal_runtime_contract::{TerminalRuntimeEvent, TerminalRuntimeStatus};
use transfers::TransferManager;
use tunnels::TunnelManager;

pub use agent_hooks::try_run_hook_forwarder;
pub use browser_runtime::try_run_mcp_browser;

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
        .plugin(tauri_plugin_notification::init())
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
            let database = Arc::new(Database::open(
                &data_dir.join(product::DATABASE_FILE),
                product::CREDENTIAL_SERVICE,
            )?);
            let browser_runtimes = BrowserRuntimeManager::new(app.handle().clone(), &data_dir);
            let selected_theme = database.get_setting("uiTheme", models::UiTheme::System);
            let window = app
                .get_webview_window("main")
                .ok_or_else(|| "找不到主窗口".to_string())?;
            window
                .set_theme(match selected_theme {
                    models::UiTheme::System => None,
                    models::UiTheme::Light => Some(Theme::Light),
                    models::UiTheme::Dark => Some(Theme::Dark),
                })
                .map_err(|error| error.to_string())?;
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
                database.clone(),
                transfers.clone(),
                tunnels.clone(),
                browser_runtimes.clone(),
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
                Some(browser_runtimes.clone()),
            );
            luna_mcp.start()?;
            let browser_ensure_db = database.clone();
            let browser_ensure_runtimes = browser_runtimes.clone();
            let browser_ensure_mcp = luna_mcp.clone();
            agent_hooks.set_browser_ensure(Arc::new(move |mux_session_id| {
                let database = browser_ensure_db.clone();
                let browser_runtimes = browser_ensure_runtimes.clone();
                let luna_mcp = browser_ensure_mcp.clone();
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
                        return warm_agent_browser_session(
                            &mux_session_id,
                            running[0].cdp_port,
                        )
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
                    warm_agent_browser_session(&mux_session_id, runtime.cdp_port).await
                })
            }));
            let agent_app_handle = app.handle().clone();
            let agent_luna_mcp = luna_mcp.clone();
            agent_hooks.set_event_sink(Arc::new(move |event| {
                let _ = agent_luna_mcp.refresh_source_pane(&event.context.pane_id);
                let _ = tauri::Emitter::emit(&agent_app_handle, "managed-agent:event", event);
            }));
            let control_events = control.event_buffer();
            let runtime_agent_hooks = agent_hooks.clone();
            let runtime_luna_mcp = luna_mcp.clone();
            let app_handle = app.handle().clone();
            terminal_backend.set_event_sink(Arc::new(move |event| {
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
            managed_agent_grants_list,
            managed_agent_grants_save,
            managed_agent_grants_remove,
            managed_agent_adapter_status,
            managed_agent_adapter_preview,
            managed_agent_adapter_apply,
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
            settings_apply_language,
            settings_save_language,
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
        let cleanup = async {
            for id in ids {
                tunnels.stop_session(&id).await;
            }
            sessions.disconnect_all().await;
            local_pty_backend.close_all().await;
            browser_runtimes.close_all().await;
        };
        let _ = tokio::time::timeout(Duration::from_secs(5), cleanup).await;
        app.exit(code);
    });
}

#[cfg(test)]
mod exit_tests {
    use super::exit_confirmation_copy;

    #[test]
    fn exit_confirmation_is_localized() {
        let zh = exit_confirmation_copy("zh-CN", 2);
        assert!(zh.1.contains('2'));
        assert!(zh.2.contains("关闭进程"));
        let en = exit_confirmation_copy("en", 3);
        assert!(en.1.contains('3'));
        assert_eq!(en.3, "Cancel");
    }
}
