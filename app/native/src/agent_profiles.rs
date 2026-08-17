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
    let checked_command = command.clone();
    let probe = tokio::task::spawn_blocking(move || {
        crate::agent_command::discover(&[&checked_command], &checked_target_id)
    });
    let result = probe.await.map_err(|error| error.to_string())?;
    let path = result.paths.get(&command);
    let available = path.is_some();
    let detail = path
        .map(|path| path.to_string_lossy().into_owned())
        .or(result.warning)
        .unwrap_or_else(|| format!("未在目标终端环境中找到 {command}"));
    Ok(AgentProfileAvailability {
        profile_id: profile.id,
        target_id,
        available,
        detail,
    })
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
