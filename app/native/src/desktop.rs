use tauri::{
    AppHandle, Emitter, Runtime,
    menu::{Menu, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder},
};

use crate::models::{AppEvent, MenuCommand};

#[cfg(target_os = "macos")]
pub fn menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let settings = MenuItemBuilder::with_id("settings", "设置...")
        .accelerator("CmdOrCtrl+,")
        .build(app)?;
    let new_connection = MenuItemBuilder::with_id("new-connection", "新增连接")
        .accelerator("CmdOrCtrl+N")
        .build(app)?;
    let import_connections =
        MenuItemBuilder::with_id("import-connections", "导入 OpenSSH Config...").build(app)?;
    let new_session = MenuItemBuilder::with_id("new-session", "为当前机器新建会话")
        .accelerator("CmdOrCtrl+Shift+T")
        .build(app)?;
    let close_tab = MenuItemBuilder::with_id("close-tab", "关闭当前标签")
        .accelerator("CmdOrCtrl+W")
        .build(app)?;
    let show_terminal = MenuItemBuilder::with_id("show-terminal", "终端")
        .accelerator("CmdOrCtrl+1")
        .build(app)?;
    let show_files = MenuItemBuilder::with_id("show-files", "文件传输")
        .accelerator("CmdOrCtrl+2")
        .build(app)?;
    let toggle_sidebar = MenuItemBuilder::with_id("toggle-sidebar", "展开/折叠连接管理")
        .accelerator("CmdOrCtrl+B")
        .build(app)?;
    let help = MenuItemBuilder::with_id("help", "SSH Client 使用帮助")
        .accelerator("F1")
        .build(app)?;

    let application = SubmenuBuilder::new(app, "SSH Client")
        .item(&PredefinedMenuItem::about(
            app,
            Some("关于 SSH Client"),
            None,
        )?)
        .separator()
        .item(&settings)
        .separator()
        .item(&PredefinedMenuItem::services(app, Some("服务"))?)
        .separator()
        .item(&PredefinedMenuItem::hide(app, Some("隐藏 SSH Client"))?)
        .item(&PredefinedMenuItem::hide_others(app, Some("隐藏其他"))?)
        .item(&PredefinedMenuItem::show_all(app, Some("全部显示"))?)
        .separator()
        .item(&PredefinedMenuItem::quit(app, Some("退出 SSH Client"))?)
        .build()?;
    let connection = SubmenuBuilder::new(app, "连接")
        .item(&new_connection)
        .item(&import_connections)
        .separator()
        .item(&new_session)
        .item(&close_tab)
        .build()?;
    let edit = SubmenuBuilder::new(app, "编辑")
        .item(&PredefinedMenuItem::undo(app, Some("撤销"))?)
        .item(&PredefinedMenuItem::redo(app, Some("重做"))?)
        .separator()
        .item(&PredefinedMenuItem::cut(app, Some("剪切"))?)
        .item(&PredefinedMenuItem::copy(app, Some("复制"))?)
        .item(&PredefinedMenuItem::paste(app, Some("粘贴"))?)
        .item(&PredefinedMenuItem::select_all(app, Some("全选"))?)
        .build()?;
    let view = SubmenuBuilder::new(app, "视图")
        .item(&show_terminal)
        .item(&show_files)
        .separator()
        .item(&toggle_sidebar)
        .separator()
        .item(&PredefinedMenuItem::fullscreen(app, Some("进入/退出全屏"))?)
        .build()?;
    let window = SubmenuBuilder::new(app, "窗口")
        .item(&PredefinedMenuItem::minimize(app, Some("最小化"))?)
        .item(&PredefinedMenuItem::maximize(app, Some("缩放"))?)
        .separator()
        .item(&PredefinedMenuItem::bring_all_to_front(
            app,
            Some("前置全部窗口"),
        )?)
        .build()?;
    let help_menu = SubmenuBuilder::new(app, "帮助").item(&help).build()?;
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
