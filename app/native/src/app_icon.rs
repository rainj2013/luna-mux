use tauri::{AppHandle, WebviewWindow};

#[cfg(not(target_os = "macos"))]
use tauri::Manager;

use crate::models::AppIconId;

pub fn bytes(icon: &AppIconId) -> &'static [u8] {
    match icon {
        AppIconId::Ssh => include_bytes!("../../../assets/icons/ssh.png"),
        AppIconId::Classic => include_bytes!("../../../assets/icons/classic.png"),
        AppIconId::Neon => include_bytes!("../../../assets/icons/neon.png"),
        AppIconId::Light => include_bytes!("../../../assets/icons/light.png"),
    }
}

pub fn apply_at_startup(_app: &AppHandle, icon: &AppIconId) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        set_macos_dock_icon(bytes(icon))
    }

    #[cfg(not(target_os = "macos"))]
    {
        let window = _app
            .get_webview_window("main")
            .ok_or_else(|| "找不到主窗口，无法恢复应用图标".to_string())?;
        set_window_icon(&window, icon)
    }
}

pub async fn apply(
    _app: &AppHandle,
    _window: &WebviewWindow,
    icon: &AppIconId,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let bytes = bytes(icon);
        _app.run_on_main_thread(move || {
            let _ = sender.send(set_macos_dock_icon(bytes));
        })
        .map_err(|error| error.to_string())?;
        receiver
            .await
            .map_err(|_| "设置 Dock 图标的主线程任务未完成".to_string())?
    }

    #[cfg(not(target_os = "macos"))]
    {
        set_window_icon(_window, icon)
    }
}

#[cfg(not(target_os = "macos"))]
fn set_window_icon(window: &WebviewWindow, icon: &AppIconId) -> Result<(), String> {
    let image = tauri::image::Image::from_bytes(bytes(icon)).map_err(|e| e.to_string())?;
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

#[cfg(test)]
mod tests {
    use super::bytes;
    use crate::models::AppIconId;

    #[test]
    fn embedded_icons_are_valid_images() {
        for icon in [
            AppIconId::Ssh,
            AppIconId::Classic,
            AppIconId::Neon,
            AppIconId::Light,
        ] {
            image::load_from_memory(bytes(&icon)).expect("valid embedded app icon");
        }
    }
}
