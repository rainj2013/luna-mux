use std::path::{Path, PathBuf};

use crate::{
    agent_profiles::{
        AgentLaunchProfile, DEFAULT_CLAUDE_CODE_PROFILE_ID, DEFAULT_CODEX_PROFILE_ID,
    },
    terminal_runtime_contract::{TerminalManagedAgentContext, TerminalRuntimeContext},
};

pub const CODEX_ADAPTER_ID: &str = "codex";
pub const CLAUDE_CODE_ADAPTER_ID: &str = "claude-code";

pub struct ManagedAgentLaunch<'a> {
    pub profile: &'a AgentLaunchProfile,
    pub target_id: &'a str,
    pub hook_endpoint: &'a str,
    pub mcp_endpoint: &'a str,
    pub context: &'a TerminalManagedAgentContext,
    pub inject_inline_hooks: bool,
    pub hook_command: Option<&'a str>,
    pub browser_command: Option<&'a str>,
    pub browser_credentials_file: Option<&'a str>,
    pub existing_developer_instructions: Option<&'a str>,
}

trait AgentAdapter: Sync {
    fn id(&self) -> &'static str;
    fn profile(&self) -> AgentLaunchProfile;
    fn automatic_profile_id(&self) -> &'static str;
    fn install_manual_shim(
        &self,
        context: &TerminalRuntimeContext,
        hook_endpoint: Option<&str>,
        mcp_endpoint: Option<&str>,
        resolved_command: Option<&Path>,
    ) -> Result<Option<PathBuf>, String>;
    fn managed_command(&self, launch: &ManagedAgentLaunch<'_>) -> Result<String, String>;
}

struct CodexAdapter;
struct ClaudeCodeAdapter;

impl AgentAdapter for CodexAdapter {
    fn id(&self) -> &'static str {
        CODEX_ADAPTER_ID
    }

    fn profile(&self) -> AgentLaunchProfile {
        AgentLaunchProfile {
            id: DEFAULT_CODEX_PROFILE_ID.into(),
            label: "Codex".into(),
            adapter: self.id().into(),
            command: "codex".into(),
            built_in: true,
        }
    }

    fn automatic_profile_id(&self) -> &'static str {
        "codex.auto"
    }

    fn install_manual_shim(
        &self,
        context: &TerminalRuntimeContext,
        _hook_endpoint: Option<&str>,
        mcp_endpoint: Option<&str>,
        resolved_command: Option<&Path>,
    ) -> Result<Option<PathBuf>, String> {
        crate::codex_shim::install(context, mcp_endpoint, resolved_command)
    }

    fn managed_command(&self, launch: &ManagedAgentLaunch<'_>) -> Result<String, String> {
        crate::codex_shim::managed_command(
            &launch.profile.command,
            launch.target_id,
            launch.inject_inline_hooks,
            launch.hook_command,
            launch.mcp_endpoint,
            launch.browser_command,
            launch.browser_credentials_file,
            &launch.context.mux_session_id,
            launch.existing_developer_instructions,
        )
    }
}

impl AgentAdapter for ClaudeCodeAdapter {
    fn id(&self) -> &'static str {
        CLAUDE_CODE_ADAPTER_ID
    }

    fn profile(&self) -> AgentLaunchProfile {
        AgentLaunchProfile {
            id: DEFAULT_CLAUDE_CODE_PROFILE_ID.into(),
            label: "Claude Code".into(),
            adapter: self.id().into(),
            command: "claude".into(),
            built_in: true,
        }
    }

    fn automatic_profile_id(&self) -> &'static str {
        "claude-code.auto"
    }

    fn install_manual_shim(
        &self,
        context: &TerminalRuntimeContext,
        hook_endpoint: Option<&str>,
        mcp_endpoint: Option<&str>,
        resolved_command: Option<&Path>,
    ) -> Result<Option<PathBuf>, String> {
        crate::claude_code_adapter::install(context, hook_endpoint, mcp_endpoint, resolved_command)
    }

    fn managed_command(&self, launch: &ManagedAgentLaunch<'_>) -> Result<String, String> {
        crate::claude_code_adapter::managed_command(launch)
    }
}

static CODEX_ADAPTER: CodexAdapter = CodexAdapter;
static CLAUDE_CODE_ADAPTER: ClaudeCodeAdapter = ClaudeCodeAdapter;

fn adapters() -> [&'static dyn AgentAdapter; 2] {
    [&CODEX_ADAPTER, &CLAUDE_CODE_ADAPTER]
}

fn resolve(adapter_id: &str) -> Result<&'static dyn AgentAdapter, String> {
    adapters()
        .into_iter()
        .find(|adapter| adapter.id() == adapter_id)
        .ok_or_else(|| format!("不支持的 Agent Adapter: {adapter_id}"))
}

pub fn profiles() -> Vec<AgentLaunchProfile> {
    adapters().into_iter().map(AgentAdapter::profile).collect()
}

pub fn automatic_profile_id(adapter_id: &str) -> String {
    resolve(adapter_id)
        .map(|adapter| adapter.automatic_profile_id().to_string())
        .unwrap_or_else(|_| format!("{adapter_id}.auto"))
}

pub fn adapter_id_for_profile(profile_id: &str) -> Option<&'static str> {
    adapters().into_iter().find_map(|adapter| {
        let profile = adapter.profile();
        (profile.id == profile_id || adapter.automatic_profile_id() == profile_id)
            .then_some(adapter.id())
    })
}

pub fn normalize_adapter_id(value: Option<&str>) -> &'static str {
    value
        .and_then(|id| adapters().into_iter().find(|adapter| adapter.id() == id))
        .map(AgentAdapter::id)
        .unwrap_or(CODEX_ADAPTER_ID)
}

pub fn install_runtime_shims(
    context: &TerminalRuntimeContext,
    target_id: &str,
    hook_endpoint: Option<&str>,
    mcp_endpoint: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    let discovery = crate::agent_command::discover(&["codex", "claude"], target_id);
    if let Some(warning) = discovery.warning.as_deref() {
        eprintln!("Unable to inspect Agent commands for {target_id}: {warning}");
    }
    let mut installed_root = None;
    for adapter in adapters() {
        let profile = adapter.profile();
        if let Some(root) = adapter.install_manual_shim(
            context,
            hook_endpoint,
            mcp_endpoint,
            discovery.paths.get(&profile.command).map(PathBuf::as_path),
        )? {
            installed_root = Some(root);
        }
    }
    Ok(installed_root)
}

pub fn managed_command(launch: &ManagedAgentLaunch<'_>) -> Result<String, String> {
    resolve(&launch.profile.adapter)?.managed_command(launch)
}

pub fn cleanup(runtime_id: &str) {
    let path = std::env::temp_dir().join("luna-mux").join(runtime_id);
    let _ = std::fs::remove_dir_all(path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_exposes_unique_profiles_and_automatic_ids() {
        let profiles = profiles();
        let ids = profiles
            .iter()
            .map(|profile| profile.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(profiles.len(), 2);
        assert_eq!(ids.len(), profiles.len());
        assert_eq!(automatic_profile_id(CODEX_ADAPTER_ID), "codex.auto");
        assert_eq!(
            automatic_profile_id(CLAUDE_CODE_ADAPTER_ID),
            "claude-code.auto"
        );
    }

    #[test]
    fn remote_adapters_receive_the_authenticated_browser_proxy() {
        let context = TerminalManagedAgentContext {
            mux_session_id: "session-1".into(),
            pane_id: "pane-1".into(),
            runtime_id: "runtime-1".into(),
            agent_id: "agent-1".into(),
            launch_profile_id: "test".into(),
        };
        for profile in profiles() {
            let command = managed_command(&ManagedAgentLaunch {
                profile: &profile,
                target_id: "ssh-bookmark:server-1",
                hook_endpoint: "http://127.0.0.1:43127/v1/hooks",
                mcp_endpoint: "http://127.0.0.1:43128/mcp",
                context: &context,
                inject_inline_hooks: true,
                hook_command: Some("'/home/user/.luna-mux/bin/remote-agent' hook"),
                browser_command: Some("/home/user/.luna-mux/bin/remote-agent"),
                browser_credentials_file: None,
                existing_developer_instructions: Some("Keep the remote user rule."),
            })
            .unwrap();
            assert!(command.contains("remote-agent"), "{command}");
            assert!(!command.contains("browser-bridge.json"), "{command}");
            assert!(command.contains("127.0.0.1:43128"), "{command}");
            assert!(!command.contains("lmxbm_"), "{command}");
            assert!(!command.contains("LUNA_MUX_BROWSER_CDP_PORT=43129"));
            if profile.adapter == CODEX_ADAPTER_ID {
                assert!(command.contains("Keep the remote user rule."), "{command}");
            }
        }
    }
}
