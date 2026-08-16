use serde::Serialize;

pub const DEFAULT_CODEX_PROFILE_ID: &str = "codex.default";
pub const DEFAULT_CLAUDE_CODE_PROFILE_ID: &str = "claude-code.default";

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentLaunchProfile {
    pub id: String,
    pub label: String,
    pub adapter: String,
    pub command: String,
    pub built_in: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfileAvailability {
    pub profile_id: String,
    pub target_id: String,
    pub available: bool,
    pub detail: String,
}

pub fn profiles() -> Vec<AgentLaunchProfile> {
    crate::agent_adapters::profiles()
}

pub fn resolve(profile_id: &str) -> Result<AgentLaunchProfile, String> {
    profiles()
        .into_iter()
        .find(|profile| profile.id == profile_id)
        .ok_or_else(|| format!("Agent Launch Profile 不存在: {profile_id}"))
}

pub async fn availability(
    profile_id: &str,
    target_id: &str,
) -> Result<AgentProfileAvailability, String> {
    let profile = resolve(profile_id)?;
    if target_id.starts_with("ssh-bookmark:") {
        return Ok(AgentProfileAvailability {
            profile_id: profile.id,
            target_id: target_id.into(),
            available: true,
            detail: "将在 SSH Runtime 就绪后验证远端命令".into(),
        });
    }
    let target_id = target_id.to_string();
    let checked_target_id = target_id.clone();
    let command = profile.command.clone();
    let probe = tokio::task::spawn_blocking(move || probe_command(&command, &checked_target_id));
    let result = tokio::time::timeout(std::time::Duration::from_secs(4), probe)
        .await
        .map_err(|_| "Agent 可用性检查超时".to_string())?
        .map_err(|error| error.to_string())?;
    let (available, detail) = result?;
    Ok(AgentProfileAvailability {
        profile_id: profile.id,
        target_id,
        available,
        detail,
    })
}

fn probe_command(command: &str, target_id: &str) -> Result<(bool, String), String> {
    #[cfg(windows)]
    let output = if let Some(distribution) = target_id.strip_prefix("local:wsl:") {
        std::process::Command::new("wsl.exe")
            .args([
                "--distribution",
                distribution,
                "--",
                "sh",
                "-lc",
                &format!("command -v {}", posix_quote(command)),
            ])
            .output()
    } else if target_id == "local:powershell" {
        std::process::Command::new(
            crate::local_pty_backend::windows_powershell7_executable()
                .ok_or_else(|| "未找到 PowerShell 7（pwsh.exe）".to_string())?,
        )
            .args([
                "-NoLogo",
                "-Command",
                &format!(
                    "$value = Get-Command -ErrorAction SilentlyContinue {}; if ($value) {{ $value.Source }} else {{ exit 127 }}",
                    powershell_quote(command)
                ),
            ])
            .output()
    } else {
        return Err("Agent Profile 当前只支持本地终端目标".into());
    };

    #[cfg(target_os = "macos")]
    let output = if target_id == "local:macos-shell" {
        std::process::Command::new(
            crate::local_pty_backend::macos_supported_shell()
                .ok_or_else(|| "未找到受支持的 macOS Shell（zsh 或 bash）".to_string())?,
        )
        .args(["-lc", &format!("command -v {}", posix_quote(command))])
        .output()
    } else {
        return Err("Agent Profile 当前只支持本地终端目标".into());
    };

    #[cfg(not(any(windows, target_os = "macos")))]
    let output: std::io::Result<std::process::Output> = {
        let _ = (command, target_id);
        return Err("当前平台尚未支持 Agent Profile 可用性检查".into());
    };

    let output = output.map_err(|error| error.to_string())?;
    let detail = String::from_utf8_lossy(if output.status.success() {
        &output.stdout
    } else {
        &output.stderr
    })
    .trim()
    .to_string();
    Ok((output.status.success(), detail))
}

fn posix_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(windows)]
fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_profile_ids_are_unique_and_resolvable() {
        let profiles = profiles();
        let ids = profiles
            .iter()
            .map(|profile| profile.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(ids.len(), profiles.len());
        assert_eq!(resolve(DEFAULT_CODEX_PROFILE_ID).unwrap().adapter, "codex");
        assert_eq!(
            resolve(DEFAULT_CLAUDE_CODE_PROFILE_ID).unwrap().adapter,
            "claude-code"
        );
        assert!(resolve("unknown").is_err());
    }

    #[tokio::test]
    #[cfg(windows)]
    async fn codex_profile_is_detected_in_the_powershell_target() {
        if !std::process::Command::new("where.exe")
            .arg("codex.cmd")
            .status()
            .is_ok_and(|status| status.success())
        {
            return;
        }
        let result = availability(DEFAULT_CODEX_PROFILE_ID, "local:powershell")
            .await
            .unwrap();
        assert!(result.available, "{}", result.detail);
        assert!(!result.detail.is_empty());
    }
}
