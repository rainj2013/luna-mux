use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::{Map, Value, json};
use crate::shell_quoting::{shell_argument_quote, shell_quote};

use crate::{
    agent_adapters::{CLAUDE_CODE_ADAPTER_ID, ManagedAgentLaunch},
    codex_shim::LUNA_MUX_BROWSER_INSTRUCTIONS,
    luna_mcp::MCP_AUTHORIZATION_ENV,
    terminal_runtime_contract::TerminalRuntimeContext,
};

const HOOK_EVENTS: [&str; 9] = [
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

pub fn install(
    context: &TerminalRuntimeContext,
    hook_endpoint: Option<&str>,
    mcp_endpoint: Option<&str>,
    resolved_command: Option<&Path>,
) -> Result<Option<PathBuf>, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    install_with_executable(
        context,
        hook_endpoint,
        mcp_endpoint,
        &executable,
        resolved_command,
    )
}

fn install_with_executable(
    context: &TerminalRuntimeContext,
    hook_endpoint: Option<&str>,
    mcp_endpoint: Option<&str>,
    executable: &Path,
    resolved_command: Option<&Path>,
) -> Result<Option<PathBuf>, String> {
    let Some(real) = resolved_command.map(Path::to_path_buf) else {
        return Ok(None);
    };
    let root = std::env::temp_dir()
        .join("luna-mux")
        .join(&context.runtime_id)
        .join("bin");
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let settings = hook_settings_json(
        hook_endpoint.unwrap_or("http://127.0.0.1:0/v1/hooks"),
        None,
    )?;
    let mcp_endpoint = mcp_endpoint.unwrap_or("http://127.0.0.1:0/mcp");
    let executable_value = executable.to_string_lossy();
    let mcp_without_browser = mcp_config_json(mcp_endpoint, None, &[], None, None, context)?;
    let mcp_with_browser = mcp_config_json(
        mcp_endpoint,
        Some(executable_value.as_ref()),
        &["mcp", "browser"],
        None,
        None,
        context,
    )?;

    #[cfg(windows)]
    {
        let forwarder = executable.to_string_lossy().replace('\'', "''");
        let quote_fn = crate::codex_shim::powershell_native_arg_quote_script();
        let process_invocation = crate::codex_shim::powershell_command_invocation(
            &real,
            "lunaMuxClaudeArguments",
            "lunaMuxClaudeExitCode",
        );
        let ps = format!(
            "{quote_fn}$settings = '{}'\r\n\
$mcpWithoutBrowser = '{}'\r\n\
$mcpWithBrowser = '{}'\r\n\
$mcpConfig = $mcpWithoutBrowser\r\n\
$lunaMuxBrowserInstructions = '{}'\r\n\
$null = & '{forwarder}' mcp browser available 2>&1\r\n\
if ($LASTEXITCODE -eq 0) {{ $mcpConfig = $mcpWithBrowser }}\r\n\
$lunaMuxProcessId = [guid]::NewGuid().ToString('N')\r\n\
$lunaMuxPreviousProcessId = [Environment]::GetEnvironmentVariable('LUNA_MUX_AGENT_PROCESS_ID', 'Process')\r\n\
$lunaMuxPreviousAdapter = [Environment]::GetEnvironmentVariable('LUNA_MUX_AGENT_ADAPTER', 'Process')\r\n\
$lunaMuxClaudeExitCode = 1\r\n\
$env:LUNA_MUX_AGENT_PROCESS_ID = $lunaMuxProcessId\r\n\
$env:LUNA_MUX_AGENT_ADAPTER = 'claude-code'\r\n\
try {{\r\n\
  '{{\"hook_event_name\":\"AgentProcessStart\",\"agent_adapter\":\"claude-code\"}}' | & '{forwarder}' hook | Out-Null\r\n\
  $lunaMuxClaudeArguments = @('--settings', $settings, '--mcp-config', $mcpConfig, '--append-system-prompt', $lunaMuxBrowserInstructions, '--no-chrome')\r\n\
  foreach ($value in $args) {{\r\n\
    $lunaMuxClaudeArguments += $value\r\n\
  }}\r\n\
{process_invocation}\
}} finally {{\r\n\
  '{{\"hook_event_name\":\"AgentProcessExit\",\"agent_adapter\":\"claude-code\"}}' | & '{forwarder}' hook | Out-Null\r\n\
  if ($null -eq $lunaMuxPreviousProcessId) {{ Remove-Item Env:LUNA_MUX_AGENT_PROCESS_ID -ErrorAction SilentlyContinue }} else {{ $env:LUNA_MUX_AGENT_PROCESS_ID = $lunaMuxPreviousProcessId }}\r\n\
  if ($null -eq $lunaMuxPreviousAdapter) {{ Remove-Item Env:LUNA_MUX_AGENT_ADAPTER -ErrorAction SilentlyContinue }} else {{ $env:LUNA_MUX_AGENT_ADAPTER = $lunaMuxPreviousAdapter }}\r\n\
}}\r\n\
$global:LASTEXITCODE = $lunaMuxClaudeExitCode\r\n",
            powershell_literal(&settings),
            powershell_literal(&mcp_without_browser),
            powershell_literal(&mcp_with_browser),
            powershell_literal(LUNA_MUX_BROWSER_INSTRUCTIONS),
        );
        fs::write(root.join("claude.ps1"), ps).map_err(|error| error.to_string())?;
        let shim = root.join("claude.ps1");
        append_powershell_bootstrap(
            &root,
            &format!(
                "function global:claude {{ & '{}' @args }}\r\n",
                shim.to_string_lossy().replace('\'', "''")
            ),
        )?;
        let cmd = "@echo off\r\npowershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File \"%~dp0claude.ps1\" %*\r\n";
        fs::write(root.join("claude.cmd"), cmd).map_err(|error| error.to_string())?;
    }

    #[cfg(not(windows))]
    {
        let forwarder = shell_quote(&executable.to_string_lossy());
        let script = format!(
            "#!/bin/sh\n\
LUNA_MUX_AGENT_PROCESS_ID=\"$$-$(date +%s)\"\n\
LUNA_MUX_AGENT_ADAPTER=\"claude-code\"\n\
export LUNA_MUX_AGENT_PROCESS_ID LUNA_MUX_AGENT_ADAPTER\n\
luna_mux_mcp_config={}\n\
if {forwarder} mcp browser available >/dev/null 2>&1; then luna_mux_mcp_config={}; fi\n\
printf '%s' '{{\"hook_event_name\":\"AgentProcessStart\",\"agent_adapter\":\"claude-code\"}}' | {forwarder} hook >/dev/null 2>&1 || true\n\
{} --settings {} --mcp-config \"$luna_mux_mcp_config\" --append-system-prompt {} --no-chrome \"$@\"\n\
luna_mux_claude_exit_code=$?\n\
printf '%s' '{{\"hook_event_name\":\"AgentProcessExit\",\"agent_adapter\":\"claude-code\"}}' | {forwarder} hook >/dev/null 2>&1 || true\n\
exit \"$luna_mux_claude_exit_code\"\n",
            shell_quote(&mcp_without_browser),
            shell_quote(&mcp_with_browser),
            shell_quote(&real.to_string_lossy()),
            shell_quote(&settings),
            shell_quote(LUNA_MUX_BROWSER_INSTRUCTIONS),
        );
        let path = root.join("claude");
        fs::write(&path, script).map_err(|error| error.to_string())?;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
        #[cfg(target_os = "macos")]
        crate::codex_shim::write_macos_zsh_startup_files(&root)?;
    }
    Ok(Some(root))
}

pub fn managed_command(launch: &ManagedAgentLaunch<'_>) -> Result<String, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let is_wsl = launch.target_id.starts_with("local:wsl:");
    let local_browser_executable = if launch.target_id.starts_with("ssh-bookmark:") {
        None
    } else {
        Some(executable_for_target(&executable, launch.target_id)?)
    };
    let browser_command = launch
        .browser_command
        .map(str::to_string)
        .or_else(|| local_browser_executable.as_ref().map(|path| path.to_string_lossy().into_owned()));
    let browser_args = if launch.browser_command.is_some() {
        &[][..]
    } else {
        &["mcp", "browser"][..]
    };
    let local_hook_command = local_browser_executable.as_ref().map(|path| {
        let path = path.to_string_lossy();
        format!("{} hook", shell_quote(path.as_ref()))
    });
    let hook_command = if is_wsl {
        local_hook_command.as_deref()
    } else {
        None
    };
    let settings = hook_settings_json(launch.hook_endpoint, hook_command)?;
    let luna_mux_args = ["mcp", "luna"];
    let luna_mux_command = local_browser_executable
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());
    let luna_mux_stdio = if is_wsl {
        luna_mux_command
            .as_deref()
            .map(|command| (command, &luna_mux_args[..]))
    } else {
        None
    };
    let mcp = mcp_config_json(
        launch.mcp_endpoint,
        browser_command.as_deref(),
        browser_args,
        launch.browser_credentials_file,
        luna_mux_stdio,
        &TerminalRuntimeContext {
            mux_session_id: launch.context.mux_session_id.clone(),
            pane_id: launch.context.pane_id.clone(),
            runtime_id: launch.context.runtime_id.clone(),
        },
    )?;
    Ok(format!(
        "{} --settings {} --mcp-config {} --append-system-prompt {} --no-chrome",
        launch.profile.command.trim(),
        shell_argument_quote(&settings, launch.target_id),
        shell_argument_quote(&mcp, launch.target_id),
        shell_argument_quote(LUNA_MUX_BROWSER_INSTRUCTIONS, launch.target_id),
    ))
}
#[cfg(windows)]
pub fn install_wsl_manual_bootstrap(
    context: &TerminalRuntimeContext,
    target_id: &str,
    hook_endpoint: &str,
    mcp_endpoint: &str,
    environment_file: Option<&str>,
) -> Result<String, String> {
    if !target_id.starts_with("local:wsl:") {
        return Err("WSL Claude Code 启动脚本只能安装到 WSL 终端".into());
    }
    let root = std::env::temp_dir()
        .join("luna-mux")
        .join(&context.runtime_id)
        .join("bin");
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let env_source = environment_file
        .map(Path::new)
        .map(|path| executable_for_target(path, target_id))
        .transpose()?
        .map(|path| {
            format!(
                "luna_mux_env_file={}\nif [ -r \"$luna_mux_env_file\" ]; then . \"$luna_mux_env_file\"; fi\n",
                shell_quote(&path.to_string_lossy())
            )
        })
        .unwrap_or_default();
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let browser_executable = executable_for_target(&executable, target_id)?;
    let browser_command = browser_executable.to_string_lossy().into_owned();
    let hook_command = format!("{} hook", shell_quote(&browser_command));
    let settings = hook_settings_json(hook_endpoint, Some(&hook_command))?;
    let luna_mux_args = ["mcp", "luna"];
    let mcp_config = mcp_config_json(
        mcp_endpoint,
        Some(&browser_command),
        &["mcp", "browser"],
        None,
        Some((browser_command.as_str(), &luna_mux_args[..])),
        context,
    )?;
    let forwarder = shell_quote(&browser_command);
    let script = format!(
        r#"claude() (
{env_source}LUNA_MUX_AGENT_PROCESS_ID="$$-$(date +%s)"
LUNA_MUX_AGENT_ADAPTER="claude-code"
export LUNA_MUX_AGENT_PROCESS_ID LUNA_MUX_AGENT_ADAPTER
export WSLENV="LUNA_MUX_AGENT_ADAPTER/w:LUNA_MUX_AGENT_PROCESS_ID/w${{WSLENV:+:$WSLENV}}"
printf '%s' '{{"hook_event_name":"AgentProcessStart","agent_adapter":"claude-code"}}' | {forwarder} hook >/dev/null 2>&1 || true
command claude --settings {settings} --mcp-config {mcp_config} --append-system-prompt {instructions} --no-chrome "$@"
luna_mux_claude_exit_code=$?
printf '%s' '{{"hook_event_name":"AgentProcessExit","agent_adapter":"claude-code"}}' | {forwarder} hook >/dev/null 2>&1 || true
exit "$luna_mux_claude_exit_code"
)"#,
        settings = shell_quote(&settings),
        mcp_config = shell_quote(&mcp_config),
        instructions = shell_quote(LUNA_MUX_BROWSER_INSTRUCTIONS),
    );
    let path = root.join("claude-wsl-bootstrap.sh");
    fs::write(&path, script).map_err(|error| error.to_string())?;
    let wsl_path = executable_for_target(&path, target_id)?;
    Ok(format!(". {}", shell_quote(&wsl_path.to_string_lossy())))
}

fn hook_settings_json(endpoint: &str, command: Option<&str>) -> Result<String, String> {
    let handler = if let Some(command) = command {
        json!({
            "type": "command",
            "command": command,
        })
    } else {
        json!({
            "type": "http",
            "url": endpoint,
            "timeout": 5,
            "headers": {
                "Authorization": "Bearer $LUNA_MUX_HOOK_AUTHORIZATION",
                "X-Luna-Mux-Agent-Adapter": CLAUDE_CODE_ADAPTER_ID,
                "X-Luna-Mux-Agent-Process-Id": "$LUNA_MUX_AGENT_PROCESS_ID"
            },
            "allowedEnvVars": ["LUNA_MUX_HOOK_AUTHORIZATION", "LUNA_MUX_AGENT_PROCESS_ID"]
        })
    };
    let hooks = HOOK_EVENTS
        .into_iter()
        .map(|event| {
            let mut event_handler = handler.clone();
            if event == "PreToolUse" {
                // The first browser tool may need to start Chrome (up to 8s)
                // and repair a persisted tab binding (up to 10s).
                event_handler["timeout"] = json!(25);
            }
            (event.into(), json!([{ "hooks": [event_handler] }]))
        })
        .collect::<Map<String, Value>>();
    serde_json::to_string(&json!({ "hooks": hooks })).map_err(|error| error.to_string())
}

fn mcp_config_json(
    endpoint: &str,
    browser_command: Option<&str>,
    browser_args: &[&str],
    browser_credentials_file: Option<&str>,
    luna_mux_stdio: Option<(&str, &[&str])>,
    context: &TerminalRuntimeContext,
) -> Result<String, String> {
    let mut servers = Map::new();
    if let Some((command, args)) = luna_mux_stdio {
        let mut env = Map::new();
        env.insert(
            "LUNA_MUX_MCP_ENDPOINT".into(),
            Value::String("${LUNA_MUX_MCP_ENDPOINT}".into()),
        );
        env.insert(
            MCP_AUTHORIZATION_ENV.into(),
            Value::String(format!("${{{MCP_AUTHORIZATION_ENV}}}")),
        );
        servers.insert(
            "luna_mux".into(),
            json!({
                "type": "stdio",
                "command": command,
                "args": args,
                "env": env,
            }),
        );
    } else {
        servers.insert(
            "luna_mux".into(),
            json!({
                "type": "http",
                "url": endpoint,
                "headers": {
                    "Authorization": format!("Bearer ${{{MCP_AUTHORIZATION_ENV}}}")
                }
            }),
        );
    }
    if let Some(command) = browser_command {
        let mut env = Map::new();
        env.insert(
            "LUNA_MUX_SESSION_ID".into(),
            Value::String(context.mux_session_id.clone()),
        );
        if browser_credentials_file.is_none() {
            env.insert(
                "LUNA_MUX_BROWSER_CDP_PORT".into(),
                Value::String("${LUNA_MUX_BROWSER_CDP_PORT}".into()),
            );
        }
        if let Some(path) = browser_credentials_file {
            env.insert(
                "LUNA_MUX_BROWSER_BRIDGE_CREDENTIALS".into(),
                Value::String(path.into()),
            );
        }
        servers.insert(
            "agent_browser".into(),
            json!({
                "type": "stdio",
                "command": command,
                "args": browser_args,
                "env": env,
            }),
        );
    }
    serde_json::to_string(&json!({ "mcpServers": servers })).map_err(|error| error.to_string())
}

#[cfg(windows)]
fn append_powershell_bootstrap(root: &Path, contribution: &str) -> Result<(), String> {
    let path = root.join("bootstrap.ps1");
    let mut contents = fs::read_to_string(&path).unwrap_or_default();
    if !contents.contains("function global:claude") {
        contents.push_str(contribution);
        fs::write(path, contents).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn executable_for_target(executable: &Path, target_id: &str) -> Result<PathBuf, String> {
    let value = executable.to_string_lossy().into_owned();
    if cfg!(windows) && target_id.starts_with("local:wsl:") {
        let bytes = value.as_bytes();
        if bytes.len() < 3 || bytes[1] != b':' || !matches!(bytes[2], b'\\' | b'/') {
            return Err("WSL Agent Adapter 需要位于 Windows 本地盘的 Luna Mux 可执行文件".into());
        }
        let drive = (bytes[0] as char).to_ascii_lowercase();
        return Ok(PathBuf::from(format!(
            "/mnt/{drive}/{}",
            value[3..].replace('\\', "/")
        )));
    }
    Ok(executable.to_path_buf())
}

#[cfg(windows)]
fn powershell_literal(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> TerminalRuntimeContext {
        TerminalRuntimeContext {
            mux_session_id: "session-1".into(),
            pane_id: "pane-1".into(),
            runtime_id: "runtime-1".into(),
        }
    }

    #[test]
    fn settings_use_command_hooks_for_wsl() {
        let settings = hook_settings_json(
            "http://127.0.0.1:43127/v1/hooks",
            Some("/mnt/c/luna-mux/luna-mux.exe hook"),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&settings).unwrap();
        assert_eq!(
            value["hooks"]["SessionStart"][0]["hooks"][0]["type"],
            "command"
        );
        assert_eq!(
            value["hooks"]["SessionStart"][0]["hooks"][0]["command"],
            "/mnt/c/luna-mux/luna-mux.exe hook"
        );
        assert_eq!(value["hooks"]["PreToolUse"][0]["hooks"][0]["timeout"], 25);
    }

    #[test]
    #[cfg(windows)]
    fn managed_command_uses_wsl_command_hooks_and_stdio_luna_mux() {
        let launch = ManagedAgentLaunch {
            profile: &crate::agent_profiles::AgentLaunchProfile {
                id: "test".into(),
                label: "Claude Code".into(),
                adapter: CLAUDE_CODE_ADAPTER_ID.into(),
                command: "claude".into(),
                built_in: true,
            },
            target_id: "local:wsl:Ubuntu",
            hook_endpoint: "http://127.0.0.1:43127/v1/hooks",
            mcp_endpoint: "http://127.0.0.1:43128/mcp",
            context: &crate::terminal_runtime_contract::TerminalManagedAgentContext {
                mux_session_id: "session-1".into(),
                pane_id: "pane-1".into(),
                runtime_id: "runtime-1".into(),
                agent_id: "agent-1".into(),
                launch_profile_id: "test".into(),
            },
            inject_inline_hooks: true,
            hook_command: None,
            browser_command: None,
            browser_credentials_file: None,
            existing_developer_instructions: None,
        };
        let command = managed_command(&launch).unwrap();
        assert!(command.contains("\"type\":\"command\""), "{command}");
        assert!(command.contains("\"type\":\"stdio\""), "{command}");
        assert!(command.contains("\"args\":[\"mcp\",\"luna\"]"), "{command}");
        assert!(command.contains("/mnt/"), "{command}");
    }

    #[test]
    fn settings_use_process_scoped_http_hooks() {
        let settings = hook_settings_json("http://127.0.0.1:43127/v1/hooks", None).unwrap();
        let value: Value = serde_json::from_str(&settings).unwrap();
        assert_eq!(
            value["hooks"]["SessionStart"][0]["hooks"][0]["type"],
            "http"
        );
        assert_eq!(
            value["hooks"]["PreToolUse"][0]["hooks"][0]["headers"]["X-Luna-Mux-Agent-Adapter"],
            CLAUDE_CODE_ADAPTER_ID
        );
        assert_eq!(value["hooks"]["PreToolUse"][0]["hooks"][0]["timeout"], 25);
        assert_eq!(value["hooks"]["SessionStart"][0]["hooks"][0]["timeout"], 5);
    }

    #[test]
    fn mcp_config_reuses_luna_mux_and_browser_transports() {
        let config = mcp_config_json(
            "http://127.0.0.1:43128/mcp",
            Some("/opt/luna-mux"),
            &["mcp", "browser"],
            None,
            None,
            &context(),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&config).unwrap();
        assert_eq!(value["mcpServers"]["luna_mux"]["type"], "http");
        assert_eq!(
            value["mcpServers"]["agent_browser"]["args"],
            json!(["mcp", "browser"])
        );
        assert_eq!(
            value["mcpServers"]["agent_browser"]["env"]["LUNA_MUX_SESSION_ID"],
            "session-1"
        );
    }

    #[test]
    fn mcp_config_uses_stdio_luna_mux_for_wsl() {
        let args = ["mcp", "luna"];
        let config = mcp_config_json(
            "http://127.0.0.1:43128/mcp",
            Some("/mnt/d/code/luna-mux/luna-mux.exe"),
            &["mcp", "browser"],
            None,
            Some(("/mnt/d/code/luna-mux/luna-mux.exe", &args[..])),
            &context(),
        )
        .unwrap();
        let value: Value = serde_json::from_str(&config).unwrap();
        assert_eq!(value["mcpServers"]["luna_mux"]["type"], "stdio");
        assert_eq!(
            value["mcpServers"]["luna_mux"]["command"],
            "/mnt/d/code/luna-mux/luna-mux.exe"
        );
        assert_eq!(
            value["mcpServers"]["luna_mux"]["args"],
            json!(["mcp", "luna"])
        );
        assert_eq!(
            value["mcpServers"]["luna_mux"]["env"]["LUNA_MUX_MCP_ENDPOINT"],
            "${LUNA_MUX_MCP_ENDPOINT}"
        );
        assert_eq!(
            value["mcpServers"]["luna_mux"]["env"]["LUNA_MUX_MCP_AUTHORIZATION"],
            "${LUNA_MUX_MCP_AUTHORIZATION}"
        );
    }

    #[test]
    fn injected_prompt_routes_luna_mux_panes_before_browser_tools() {
        assert!(LUNA_MUX_BROWSER_INSTRUCTIONS.contains("Luna Mux tool routing contract"));
        assert!(LUNA_MUX_BROWSER_INSTRUCTIONS.contains("mux.pane.create"));
        assert!(LUNA_MUX_BROWSER_INSTRUCTIONS.contains("mux.layout.set"));
        assert!(LUNA_MUX_BROWSER_INSTRUCTIONS.contains("窗格"));
        assert!(
            LUNA_MUX_BROWSER_INSTRUCTIONS
                .contains("Web automation: use agent_browser only for web-page concepts")
        );
        assert!(LUNA_MUX_BROWSER_INSTRUCTIONS.contains("General development and host work"));
        assert!(LUNA_MUX_BROWSER_INSTRUCTIONS.contains("settings.theme.set"));
    }
}
