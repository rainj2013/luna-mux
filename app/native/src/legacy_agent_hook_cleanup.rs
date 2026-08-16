use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use chrono::Utc;
use serde_json::Value;
use uuid::Uuid;

use crate::product;

const OWNER_ARGUMENT: &str = "--adapter-owner";

pub fn remove_legacy_persistent_hooks() -> Result<Option<PathBuf>, String> {
    let Some(home) = codex_home() else {
        return Ok(None);
    };
    let hooks_path = home.join("hooks.json");
    if !hooks_path.exists() {
        return Ok(None);
    }

    let contents = fs::read_to_string(&hooks_path).map_err(|error| error.to_string())?;
    let mut root: Value = serde_json::from_str(&contents).map_err(|error| {
        format!(
            "{} 不是有效的 Codex hooks.json: {error}",
            hooks_path.display()
        )
    })?;
    if !remove_owned_hooks(&mut root)? {
        return Ok(None);
    }

    let updated = serde_json::to_string_pretty(&root)
        .map(|value| format!("{value}\n"))
        .map_err(|error| error.to_string())?;
    write_recoverable(&hooks_path, updated.as_bytes()).map(Some)
}

fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|path| path.join(".codex")))
}

fn remove_owned_hooks(root: &mut Value) -> Result<bool, String> {
    let root = root
        .as_object_mut()
        .ok_or_else(|| "Codex hooks.json 顶层必须是对象".to_string())?;
    let Some(hooks_value) = root.get_mut("hooks") else {
        return Ok(false);
    };
    let hooks = hooks_value
        .as_object_mut()
        .ok_or_else(|| "Codex hooks.json 中的 hooks 必须是对象".to_string())?;
    let mut changed = false;

    for (event, groups) in hooks.iter_mut() {
        let groups = groups
            .as_array_mut()
            .ok_or_else(|| format!("Codex hooks.json 中 hooks.{event} 必须是数组"))?;
        for group in groups.iter_mut() {
            let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                continue;
            };
            let before = handlers.len();
            handlers.retain(|handler| {
                !handler
                    .get("command")
                    .and_then(Value::as_str)
                    .is_some_and(is_owned_command)
            });
            changed |= handlers.len() != before;
        }
        groups.retain(|group| {
            !group
                .get("hooks")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty)
        });
    }
    hooks.retain(|_, groups| !groups.as_array().is_some_and(Vec::is_empty));
    if hooks.is_empty() {
        root.remove("hooks");
    }
    Ok(changed)
}

fn is_owned_command(command: &str) -> bool {
    command
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .any(|pair| {
            pair[0] == OWNER_ARGUMENT && pair[1].trim_matches(['\'', '"']) == product::PRODUCT_KEY
        })
}

fn write_recoverable(target: &Path, contents: &[u8]) -> Result<PathBuf, String> {
    let directory = target
        .parent()
        .ok_or_else(|| "Codex Hook 路径缺少父目录".to_string())?;
    let temporary = directory.join(format!(
        ".hooks.json.luna-mux-{}.tmp",
        Uuid::new_v4().simple()
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    file.write_all(contents)
        .map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    drop(file);

    let backup = directory.join(format!(
        "hooks.json.luna-mux-backup-{}-{}",
        Utc::now().format("%Y%m%dT%H%M%SZ"),
        Uuid::new_v4().simple()
    ));
    fs::rename(target, &backup).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        error.to_string()
    })?;
    if let Err(error) = fs::rename(&temporary, target) {
        let _ = fs::rename(&backup, target);
        let _ = fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn removes_only_luna_mux_owned_handlers() {
        let mut root = json!({
            "custom": true,
            "hooks": {
                "Stop": [{ "hooks": [
                    { "type": "command", "command": "other-tool hook" },
                    { "type": "command", "command": "\"/Applications/Luna Mux\" hook --adapter-owner luna-mux" }
                ] }],
                "SessionStart": [{ "hooks": [
                    { "type": "command", "command": "luna-mux hook --adapter-owner luna-mux" }
                ] }]
            }
        });

        assert!(remove_owned_hooks(&mut root).unwrap());
        assert_eq!(root["custom"], true);
        assert_eq!(
            root["hooks"]["Stop"][0]["hooks"].as_array().unwrap().len(),
            1
        );
        assert!(root["hooks"].get("SessionStart").is_none());
    }

    #[test]
    fn leaves_unrelated_configuration_unchanged() {
        let mut root = json!({
            "hooks": { "Stop": [{ "hooks": [{ "command": "other-tool hook" }] }] }
        });
        let original = root.clone();
        assert!(!remove_owned_hooks(&mut root).unwrap());
        assert_eq!(root, original);
    }
}
