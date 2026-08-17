use std::path::Path;

const IMAGE_FILE_EXTENSIONS: &[&str] = &[
    "avif", "bmp", "gif", "ico", "jpeg", "jpg", "png", "tif", "tiff", "webp",
];

fn is_image_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            IMAGE_FILE_EXTENSIONS
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

#[cfg(target_os = "windows")]
fn clipboard_has_image_file() -> bool {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt};
    use windows::Win32::{
        System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard},
        UI::Shell::{DragQueryFileW, HDROP},
    };

    const CF_HDROP: u32 = 15;

    struct ClipboardGuard;
    impl Drop for ClipboardGuard {
        fn drop(&mut self) {
            // SAFETY: this guard is created only after OpenClipboard succeeds.
            let _ = unsafe { CloseClipboard() };
        }
    }

    // SAFETY: the clipboard is kept open while the borrowed HDROP is queried.
    unsafe {
        if OpenClipboard(None).is_err() {
            return false;
        }
        let _guard = ClipboardGuard;
        let Ok(handle) = GetClipboardData(CF_HDROP) else {
            return false;
        };
        let drop = HDROP(handle.0);
        let count = DragQueryFileW(drop, u32::MAX, None);
        (0..count).any(|index| {
            let length = DragQueryFileW(drop, index, None);
            if length == 0 {
                return false;
            }
            let mut buffer = vec![0_u16; length as usize + 1];
            let written = DragQueryFileW(drop, index, Some(&mut buffer));
            let path = OsString::from_wide(&buffer[..written as usize]);
            is_image_file(Path::new(&path))
        })
    }
}

#[cfg(not(target_os = "windows"))]
fn clipboard_has_image_file() -> bool {
    false
}

#[tauri::command]
pub fn system_clipboard_has_image_file() -> bool {
    clipboard_has_image_file()
}

#[cfg(test)]
mod tests {
    use super::is_image_file;
    use std::path::Path;

    #[test]
    fn recognizes_image_extensions_case_insensitively() {
        assert!(is_image_file(Path::new(r"C:\screenshots\capture.PNG")));
        assert!(is_image_file(Path::new("/tmp/photo.jpeg")));
        assert!(is_image_file(Path::new("image.webp")));
    }

    #[test]
    fn rejects_non_image_paths() {
        assert!(!is_image_file(Path::new(r"C:\notes\readme.txt")));
        assert!(!is_image_file(Path::new("no-extension")));
    }
}
