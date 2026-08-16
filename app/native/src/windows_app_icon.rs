use std::{
    env, fs,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

use ico::{IconDir, IconDirEntry, IconImage, ResourceType};
use tauri::{AppHandle, Manager};
use walkdir::WalkDir;
use windows::{
    Win32::{
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize, IPersistFile, STGM_READ, STGM_READWRITE,
        },
        UI::Shell::{
            IShellLinkW, SHCNE_ASSOCCHANGED, SHCNE_UPDATEITEM, SHCNF_IDLIST, SHCNF_PATHW,
            SHChangeNotify, SLGP_RAWPATH, ShellLink,
        },
    },
    core::{Interface, PCWSTR},
};

use crate::{app_icon, models::AppIconId};

pub async fn update_shortcuts(app: &AppHandle, icon: &AppIconId) -> Result<PathBuf, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let current_exe = env::current_exe().map_err(|error| error.to_string())?;
    let icon = icon.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let icon_path = write_icon_file(&data_dir, &icon)?;
        if !cfg!(debug_assertions) {
            update_matching_shortcuts(&current_exe, &icon_path)?;
        }
        Ok(icon_path)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn write_icon_file(data_dir: &Path, icon: &AppIconId) -> Result<PathBuf, String> {
    let icon_dir = data_dir.join("icons");
    fs::create_dir_all(&icon_dir).map_err(|error| error.to_string())?;
    let name = match icon {
        AppIconId::Luna => "luna",
        AppIconId::Graphite => "graphite",
        AppIconId::Signal => "signal",
        AppIconId::Light => "light",
    };
    let path = icon_dir.join(format!("{name}.ico"));
    let source =
        image::load_from_memory(app_icon::bytes(icon)).map_err(|error| error.to_string())?;
    let mut output = IconDir::new(ResourceType::Icon);
    for size in [16, 20, 24, 32, 40, 48, 64, 128, 256] {
        let rgba = source
            .resize_exact(size, size, image::imageops::FilterType::Lanczos3)
            .to_rgba8()
            .into_raw();
        let image = IconImage::from_rgba_data(size, size, rgba);
        output.add_entry(IconDirEntry::encode(&image).map_err(|error| error.to_string())?);
    }
    let file = fs::File::create(&path).map_err(|error| error.to_string())?;
    output.write(file).map_err(|error| error.to_string())?;
    Ok(path)
}

fn update_matching_shortcuts(current_exe: &Path, icon_path: &Path) -> Result<(), String> {
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .map_err(|error| error.to_string())?;
    let result = update_matching_shortcuts_initialized(current_exe, icon_path);
    unsafe { CoUninitialize() };
    result
}

fn update_matching_shortcuts_initialized(
    current_exe: &Path,
    icon_path: &Path,
) -> Result<(), String> {
    let mut roots = Vec::new();
    if let Some(desktop) = dirs::desktop_dir() {
        roots.push(desktop);
    }
    if let Some(app_data) = env::var_os("APPDATA") {
        let app_data = PathBuf::from(app_data);
        roots.push(app_data.join(r"Microsoft\Windows\Start Menu\Programs"));
        roots.push(app_data.join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar"));
    }
    if let Some(program_data) = env::var_os("PROGRAMDATA") {
        roots.push(PathBuf::from(program_data).join(r"Microsoft\Windows\Start Menu\Programs"));
    }

    for shortcut in roots
        .into_iter()
        .filter(|root| root.exists())
        .flat_map(|root| WalkDir::new(root).into_iter().filter_map(Result::ok))
        .map(|entry| entry.into_path())
        .filter(|path| {
            path.extension()
                .is_some_and(|value| value.eq_ignore_ascii_case("lnk"))
                && path
                    .file_stem()
                    .is_some_and(|value| value.eq_ignore_ascii_case(crate::product::DISPLAY_NAME))
        })
    {
        if let Err(error) = update_shortcut(&shortcut, current_exe, icon_path) {
            eprintln!("无法更新快捷方式 {}: {error}", shortcut.display());
        }
    }
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    Ok(())
}

fn update_shortcut(
    shortcut_path: &Path,
    current_exe: &Path,
    icon_path: &Path,
) -> Result<(), String> {
    let shortcut_wide = wide(shortcut_path);
    let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
        .map_err(|error| error.to_string())?;
    let persist: IPersistFile = link.cast().map_err(|error| error.to_string())?;
    unsafe { persist.Load(PCWSTR(shortcut_wide.as_ptr()), STGM_READ) }
        .map_err(|error| error.to_string())?;

    let mut target = vec![0u16; 32_768];
    unsafe { link.GetPath(&mut target, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32) }
        .map_err(|error| error.to_string())?;
    let length = target
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(target.len());
    let target = PathBuf::from(std::ffi::OsString::from_wide(&target[..length]));
    if !same_path(&target, current_exe) {
        return Ok(());
    }

    let icon_wide = wide(icon_path);
    unsafe { persist.Load(PCWSTR(shortcut_wide.as_ptr()), STGM_READWRITE) }
        .map_err(|error| error.to_string())?;
    unsafe {
        link.SetIconLocation(PCWSTR(icon_wide.as_ptr()), 0)
            .and_then(|_| persist.Save(PCWSTR(shortcut_wide.as_ptr()), true))
    }
    .map_err(|error| error.to_string())?;
    unsafe {
        SHChangeNotify(
            SHCNE_UPDATEITEM,
            SHCNF_PATHW,
            Some(shortcut_wide.as_ptr().cast()),
            None,
        )
    };
    Ok(())
}

fn same_path(left: &Path, right: &Path) -> bool {
    let left = fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    left.to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy())
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
