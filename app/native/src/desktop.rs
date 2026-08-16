use tauri::{AppHandle, Emitter};

#[cfg(target_os = "macos")]
use tauri::{
    Runtime,
    menu::{Menu, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder},
};

use crate::models::{AppEvent, MenuCommand};

#[cfg(target_os = "macos")]
use crate::{models::NativeMenuLabels, product};

#[cfg(target_os = "macos")]
pub fn menu<R: Runtime>(app: &AppHandle<R>, labels: &NativeMenuLabels) -> tauri::Result<Menu<R>> {
    let settings = MenuItemBuilder::with_id("settings", labels.settings.as_str())
        .accelerator("CmdOrCtrl+,")
        .build(app)?;
    let new_connection = MenuItemBuilder::with_id("new-connection", labels.new_connection.as_str())
        .accelerator("CmdOrCtrl+N")
        .build(app)?;
    let import_connections =
        MenuItemBuilder::with_id("import-connections", labels.import_open_ssh_config.as_str())
            .build(app)?;
    let new_session = MenuItemBuilder::with_id("new-session", labels.new_session.as_str())
        .accelerator("CmdOrCtrl+Shift+T")
        .build(app)?;
    let close_tab = MenuItemBuilder::with_id("close-tab", labels.close_tab.as_str())
        .accelerator("CmdOrCtrl+W")
        .build(app)?;
    let show_terminal = MenuItemBuilder::with_id("show-terminal", labels.terminal.as_str())
        .accelerator("CmdOrCtrl+1")
        .build(app)?;
    let show_files = MenuItemBuilder::with_id("show-files", labels.files.as_str())
        .accelerator("CmdOrCtrl+2")
        .build(app)?;
    let toggle_sidebar = MenuItemBuilder::with_id("toggle-sidebar", labels.toggle_sidebar.as_str())
        .accelerator("CmdOrCtrl+B")
        .build(app)?;
    let help = MenuItemBuilder::with_id("help", labels.help_item.as_str())
        .accelerator("F1")
        .build(app)?;

    let application = SubmenuBuilder::new(app, product::DISPLAY_NAME)
        .item(&PredefinedMenuItem::about(
            app,
            Some(labels.about.as_str()),
            None,
        )?)
        .separator()
        .item(&settings)
        .separator()
        .item(&PredefinedMenuItem::services(
            app,
            Some(labels.services.as_str()),
        )?)
        .separator()
        .item(&PredefinedMenuItem::hide(app, Some(labels.hide.as_str()))?)
        .item(&PredefinedMenuItem::hide_others(
            app,
            Some(labels.hide_others.as_str()),
        )?)
        .item(&PredefinedMenuItem::show_all(
            app,
            Some(labels.show_all.as_str()),
        )?)
        .separator()
        .item(&PredefinedMenuItem::quit(app, Some(labels.quit.as_str()))?)
        .build()?;
    let connection = SubmenuBuilder::new(app, labels.connection_menu.as_str())
        .item(&new_connection)
        .item(&import_connections)
        .separator()
        .item(&new_session)
        .item(&close_tab)
        .build()?;
    let edit = SubmenuBuilder::new(app, labels.edit_menu.as_str())
        .item(&PredefinedMenuItem::undo(app, Some(labels.undo.as_str()))?)
        .item(&PredefinedMenuItem::redo(app, Some(labels.redo.as_str()))?)
        .separator()
        .item(&PredefinedMenuItem::cut(app, Some(labels.cut.as_str()))?)
        .item(&PredefinedMenuItem::copy(app, Some(labels.copy.as_str()))?)
        .item(&PredefinedMenuItem::paste(
            app,
            Some(labels.paste.as_str()),
        )?)
        .item(&PredefinedMenuItem::select_all(
            app,
            Some(labels.select_all.as_str()),
        )?)
        .build()?;
    let view = SubmenuBuilder::new(app, labels.view_menu.as_str())
        .item(&show_terminal)
        .item(&show_files)
        .separator()
        .item(&toggle_sidebar)
        .separator()
        .item(&PredefinedMenuItem::fullscreen(
            app,
            Some(labels.fullscreen.as_str()),
        )?)
        .build()?;
    let window = SubmenuBuilder::new(app, labels.window_menu.as_str())
        .item(&PredefinedMenuItem::minimize(
            app,
            Some(labels.minimize.as_str()),
        )?)
        .item(&PredefinedMenuItem::maximize(
            app,
            Some(labels.zoom.as_str()),
        )?)
        .separator()
        .item(&PredefinedMenuItem::bring_all_to_front(
            app,
            Some(labels.bring_all_to_front.as_str()),
        )?)
        .build()?;
    let help_menu = SubmenuBuilder::new(app, labels.help_menu.as_str())
        .item(&help)
        .build()?;
    MenuBuilder::new(app)
        .items(&[&application, &connection, &edit, &view, &window, &help_menu])
        .build()
}

pub fn handle_menu(app: &AppHandle, id: &str) {
    let command = match id {
        "new-connection" => MenuCommand::NewConnection,
        "import-connections" => MenuCommand::ImportConnections,
        "new-session" => MenuCommand::NewSession,
        "close-tab" => MenuCommand::CloseTab,
        "settings" => MenuCommand::Settings,
        "help" => MenuCommand::Help,
        "show-terminal" => MenuCommand::ShowTerminal,
        "show-files" => MenuCommand::ShowFiles,
        "toggle-sidebar" => MenuCommand::ToggleSidebar,
        _ => return,
    };
    let _ = app.emit("app:event", AppEvent::MenuCommand(command));
}
