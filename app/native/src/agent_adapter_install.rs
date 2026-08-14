use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::product;

const CODEX_HOOK_EVENTS: [&str; 9] = [
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "SubagentStart",
    "SubagentStop",
    "Stop",
];
const OWNER_ARGUMENT: &str = "--adapter-owner";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AgentAdapterInstallAction {
    Install,
    Uninstall,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentAdapterConfigChange {
    pub event: String,
    pub operation: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentAdapterInstallStatus {
    pub installed: bool,
    pub current: bool,
    pub hooks_path: String,
    pub config_path: String,
    pub inline_hooks_detected: bool,
    pub hooks_disabled: bool,
    pub trust_review_required: bool,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentAdapterInstallPreview {
    pub action: AgentAdapterInstallAction,
    pub status: AgentAdapterInstallStatus,
    pub changed: bool,
    pub backup_will_be_created: bool,
    pub before_contents: String,
    pub after_contents: String,
    pub changes: Vec<AgentAdapterConfigChange>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentAdapterInstallResult {
    pub status: AgentAdapterInstallStatus,
    pub backup_path: Option<String>,
}

pub fn status() -> Result<AgentAdapterInstallStatus, String> {
    status_in(
        &codex_home()?,
        &std::env::current_exe().map_err(|error| error.to_string())?,
    )
}

pub fn preview(action: AgentAdapterInstallAction) -> Result<AgentAdapterInstallPreview, String> {
    preview_in(
        &codex_home()?,
        &std::env::current_exe().map_err(|error| error.to_string())?,
        action,
    )
}

pub fn apply(action: AgentAdapterInstallAction) -> Result<AgentAdapterInstallResult, String> {
    let home = codex_home()?;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    apply_in(&home, &executable, action)
}

pub fn persistent_hook_is_active() -> bool {
    status().is_ok_and(|value| value.current && !value.hooks_disabled)
}

fn codex_home() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("CODEX_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    dirs::home_dir()
        .map(|path| path.join(".codex"))
        .ok_or_else(|| "无法确定 Codex 配置目录".to_string())
}

fn status_in(home: &Path, executable: &Path) -> Result<AgentAdapterInstallStatus, String> {
    let hooks_path = home.join("hooks.json");
    let config_path = home.join("config.toml");
    let root = read_hooks(&hooks_path)?;
    let expected = hook_command(executable);
    let owned = owned_commands(&root);
    let installed = !owned.is_empty();
    let current = CODEX_HOOK_EVENTS.iter().all(|event| {
        owned
            .iter()
            .filter(|(owned_event, _)| owned_event == event)
            .count()
            == 1
            && owned
                .iter()
                .any(|(owned_event, command)| owned_event == event && command == &expected)
    }) && owned.len() == CODEX_HOOK_EVENTS.len()
        && owned_handlers_are_current(&root, &expected);
    let (inline_hooks_detected, hooks_disabled, warnings) = inspect_config(&config_path);
    Ok(AgentAdapterInstallStatus {
        installed,
        current,
        hooks_path: hooks_path.to_string_lossy().into_owned(),
        config_path: config_path.to_string_lossy().into_owned(),
        inline_hooks_detected,
        hooks_disabled,
        trust_review_required: installed,
        warnings,
    })
}

fn preview_in(
    home: &Path,
    executable: &Path,
    action: AgentAdapterInstallAction,
) -> Result<AgentAdapterInstallPreview, String> {
    let hooks_path = home.join("hooks.json");
    let existed = hooks_path.exists();
    let root = read_hooks(&hooks_path)?;
    let before_contents = if existed {
        fs::read_to_string(&hooks_path).map_err(|error| error.to_string())?
    } else {
        String::new()
    };
    let (after, changes) = transform_hooks(root, executable, &action)?;
    let after_contents = serialize_hooks(&after)?;
    let normalized_before = if before_contents.trim().is_empty() {
        serialize_hooks(&json!({}))?
    } else {
        serialize_hooks(&read_hooks(&hooks_path)?)?
    };
    let changed = normalized_before != after_contents;
    Ok(AgentAdapterInstallPreview {
        action,
        status: status_in(home, executable)?,
        changed,
        backup_will_be_created: changed && existed,
        before_contents,
        after_contents,
        changes,
    })
}

fn apply_in(
    home: &Path,
    executable: &Path,
    action: AgentAdapterInstallAction,
) -> Result<AgentAdapterInstallResult, String> {
    let preview = preview_in(home, executable, action)?;
    if !preview.changed {
        return Ok(AgentAdapterInstallResult {
            status: status_in(home, executable)?,
            backup_path: None,
        });
    }
    fs::create_dir_all(home).map_err(|error| error.to_string())?;
    let hooks_path = home.join("hooks.json");
    let backup_path = write_recoverable(&hooks_path, preview.after_contents.as_bytes())?;
    Ok(AgentAdapterInstallResult {
        status: status_in(home, executable)?,
        backup_path: backup_path.map(|path| path.to_string_lossy().into_owned()),
    })
}

fn read_hooks(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let contents = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let value: Value = serde_json::from_str(&contents)
        .map_err(|error| format!("{} 不是有效的 Codex hooks.json: {error}", path.display()))?;
    if !value.is_object() {
        return Err(format!("{} 的顶层必须是 JSON 对象", path.display()));
    }
    if value.get("hooks").is_some_and(|hooks| !hooks.is_object()) {
        return Err(format!("{} 中的 hooks 必须是 JSON 对象", path.display()));
    }
    Ok(value)
}

fn transform_hooks(
    mut root: Value,
    executable: &Path,
    action: &AgentAdapterInstallAction,
) -> Result<(Value, Vec<AgentAdapterConfigChange>), String> {
    let root_object = root
        .as_object_mut()
        .ok_or_else(|| "Codex hooks.json 顶层必须是对象".to_string())?;
    let hooks = root_object
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| "Codex hooks.json 中的 hooks 必须是对象".to_string())?;
    let mut changes = Vec::new();

    for (event, groups) in hooks.iter_mut() {
        let Some(groups) = groups.as_array_mut() else {
            return Err(format!("Codex hooks.json 中 hooks.{event} 必须是数组"));
        };
        let before = count_owned_in_groups(groups);
        remove_owned_from_groups(groups);
        if before > 0 {
            changes.push(AgentAdapterConfigChange {
                event: event.clone(),
                operation: "remove".into(),
            });
        }
    }
    hooks.retain(|_, value| !value.as_array().is_some_and(Vec::is_empty));

    if action == &AgentAdapterInstallAction::Install {
        let command = hook_command(executable);
        for event in CODEX_HOOK_EVENTS {
            let handler = if cfg!(windows) {
                json!({
                    "type": "command",
                    "command": command,
                    "commandWindows": format!("& {command}"),
                    "timeout": 3
                })
            } else {
                json!({
                    "type": "command",
                    "command": command,
                    "timeout": 3
                })
            };
            hooks
                .entry(event)
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .ok_or_else(|| format!("Codex hooks.json 中 hooks.{event} 必须是数组"))?
                .push(json!({
                    "hooks": [handler]
                }));
            changes.push(AgentAdapterConfigChange {
                event: event.into(),
                operation: "add".into(),
            });
        }
    }
    if hooks.is_empty() {
        root_object.remove("hooks");
    }
    Ok((root, changes))
}

fn owned_commands(root: &Value) -> Vec<(String, String)> {
    let Some(hooks) = root.get("hooks").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for (event, groups) in hooks {
        let Some(groups) = groups.as_array() else {
            continue;
        };
        for group in groups {
            let Some(handlers) = group.get("hooks").and_then(Value::as_array) else {
                continue;
            };
            for handler in handlers {
                if let Some(command) = handler.get("command").and_then(Value::as_str)
                    && is_owned_command(command)
                {
                    result.push((event.clone(), command.to_string()));
                }
            }
        }
    }
    result
}

fn owned_handlers_are_current(root: &Value, expected_command: &str) -> bool {
    let Some(hooks) = root.get("hooks").and_then(Value::as_object) else {
        return false;
    };
    hooks.values().all(|groups| {
        groups
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|group| {
                group
                    .get("hooks")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .filter(|handler| {
                handler
                    .get("command")
                    .and_then(Value::as_str)
                    .is_some_and(is_owned_command)
            })
            .all(|handler| {
                handler.get("command").and_then(Value::as_str) == Some(expected_command)
                    && (!cfg!(windows)
                        || handler.get("commandWindows").and_then(Value::as_str)
                            == Some(format!("& {expected_command}").as_str()))
            })
    })
}

fn count_owned_in_groups(groups: &[Value]) -> usize {
    groups
        .iter()
        .flat_map(|group| {
            group
                .get("hooks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter(|handler| {
            handler
                .get("command")
                .and_then(Value::as_str)
                .is_some_and(is_owned_command)
        })
        .count()
}

fn remove_owned_from_groups(groups: &mut Vec<Value>) {
    for group in groups.iter_mut() {
        if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
            handlers.retain(|handler| {
                !handler
                    .get("command")
                    .and_then(Value::as_str)
                    .is_some_and(is_owned_command)
            });
        }
    }
    groups.retain(|group| {
        !group
            .get("hooks")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
    });
}

fn hook_command(executable: &Path) -> String {
    format!(
        "\"{}\" hook {OWNER_ARGUMENT} {}",
        executable.to_string_lossy().replace('"', "\\\""),
        product::PRODUCT_KEY
    )
}

fn is_owned_command(command: &str) -> bool {
    let marker = format!("{OWNER_ARGUMENT} {}", product::PRODUCT_KEY);
    command
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .any(|pair| {
            pair[0] == OWNER_ARGUMENT && pair[1].trim_matches(['\'', '"']) == product::PRODUCT_KEY
        })
        || command.contains(&marker)
}

fn inspect_config(path: &Path) -> (bool, bool, Vec<String>) {
    if !path.exists() {
        return (false, false, Vec::new());
    }
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) => return (false, false, vec![format!("configReadFailed:{error}")]),
    };
    let value: toml::Value = match toml::from_str(&contents) {
        Ok(value) => value,
        Err(error) => return (false, false, vec![format!("configParseFailed:{error}")]),
    };
    let inline = value.get("hooks").is_some();
    let disabled = value
        .get("features")
        .and_then(|value| value.get("hooks"))
        .and_then(toml::Value::as_bool)
        == Some(false)
        || value
            .get("features")
            .and_then(|value| value.get("codex_hooks"))
            .and_then(toml::Value::as_bool)
            == Some(false);
    let mut warnings = Vec::new();
    if inline {
        warnings.push("inlineHooksAlsoDetected".into());
    }
    if disabled {
        warnings.push("hooksDisabled".into());
    }
    (inline, disabled, warnings)
}

fn serialize_hooks(value: &Value) -> Result<String, String> {
    serde_json::to_string_pretty(value)
        .map(|value| format!("{value}\n"))
        .map_err(|error| error.to_string())
}

fn write_recoverable(target: &Path, contents: &[u8]) -> Result<Option<PathBuf>, String> {
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

    let backup = if target.exists() {
        let backup = directory.join(format!(
            "hooks.json.luna-mux-backup-{}-{}",
            Utc::now().format("%Y%m%dT%H%M%SZ"),
            Uuid::new_v4().simple()
        ));
        fs::rename(target, &backup).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            error.to_string()
        })?;
        Some(backup)
    } else {
        None
    };
    if let Err(error) = fs::rename(&temporary, target) {
        if let Some(backup) = &backup {
            let _ = fs::rename(backup, target);
        }
        let _ = fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_home() -> PathBuf {
        let path = std::env::temp_dir().join(format!("luna-mux-adapter-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn install_merges_unrelated_hooks_and_uninstall_removes_only_owned_handlers() {
        let home = test_home();
        let hooks_path = home.join("hooks.json");
        fs::write(
            &hooks_path,
            r#"{
  "description": "keep me",
  "hooks": {
    "Stop": [{"hooks": [{"type": "command", "command": "other-tool stop"}]}]
  }
}"#,
        )
        .unwrap();
        let executable = Path::new(r#"C:\Program Files\Luna Mux\luna-mux.exe"#);
        let installed = apply_in(&home, executable, AgentAdapterInstallAction::Install).unwrap();
        assert!(installed.status.current);
        assert!(installed.backup_path.is_some());
        let merged = read_hooks(&hooks_path).unwrap();
        assert_eq!(merged["description"], "keep me");
        assert!(merged.to_string().contains("other-tool stop"));
        assert_eq!(owned_commands(&merged).len(), CODEX_HOOK_EVENTS.len());
        if cfg!(windows) {
            assert!(merged.to_string().contains("commandWindows"));
            assert!(merged.to_string().contains("& \\\"C:\\\\Program Files"));
        }

        let removed = apply_in(&home, executable, AgentAdapterInstallAction::Uninstall).unwrap();
        assert!(!removed.status.installed);
        let remaining = read_hooks(&hooks_path).unwrap();
        assert_eq!(remaining["description"], "keep me");
        assert!(remaining.to_string().contains("other-tool stop"));
    }

    #[test]
    fn stale_owned_entries_are_replaced_and_install_is_idempotent() {
        let home = test_home();
        fs::write(
            home.join("hooks.json"),
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"\"old.exe\" hook --adapter-owner luna-mux"}]}]}}"#,
        )
        .unwrap();
        let executable = Path::new("new-luna-mux");
        let first = preview_in(&home, executable, AgentAdapterInstallAction::Install).unwrap();
        assert!(first.changed);
        apply_in(&home, executable, AgentAdapterInstallAction::Install).unwrap();
        let second = preview_in(&home, executable, AgentAdapterInstallAction::Install).unwrap();
        assert!(!second.changed);
        assert!(second.status.current);
    }

    #[test]
    fn config_inspection_reports_inline_and_disabled_hooks_without_mutating_toml() {
        let home = test_home();
        let config = "[features]\nhooks = false\n[hooks]\n";
        fs::write(home.join("config.toml"), config).unwrap();
        let status = status_in(&home, Path::new("luna-mux")).unwrap();
        assert!(status.inline_hooks_detected);
        assert!(status.hooks_disabled);
        assert_eq!(
            fs::read_to_string(home.join("config.toml")).unwrap(),
            config
        );
    }
}
