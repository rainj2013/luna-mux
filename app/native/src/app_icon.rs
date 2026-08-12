use tauri::{AppHandle, WebviewWindow};

#[cfg(not(target_os = "macos"))]
use tauri::Manager;

#[cfg(target_os = "windows")]
use std::time::Duration;
#[cfg(target_os = "windows")]
use windows::Win32::{
    Foundation::{LPARAM, WPARAM},
    Storage::EnhancedStorage::PKEY_AppUserModel_RelaunchIconResource,
    UI::WindowsAndMessaging::{
        ICON_BIG, ICON_SMALL, ICON_SMALL2, SendMessageW, WM_GETICON, WM_SETICON,
    },
};
#[cfg(target_os = "windows")]
use windows::Win32::{
    System::Com::StructuredStorage::PROPVARIANT,
    UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow},
};

use crate::models::AppIconId;

pub fn bytes(icon: &AppIconId) -> &'static [u8] {
    match icon {
        AppIconId::Luna => include_bytes!("../../../assets/icons/luna.png"),
        AppIconId::Graphite => include_bytes!("../../../assets/icons/graphite.png"),
        AppIconId::Signal => include_bytes!("../../../assets/icons/signal.png"),
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
        set_window_icon(&window, icon)?;
        #[cfg(target_os = "windows")]
        let app = _app.clone();
        #[cfg(target_os = "windows")]
        let icon = icon.clone();
        #[cfg(target_os = "windows")]
        tauri::async_runtime::spawn(async move {
            let result = async {
                let icon_path = crate::windows_app_icon::update_shortcuts(&app, &icon).await?;
                sync_windows_taskbar_icon(&window, &icon_path).await
            }
            .await;
            if let Err(error) = result {
                eprintln!("无法恢复 Windows 任务栏图标: {error}");
            }
        });
        Ok(())
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
        set_window_icon(_window, icon)?;
        #[cfg(target_os = "windows")]
        {
            let icon_path = crate::windows_app_icon::update_shortcuts(_app, icon).await?;
            sync_windows_taskbar_icon(_window, &icon_path).await?;
        }
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
fn set_window_icon(window: &WebviewWindow, icon: &AppIconId) -> Result<(), String> {
    let image = tauri::image::Image::from_bytes(bytes(icon)).map_err(|e| e.to_string())?;
    window.set_icon(image).map_err(|e| e.to_string())
}

#[cfg(target_os = "windows")]
async fn sync_windows_taskbar_icon(
    window: &WebviewWindow,
    icon_path: &std::path::Path,
) -> Result<(), String> {
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(25)).await;
        let hwnd = window.hwnd().map_err(|error| error.to_string())?;
        let small_icon =
            unsafe { SendMessageW(hwnd, WM_GETICON, Some(WPARAM(ICON_SMALL as usize)), None) };
        if small_icon.0 != 0 {
            unsafe {
                let _ = SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(ICON_BIG as usize)),
                    Some(LPARAM(small_icon.0)),
                );
                let _ = SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(ICON_SMALL2 as usize)),
                    Some(LPARAM(small_icon.0)),
                );
            }
            let icon_resource = format!("{},0", icon_path.display());
            let value = PROPVARIANT::from(icon_resource.as_str());
            let properties: IPropertyStore =
                unsafe { SHGetPropertyStoreForWindow(hwnd) }.map_err(|error| error.to_string())?;
            unsafe {
                properties
                    .SetValue(&PKEY_AppUserModel_RelaunchIconResource, &value)
                    .and_then(|_| properties.Commit())
            }
            .map_err(|error| error.to_string())?;
            return Ok(());
        }
    }
    Err("Windows 未返回已设置的窗口图标".to_string())
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
            AppIconId::Luna,
            AppIconId::Graphite,
            AppIconId::Signal,
            AppIconId::Light,
        ] {
            let image = image::load_from_memory(bytes(&icon)).expect("valid embedded app icon");
            assert_eq!(image.width(), 512);
            assert_eq!(image.height(), 512);
            assert_eq!(image.to_rgba8().get_pixel(0, 0).0[3], 0);
        }
    }

    #[test]
    fn icon_ids_read_legacy_values_but_serialize_with_luna_mux_names() {
        let legacy = [
            ("\"ssh\"", AppIconId::Luna),
            ("\"classic\"", AppIconId::Graphite),
            ("\"neon\"", AppIconId::Signal),
        ];
        for (json, expected) in legacy {
            let parsed: AppIconId = serde_json::from_str(json).expect("legacy app icon id");
            assert_eq!(
                std::mem::discriminant(&parsed),
                std::mem::discriminant(&expected)
            );
        }
        assert_eq!(serde_json::to_string(&AppIconId::Luna).unwrap(), "\"luna\"");
        assert_eq!(
            serde_json::to_string(&AppIconId::Graphite).unwrap(),
            "\"graphite\""
        );
        assert_eq!(
            serde_json::to_string(&AppIconId::Signal).unwrap(),
            "\"signal\""
        );
    }
}
