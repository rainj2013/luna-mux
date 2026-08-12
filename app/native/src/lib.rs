mod ai;
mod app_icon;
mod commands;
mod control_contract;
mod control_service;
mod database;
mod desktop;
mod models;
mod product;
mod sessions;
mod ssh_config;
mod ssh_terminal_backend;
mod terminal_backend;
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

use commands::*;
use control_service::InProcessControlService;
use database::Database;
use sessions::SessionManager;
use ssh_terminal_backend::InProcessSshTerminalBackend;
use tauri::{Manager, RunEvent, Theme};
use terminal_backend::TerminalBackend;
use transfers::TransferManager;
use tunnels::TunnelManager;

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
            let database = Arc::new(Database::open(
                &data_dir.join(product::DATABASE_FILE),
                product::CREDENTIAL_SERVICE,
            )?);
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
            let control = InProcessControlService::new(ssh_terminal_backend.clone());
            let control_events = control.event_buffer();
            let app_handle = app.handle().clone();
            ssh_terminal_backend.set_event_sink(Arc::new(move |event| {
                control_events.record_runtime_event(event.clone());
                let _ = tauri::Emitter::emit(&app_handle, "terminal-runtime:event", event);
            }));
            let transfers =
                TransferManager::new(app.handle().clone(), database.clone(), sessions.clone());
            let tunnels = TunnelManager::new(app.handle().clone(), sessions.clone());
            app.manage(AppState {
                db: database,
                sessions,
                ssh_terminal_backend,
                control,
                transfers,
                tunnels,
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
            bookmarks_list,
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
            control_catalog,
            control_invoke,
            control_read_events,
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
                if state.exit_cleanup_started.swap(true, Ordering::AcqRel) {
                    return;
                }
                api.prevent_exit();
                let sessions = state.sessions.clone();
                let tunnels = state.tunnels.clone();
                let ids = sessions
                    .list()
                    .into_iter()
                    .map(|item| item.id)
                    .collect::<Vec<_>>();
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let cleanup = async {
                        for id in ids {
                            tunnels.stop_session(&id).await;
                        }
                        sessions.disconnect_all().await;
                    };
                    let _ = tokio::time::timeout(Duration::from_secs(5), cleanup).await;
                    app.exit(code.unwrap_or(0));
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
