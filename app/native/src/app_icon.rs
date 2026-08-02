use std::path::PathBuf;

use tauri::{AppHandle, Manager, WebviewWindow};

use crate::models::AppIconId;

pub fn path(app: &AppHandle, icon: &AppIconId) -> PathBuf {
    let name = format!("{}.png", icon.as_str());
    let packaged = app
        .path()
        .resource_dir()
        .ok()
        .map(|path| path.join("icons").join(&name));
    packaged.filter(|path| path.exists()).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/icons")
            .join(name)
    })
}

pub fn apply_at_startup(app: &AppHandle, icon: &AppIconId) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let bytes = std::fs::read(path(app, icon)).map_err(|error| error.to_string())?;
        set_macos_dock_icon(&bytes)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let window = app
            .get_webview_window("main")
            .ok_or_else(|| "找不到主窗口，无法恢复应用图标".to_string())?;
        set_window_icon(app, &window, icon)
    }
}

pub async fn apply(
    app: &AppHandle,
    _window: &WebviewWindow,
    icon: &AppIconId,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let bytes = std::fs::read(path(app, icon)).map_err(|error| error.to_string())?;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        app.run_on_main_thread(move || {
            let _ = sender.send(set_macos_dock_icon(&bytes));
        })
        .map_err(|error| error.to_string())?;
        receiver
            .await
            .map_err(|_| "设置 Dock 图标的主线程任务未完成".to_string())?
    }

    #[cfg(not(target_os = "macos"))]
    {
        set_window_icon(app, _window, icon)
    }
}

#[cfg(not(target_os = "macos"))]
fn set_window_icon(
    app: &AppHandle,
    window: &WebviewWindow,
    icon: &AppIconId,
) -> Result<(), String> {
    let image = tauri::image::Image::from_path(path(app, icon)).map_err(|e| e.to_string())?;
    window.set_icon(image).map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
fn set_macos_dock_icon(bytes: &[u8]) -> Result<(), String> {
    use objc2::{AllocAnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let main_thread =
        MainThreadMarker::new().ok_or_else(|| "必须在 macOS 主线程设置 Dock 图标".to_string())?;
    let data = NSData::with_bytes(bytes);
    let image = NSImage::initWithData(NSImage::alloc(), &data)
        .ok_or_else(|| "无法读取应用图标".to_string())?;
    let application = NSApplication::sharedApplication(main_thread);
    unsafe { application.setApplicationIconImage(Some(&image)) };
    Ok(())
}
