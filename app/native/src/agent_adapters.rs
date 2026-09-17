use std::path::{Path, PathBuf};

use crate::{
    agent_profiles::{
        AgentLaunchProfile, DEFAULT_CLAUDE_CODE_PROFILE_ID, DEFAULT_CODEX_PROFILE_ID,
        DEFAULT_GROK_BUILD_PROFILE_ID,
    },
    terminal_runtime_contract::{TerminalManagedAgentContext, TerminalRuntimeContext},
};

pub const CODEX_ADAPTER_ID: &str = "codex";
pub const CLAUDE_CODE_ADAPTER_ID: &str = "claude-code";
pub const GROK_BUILD_ADAPTER_ID: &str = "grok-build";

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
    fn supports_managed_mcp(&self) -> bool {
        false
    }
    fn requires_remote_hook_helper(&self) -> bool {
        false
    }
}

struct CodexAdapter;
struct ClaudeCodeAdapter;
struct GrokBuildAdapter;

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

    fn supports_managed_mcp(&self) -> bool {
        true
    }

    fn requires_remote_hook_helper(&self) -> bool {
        true
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

    fn supports_managed_mcp(&self) -> bool {
        true
    }
}

impl AgentAdapter for GrokBuildAdapter {
    fn id(&self) -> &'static str {
        GROK_BUILD_ADAPTER_ID
    }

    fn profile(&self) -> AgentLaunchProfile {
        AgentLaunchProfile {
            id: DEFAULT_GROK_BUILD_PROFILE_ID.into(),
            label: "Grok Build".into(),
            adapter: self.id().into(),
            command: "grok".into(),
            built_in: true,
        }
    }

    fn automatic_profile_id(&self) -> &'static str {
        "grok-build.auto"
    }

    fn install_manual_shim(
        &self,
        context: &TerminalRuntimeContext,
        hook_endpoint: Option<&str>,
        mcp_endpoint: Option<&str>,
        resolved_command: Option<&Path>,
    ) -> Result<Option<PathBuf>, String> {
        crate::grok_build_adapter::install(context, hook_endpoint, mcp_endpoint, resolved_command)
    }

    fn managed_command(&self, launch: &ManagedAgentLaunch<'_>) -> Result<String, String> {
        crate::grok_build_adapter::managed_command(launch)
    }

    fn supports_managed_mcp(&self) -> bool {
        false
    }

    fn requires_remote_hook_helper(&self) -> bool {
        crate::grok_build_adapter::requires_remote_hook_helper()
    }
}

static CODEX_ADAPTER: CodexAdapter = CodexAdapter;
static CLAUDE_CODE_ADAPTER: ClaudeCodeAdapter = ClaudeCodeAdapter;
static GROK_BUILD_ADAPTER: GrokBuildAdapter = GrokBuildAdapter;

fn adapters() -> [&'static dyn AgentAdapter; 3] {
    [&CODEX_ADAPTER, &CLAUDE_CODE_ADAPTER, &GROK_BUILD_ADAPTER]
}

pub fn command_names() -> Vec<String> {
    adapters()
        .into_iter()
        .map(|adapter| adapter.profile().command)
        .collect()
}

pub fn requires_remote_hook_helper(adapter_id: &str) -> bool {
    resolve(adapter_id)
        .map(|adapter| adapter.requires_remote_hook_helper())
        .unwrap_or(false)
}

pub fn supports_managed_mcp(adapter_id: &str) -> bool {
    resolve(adapter_id)
        .map(|adapter| adapter.supports_managed_mcp())
        .unwrap_or(false)
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

pub(crate) fn runtime_root(context: &TerminalRuntimeContext) -> PathBuf {
    std::env::temp_dir()
        .join("luna-mux")
        .join(&context.runtime_id)
}

pub(crate) fn runtime_shim_root(context: &TerminalRuntimeContext) -> PathBuf {
    runtime_root(context).join("bin")
}

pub fn install_runtime_shims(
    context: &TerminalRuntimeContext,
    target_id: &str,
    hook_endpoint: Option<&str>,
    mcp_endpoint: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    let commands = command_names();
    let command_refs = commands.iter().map(String::as_str).collect::<Vec<_>>();
    let discovery = crate::agent_command::discover(&command_refs, target_id);
    if let Some(warning) = discovery.warning.as_deref() {
        eprintln!("Unable to inspect Agent commands for {target_id}: {warning}");
    }
    let expected_root = runtime_shim_root(context);
    let mut installed = false;
    for adapter in adapters() {
        let profile = adapter.profile();
        if let Some(root) = adapter.install_manual_shim(
            context,
            hook_endpoint,
            mcp_endpoint,
            discovery.paths.get(&profile.command).map(PathBuf::as_path),
        )? {
            if root != expected_root {
                return Err(format!(
                    "Agent Adapter {} installed its shim outside the shared runtime bin: {}",
                    adapter.id(),
                    root.display()
                ));
            }
            installed = true;
        }
    }
    Ok(installed.then_some(expected_root))
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
        assert_eq!(profiles.len(), 3);
        assert_eq!(ids.len(), profiles.len());
        assert_eq!(automatic_profile_id(CODEX_ADAPTER_ID), "codex.auto");
        assert_eq!(
            automatic_profile_id(CLAUDE_CODE_ADAPTER_ID),
            "claude-code.auto"
        );
        assert_eq!(
            automatic_profile_id(GROK_BUILD_ADAPTER_ID),
            "grok-build.auto"
        );
        assert!(supports_managed_mcp(CODEX_ADAPTER_ID));
        assert!(supports_managed_mcp(CLAUDE_CODE_ADAPTER_ID));
        assert!(!supports_managed_mcp(GROK_BUILD_ADAPTER_ID));
        assert!(requires_remote_hook_helper(CODEX_ADAPTER_ID));
        assert!(!requires_remote_hook_helper(CLAUDE_CODE_ADAPTER_ID));
        assert!(requires_remote_hook_helper(GROK_BUILD_ADAPTER_ID));
    }

    #[test]
    fn local_adapters_share_one_runtime_shim_directory() {
        let context = TerminalRuntimeContext {
            mux_session_id: "session".into(),
            pane_id: "pane".into(),
            runtime_id: format!("adapter-shims-{}", uuid::Uuid::new_v4()),
        };
        let executable = std::env::current_exe().expect("current test executable");
        let expected_root = runtime_shim_root(&context);

        for adapter in adapters() {
            let root = adapter
                .install_manual_shim(
                    &context,
                    Some("http://127.0.0.1:43127/v1/hooks"),
                    Some("http://127.0.0.1:43128/mcp"),
                    Some(&executable),
                )
                .unwrap_or_else(|error| panic!("{} shim failed: {error}", adapter.id()))
                .unwrap_or_else(|| panic!("{} shim was not installed", adapter.id()));
            assert_eq!(
                root,
                expected_root,
                "{} returned the wrong root",
                adapter.id()
            );
        }

        #[cfg(windows)]
        {
            let bootstrap = std::fs::read_to_string(expected_root.join("bootstrap.ps1"))
                .expect("shared PowerShell bootstrap");
            for command in ["codex", "claude", "grok"] {
                assert!(
                    bootstrap.contains(&format!("function global:{command}")),
                    "shared bootstrap did not route {command}: {bootstrap}"
                );
            }
            let grok = std::fs::read_to_string(expected_root.join("grok.ps1"))
                .expect("generated Grok shim");
            assert!(!grok.contains("GROK_HOME"), "{grok}");
            assert!(!grok.contains("Copy-Item"), "{grok}");
        }

        assert!(!runtime_root(&context).join("grok/config.toml").exists());

        cleanup(&context.runtime_id);
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
            if profile.adapter == GROK_BUILD_ADAPTER_ID {
                assert!(command.contains("AgentProcessStart"), "{command}");
                assert!(command.contains("AgentProcessExit"), "{command}");
                assert!(command.contains("remote-agent"), "{command}");
                assert!(!command.contains("GROK_HOME"), "{command}");
                assert!(!command.contains("base64"), "{command}");
                assert!(!command.contains("127.0.0.1:43128"), "{command}");
            } else {
                assert!(command.contains("remote-agent"), "{command}");
                assert!(!command.contains("browser-bridge.json"), "{command}");
                assert!(command.contains("127.0.0.1:43128"), "{command}");
            }
            assert!(!command.contains("lmxbm_"), "{command}");
            assert!(!command.contains("LUNA_MUX_BROWSER_CDP_PORT=43129"));
            if profile.adapter == CODEX_ADAPTER_ID {
                assert!(command.contains("Keep the remote user rule."), "{command}");
            }
        }
    }
}
