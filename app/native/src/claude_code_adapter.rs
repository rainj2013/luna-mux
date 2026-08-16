use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::{Map, Value, json};

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
) -> Result<Option<PathBuf>, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    install_with_executable(context, hook_endpoint, mcp_endpoint, &executable)
}

fn install_with_executable(
    context: &TerminalRuntimeContext,
    hook_endpoint: Option<&str>,
    mcp_endpoint: Option<&str>,
    executable: &Path,
) -> Result<Option<PathBuf>, String> {
    let Some(real) = resolve_claude() else {
        return Ok(None);
    };
    let root = std::env::temp_dir()
        .join("luna-mux")
        .join(&context.runtime_id)
        .join("bin");
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let settings = hook_settings_json(hook_endpoint.unwrap_or("http://127.0.0.1:0/v1/hooks"))?;
    let mcp_endpoint = mcp_endpoint.unwrap_or("http://127.0.0.1:0/mcp");
    let executable_value = executable.to_string_lossy();
    let mcp_without_browser = mcp_config_json(mcp_endpoint, None, &[], None, context)?;
    let mcp_with_browser = mcp_config_json(
        mcp_endpoint,
        Some(executable_value.as_ref()),
        &["mcp", "browser"],
        None,
        context,
    )?;

    #[cfg(windows)]
    {
        let forwarder = executable.to_string_lossy().replace('\'', "''");
        let ps = format!(
            "$settings = '{}'\r\n\
$mcpWithoutBrowser = '{}'\r\n\
$mcpWithBrowser = '{}'\r\n\
$mcpConfig = $mcpWithoutBrowser\r\n\
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
  & '{}' '--settings' $settings '--mcp-config' $mcpConfig '--append-system-prompt' '{}' '--no-chrome' @args\r\n\
  $lunaMuxClaudeExitCode = $LASTEXITCODE\r\n\
}} finally {{\r\n\
  '{{\"hook_event_name\":\"AgentProcessExit\",\"agent_adapter\":\"claude-code\"}}' | & '{forwarder}' hook | Out-Null\r\n\
  if ($null -eq $lunaMuxPreviousProcessId) {{ Remove-Item Env:LUNA_MUX_AGENT_PROCESS_ID -ErrorAction SilentlyContinue }} else {{ $env:LUNA_MUX_AGENT_PROCESS_ID = $lunaMuxPreviousProcessId }}\r\n\
  if ($null -eq $lunaMuxPreviousAdapter) {{ Remove-Item Env:LUNA_MUX_AGENT_ADAPTER -ErrorAction SilentlyContinue }} else {{ $env:LUNA_MUX_AGENT_ADAPTER = $lunaMuxPreviousAdapter }}\r\n\
}}\r\n\
$global:LASTEXITCODE = $lunaMuxClaudeExitCode\r\n",
            powershell_literal(&settings),
            powershell_literal(&mcp_without_browser),
            powershell_literal(&mcp_with_browser),
            real.to_string_lossy().replace('\'', "''"),
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
    let settings = hook_settings_json(launch.hook_endpoint)?;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let local_browser_executable = if launch.target_id.starts_with("ssh-bookmark:") {
        None
    } else {
        Some(executable_for_target(&executable, launch.target_id)?)
    };
    let browser_command = launch
        .browser_command
        .map(str::to_string)
        .or_else(|| local_browser_executable.map(|path| path.to_string_lossy().into_owned()));
    let browser_args = if launch.browser_command.is_some() {
        &[][..]
    } else {
        &["mcp", "browser"][..]
    };
    let mcp = mcp_config_json(
        launch.mcp_endpoint,
        browser_command.as_deref(),
        browser_args,
        launch.browser_credentials_file,
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

fn hook_settings_json(endpoint: &str) -> Result<String, String> {
    let handler = json!({
        "type": "http",
        "url": endpoint,
        "timeout": 5,
        "headers": {
            "Authorization": "Bearer $LUNA_MUX_HOOK_AUTHORIZATION",
            "X-Luna-Mux-Agent-Adapter": CLAUDE_CODE_ADAPTER_ID,
            "X-Luna-Mux-Agent-Process-Id": "$LUNA_MUX_AGENT_PROCESS_ID"
        },
        "allowedEnvVars": ["LUNA_MUX_HOOK_AUTHORIZATION", "LUNA_MUX_AGENT_PROCESS_ID"]
    });
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
    context: &TerminalRuntimeContext,
) -> Result<String, String> {
    let mut servers = Map::new();
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

fn resolve_claude() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        for command in ["claude.exe", "claude.cmd"] {
            let output = Command::new("where.exe").arg(command).output().ok()?;
            if output.status.success()
                && let Some(path) = first_existing_path(&output.stdout)
            {
                return Some(path);
            }
        }
        None
    }
    #[cfg(not(windows))]
    {
        let output = Command::new("which").arg("claude").output().ok()?;
        output
            .status
            .success()
            .then(|| first_existing_path(&output.stdout))
            .flatten()
    }
}

fn first_existing_path(output: &[u8]) -> Option<PathBuf> {
    String::from_utf8_lossy(output)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .find(|path| path.is_file())
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

fn shell_argument_quote(value: &str, target_id: &str) -> String {
    if target_id == "local:powershell" {
        format!("'{}'", value.replace('\'', "''"))
    } else {
        shell_quote(value)
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
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
    fn settings_use_process_scoped_http_hooks() {
        let settings = hook_settings_json("http://127.0.0.1:43127/v1/hooks").unwrap();
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
