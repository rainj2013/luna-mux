mod commands;
mod database;
mod desktop;
mod models;
mod sessions;
mod ssh_config;
mod transfers;
mod tunnels;

use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use commands::*;
use database::Database;
use sessions::SessionManager;
use tauri::{Manager, RunEvent};
use transfers::TransferManager;
use tunnels::TunnelManager;

pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .on_menu_event(|app, event| desktop::handle_menu(app, event.id().as_ref()));
    #[cfg(target_os = "macos")]
    let builder = builder.menu(desktop::menu);
    builder
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|error| error.to_string())?;
            let base = dirs::data_dir().unwrap_or_default();
            let legacy = [
                base.join("com.local.sshclient.tauri-preview")
                    .join("ssh-client.db"),
                base.join("ssh-client").join("ssh-client.db"),
            ]
            .into_iter()
            .filter(|path| *path != data_dir.join("ssh-client.db"))
            .collect::<Vec<_>>();
            let database = Arc::new(Database::open(
                &data_dir.join("ssh-client.db"),
                &legacy,
                &format!("{}.credentials", app.config().identifier),
            )?);
            let sessions = SessionManager::new(app.handle().clone(), database.clone());
            let transfers =
                TransferManager::new(app.handle().clone(), database.clone(), sessions.clone());
            let tunnels = TunnelManager::new(app.handle().clone(), sessions.clone());
            app.manage(AppState {
                db: database,
                sessions,
                transfers,
                tunnels,
                allowed_imports: Mutex::new(HashSet::new()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            platform,
            system_open_external,
            bookmarks_list,
            bookmarks_save,
            bookmarks_remove,
            bookmarks_forget_credential,
            bookmarks_preview_ssh_config,
            bookmarks_import_ssh_config,
            sessions_list,
            sessions_connect,
            sessions_disconnect,
            sessions_write,
            sessions_resize,
            sessions_flow,
            sessions_host_key_decision,
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
            state_get_open_bookmarks,
            state_set_open_bookmarks,
            state_get_sidebar_collapsed,
            state_set_sidebar_collapsed,
            state_get_sidebar_width,
            state_set_sidebar_width,
            settings_get_ui_theme,
            settings_save_ui_theme,
            settings_get_terminal,
            settings_save_terminal,
            settings_choose_terminal_background,
            settings_load_terminal_background,
            settings_get_app_icons,
            settings_set_app_icon
        ])
        .build(tauri::generate_context!())
        .expect("failed to build SSH Client")
        .run(|app, event| {
            if matches!(event, RunEvent::ExitRequested { .. } | RunEvent::Exit) {
                let state = app.state::<AppState>();
                let sessions = state.sessions.clone();
                let tunnels = state.tunnels.clone();
                let ids = sessions
                    .list()
                    .into_iter()
                    .map(|item| item.id)
                    .collect::<Vec<_>>();
                tauri::async_runtime::block_on(async move {
                    for id in ids {
                        tunnels.stop_session(&id).await;
                    }
                    sessions.disconnect_all().await;
                });
            }
        });
}
