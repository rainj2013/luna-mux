use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::{luna_mcp::MCP_AUTHORIZATION_ENV, terminal_runtime_contract::TerminalRuntimeContext};
use crate::shell_quoting::{executable_command_quote, shell_argument_quote};
#[cfg(any(not(windows), test))]
#[allow(unused_imports)]
use crate::shell_quoting::shell_quote;

#[derive(Clone, Debug, PartialEq, Eq)]
struct SkillConfigEntry {
    path: String,
    enabled: bool,
}

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

pub(crate) const LUNA_MUX_BROWSER_INSTRUCTIONS: &str = r#"Luna Mux tool routing contract (apply these routing rules before choosing any tool):
- First classify each requested action into one of three domains. A request may contain actions from more than one domain; route each action separately.
- Luna Mux application control: use the luna_mux MCP for the Luna Mux app's own theme or terminal appearance; saved connection summaries and terminal targets; Mux Sessions; 窗格/Pane creation, metadata, split, and layout; Luna-owned terminal Runtimes including another Pane's input/output/size/flow/lifecycle; managed Agent status/task/interrupt; SFTP transfers; and SSH tunnels/port forwards. Product-UI nouns such as 当前会话, 侧边栏里的连接, 终端窗格, Agent 面板, 传输队列, and 隧道 refer to this domain.
- Web automation: use agent_browser only for web-page concepts such as URL, webpage, DOM, link, form, page screenshot, browser tab, browser window, browser console, or page network traffic. A browser tab/window is not a Luna Mux Pane. Never use agent_browser to create, inspect, rename, split, resize, or arrange Luna Mux application resources.
- General development and host work: use the normal shell, filesystem, code-editing, Git, build, search, and other relevant tools for source files, commands needed to complete the coding task, operating-system settings, and external services. Do not use luna_mux merely because the shell happens to run inside Luna Mux. Do not use Luna Mux `agents.*` as a substitute for the coding agent's own subagent/delegation tools.
- When the user says 窗格, 面板, Pane, terminal pane, 新建窗格, 分屏, split, 布局, or layout without explicitly referring to a web page or browser, they mean a Luna Mux Pane or Mux layout. Use terminal.targets.list when a target is needed, mux.pane.create to create the Pane, and mux.layout.set to replace the split layout. The unqualified Chinese word “窗格” always means a Luna Mux Pane, never a browser tab or window.
- Disambiguate by the object being changed: Luna Mux app chrome/resources => luna_mux; content rendered by a website => agent_browser; repository/host/external-system state => the corresponding native tool. For example, “切换 Luna Mux 深色主题” uses settings.theme.set, “把网页改成深色” uses agent_browser or edits the website code as the task requires, and “修改 macOS 外观” uses neither MCP server.

Luna Mux browser resource contract:
- Browser process lifecycle is owned exclusively by Luna Mux Browser Resources. Never launch Chrome, Chromium, Edge, or another browser through shell commands such as Start-Process, start, open, or direct executable invocation.
- Use only the agent_browser MCP server for browser operations. Do not use a bundled Browser plugin, node_repl browser runtime, Playwright bootstrap, chrome_devtools, or another browser backend.
- Begin each browser workflow with agent_browser_get_url and agent_browser_snapshot. Omit the session argument from every agent_browser tool call; Luna Mux injects the only allowed default session, already pinned to the Browser Resource's existing page. A named session creates a separate page and must not be used. The first agent_browser tool call automatically starts the Session's Browser Resource when needed. Use agent_browser_open to navigate the current bound page.
- Treat the current bound page as the default workspace and reuse it for ordinary single-page navigation, including requests such as "open GitHub".
- Choose browser tools from the needs of the task; the user does not need to prescribe tool-level steps. Create a new tab or window only when the user explicitly asks for one or the task explicitly requires simultaneous page context; use tab/window tools within the injected default session. Do not create a new tab merely as routine navigation setup or error recovery. Do not start or stop browser processes, connect to another browser, or create Browser Resources from Agent tools.
- If automatic startup fails or agent_browser disconnects, report the exact Luna Mux error. Do not attempt to recover by launching a browser process."#;

pub fn install(
    context: &TerminalRuntimeContext,
    mcp_endpoint: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    install_with_executable(context, mcp_endpoint, &executable)
}

fn install_with_executable(
    context: &TerminalRuntimeContext,
    mcp_endpoint: Option<&str>,
    executable: &Path,
) -> Result<Option<PathBuf>, String> {
    let Some(real) = resolve_codex() else {
        return Ok(None);
    };
    let root = std::env::temp_dir()
        .join("luna-mux")
        .join(&context.runtime_id)
        .join("bin");
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let hook = format!("{} hook", quote_path(&executable));
    let hook_value = toml_string(&hook);
    let mut args = Vec::new();
    for event in HOOK_EVENTS {
        let command = if cfg!(windows) {
            let windows_hook_value = toml_string(&format!("& {hook}"));
            format!(
                "hooks.{event}=[{{hooks=[{{type=\"command\",command={hook_value},commandWindows={windows_hook_value}}}]}}]"
            )
        } else {
            format!("hooks.{event}=[{{hooks=[{{type=\"command\",command={hook_value}}}]}}]")
        };
        args.push(command);
    }
    let endpoint = mcp_endpoint.unwrap_or("http://127.0.0.1:0/mcp");
    args.extend([
        "features.hooks=true".into(),
        "features.network_proxy=true".into(),
        "network_proxy.domains.\"127.0.0.1\"=\"allow\"".into(),
        format!("mcp_servers.luna_mux.url={}", toml_string(endpoint)),
        format!(
            "mcp_servers.luna_mux.bearer_token_env_var={}",
            toml_string("LUNA_MUX_MCP_AUTHORIZATION")
        ),
        // Codex's bundled Browser route requires a private desktop-host backend and
        // cannot discover Luna Mux Chrome. Disable only that route, not other plugins.
        "plugins.\"browser@openai-bundled\".enabled=false".into(),
        format!(
            "developer_instructions={}",
            toml_string(&merged_developer_instructions())
        ),
    ]);
    if let Some(override_value) = bundled_browser_skill_override() {
        args.push(override_value);
    }
    let browser_command = format!(
        "mcp_servers.agent_browser.command={}",
        toml_string(&executable.to_string_lossy())
    );
    // TOML literal strings survive Windows native argument forwarding.
    let browser_args = "mcp_servers.agent_browser.args=['mcp','browser']";
    args.extend([
        browser_command.clone(),
        browser_args.into(),
        format!(
            "mcp_servers.agent_browser.env.LUNA_MUX_SESSION_ID={}",
            toml_string(&context.mux_session_id)
        ),
        "mcp_servers.agent_browser.startup_timeout_sec=30".into(),
        "mcp_servers.agent_browser.disabled_tools=['agent_browser_close','agent_browser_connect','agent_browser_dashboard_start','agent_browser_dashboard_stop','agent_browser_install','agent_browser_upgrade','agent_browser_plugin_add','agent_browser_plugin_run','agent_browser_chat']".into(),
        "mcp_servers.agent_browser.enabled=false".into(),
    ]);
    #[cfg(windows)]
    {
        let overrides = args
            .iter()
            .map(|value| format!("'{}'", value.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",");
        let forwarder = executable.to_string_lossy().replace('\'', "''");
        let quote_fn = powershell_native_arg_quote_script();
        let ps = format!(
            "{quote_fn}$overrides = @({overrides})\r\n\
$browserCdpPort = [Environment]::GetEnvironmentVariable('LUNA_MUX_BROWSER_CDP_PORT', 'Process')\r\n\
if (![string]::IsNullOrWhiteSpace($browserCdpPort)) {{\r\n\
  $browserCdpPortToml = ConvertTo-Json ([string]$browserCdpPort) -Compress\r\n\
  $overrides += @(\"mcp_servers.agent_browser.env.LUNA_MUX_BROWSER_CDP_PORT=$browserCdpPortToml\")\r\n\
}}\r\n\
$null = & '{forwarder}' mcp browser available 2>&1\r\n\
if ($LASTEXITCODE -eq 0) {{ $overrides += @('mcp_servers.agent_browser.enabled=true') }}\r\n\
$lunaMuxProcessId = [guid]::NewGuid().ToString('N')\r\n\
$lunaMuxPreviousProcessId = [Environment]::GetEnvironmentVariable('LUNA_MUX_AGENT_PROCESS_ID', 'Process')\r\n\
$lunaMuxPreviousAdapter = [Environment]::GetEnvironmentVariable('LUNA_MUX_AGENT_ADAPTER', 'Process')\r\n\
$lunaMuxCodexExitCode = 1\r\n\
$env:LUNA_MUX_AGENT_PROCESS_ID = $lunaMuxProcessId\r\n\
$env:LUNA_MUX_AGENT_ADAPTER = 'codex'\r\n\
try {{\r\n\
  '{{\"hook_event_name\":\"AgentProcessStart\"}}' | & '{forwarder}' hook | Out-Null\r\n\
  $lunaMuxCodexArguments = @()\r\n\
  foreach ($value in $overrides) {{\r\n\
    $lunaMuxCodexArguments += @('--config', $value)\r\n\
  }}\r\n\
  foreach ($value in $args) {{\r\n\
    $lunaMuxCodexArguments += $value\r\n\
  }}\r\n\
  $lunaMuxCommandLine = (($lunaMuxCodexArguments | ForEach-Object {{ ConvertTo-LunaMuxNativeArg $_ }}) -join ' ')\r\n\
  $psi = New-Object System.Diagnostics.ProcessStartInfo\r\n\
  $psi.FileName = '{}'\r\n\
  $psi.UseShellExecute = $false\r\n\
  $psi.Arguments = $lunaMuxCommandLine\r\n\
  $process = [System.Diagnostics.Process]::Start($psi)\r\n\
  $process.WaitForExit()\r\n\
  $lunaMuxCodexExitCode = $process.ExitCode\r\n\
}} finally {{\r\n\
  '{{\"hook_event_name\":\"AgentProcessExit\"}}' | & '{forwarder}' hook | Out-Null\r\n\
  if ($null -eq $lunaMuxPreviousProcessId) {{ Remove-Item Env:LUNA_MUX_AGENT_PROCESS_ID -ErrorAction SilentlyContinue }} else {{ $env:LUNA_MUX_AGENT_PROCESS_ID = $lunaMuxPreviousProcessId }}\r\n\
  if ($null -eq $lunaMuxPreviousAdapter) {{ Remove-Item Env:LUNA_MUX_AGENT_ADAPTER -ErrorAction SilentlyContinue }} else {{ $env:LUNA_MUX_AGENT_ADAPTER = $lunaMuxPreviousAdapter }}\r\n\
}}\r\n\
$global:LASTEXITCODE = $lunaMuxCodexExitCode\r\n",
            real.to_string_lossy().replace('\'', "''"),
        );
        fs::write(root.join("codex.ps1"), ps).map_err(|error| error.to_string())?;
        // PowerShell profiles (notably fnm) may rewrite PATH after the shell starts.
        // This session-local function is installed after the normal profile loads.
        let shim = root.join("codex.ps1");
        let bootstrap = format!(
            "function global:codex {{ & '{}' @args }}\r\n",
            shim.to_string_lossy().replace('\'', "''"),
        );
        fs::write(root.join("bootstrap.ps1"), bootstrap).map_err(|error| error.to_string())?;
        let cmd = "@echo off\r\npowershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File \"%~dp0codex.ps1\" %*\r\n";
        fs::write(root.join("codex.cmd"), cmd).map_err(|error| error.to_string())?;
    }
    #[cfg(not(windows))]
    {
        let config = args
            .iter()
            .map(|value| format!("--config {}", shell_quote(value)))
            .collect::<Vec<_>>()
            .join(" ");
        let forwarder = shell_quote(&executable.to_string_lossy());
        let browser_config = format!(
            "--config {}",
            shell_quote("mcp_servers.agent_browser.enabled=true")
        );
        let script = format!(
            "#!/bin/sh\n\
LUNA_MUX_AGENT_PROCESS_ID=\"$$-$(date +%s)\"\n\
LUNA_MUX_AGENT_ADAPTER=\"codex\"\n\
export LUNA_MUX_AGENT_PROCESS_ID LUNA_MUX_AGENT_ADAPTER\n\
luna_mux_browser_cdp_port=\"$LUNA_MUX_BROWSER_CDP_PORT\"\n\
luna_mux_browser_cdp_config=\"\"\n\
case \"$luna_mux_browser_cdp_port\" in\n\
  ''|*[!0-9]*) ;;\n\
  *) luna_mux_browser_cdp_config=\"mcp_servers.agent_browser.env.LUNA_MUX_BROWSER_CDP_PORT=\\\"$luna_mux_browser_cdp_port\\\"\" ;;\n\
esac\n\
printf '%s' '{{\"hook_event_name\":\"AgentProcessStart\"}}' | {forwarder} hook >/dev/null 2>&1 || true\n\
if {forwarder} mcp browser available >/dev/null 2>&1; then\n\
  if [ -n \"$luna_mux_browser_cdp_config\" ]; then set -- \"$@\" --config \"$luna_mux_browser_cdp_config\"; fi\n\
  {} \"$@\" {config} {browser_config}\n\
else\n\
  {} \"$@\" {config}\n\
fi\n\
luna_mux_codex_exit_code=$?\n\
printf '%s' '{{\"hook_event_name\":\"AgentProcessExit\"}}' | {forwarder} hook >/dev/null 2>&1 || true\n\
exit \"$luna_mux_codex_exit_code\"\n",
            shell_quote(&real.to_string_lossy()),
            shell_quote(&real.to_string_lossy())
        );
        let path = root.join("codex");
        fs::write(&path, script).map_err(|error| error.to_string())?;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
        #[cfg(target_os = "macos")]
        write_macos_zsh_startup_files(&root)?;
    }
    Ok(Some(root))
}

#[cfg(target_os = "macos")]
pub(crate) fn write_macos_zsh_startup_files(root: &Path) -> Result<(), String> {
    let startup_root = shell_quote(&root.to_string_lossy());
    let source_user_file = |name: &str| {
        format!(
            "_luna_mux_startup_zdotdir=\"$ZDOTDIR\"\n\
ZDOTDIR=\"${{LUNA_MUX_USER_ZDOTDIR:-$HOME}}\"\n\
if [[ \"$ZDOTDIR\" != \"$_luna_mux_startup_zdotdir\" && -r \"$ZDOTDIR/{name}\" ]]; then\n\
  source \"$ZDOTDIR/{name}\"\n\
fi\n\
LUNA_MUX_USER_ZDOTDIR=\"${{ZDOTDIR:-${{LUNA_MUX_USER_ZDOTDIR:-$HOME}}}}\"\n\
export LUNA_MUX_USER_ZDOTDIR\n\
ZDOTDIR=\"$_luna_mux_startup_zdotdir\"\n\
export ZDOTDIR\n\
unset _luna_mux_startup_zdotdir\n"
        )
    };
    let zshenv = source_user_file(".zshenv");
    let zprofile = source_user_file(".zprofile");
    let zshrc = format!(
        "{}source {startup_root}/bootstrap.zsh\n",
        source_user_file(".zshrc")
    );
    let zlogin = format!(
        "{}ZDOTDIR=\"${{LUNA_MUX_USER_ZDOTDIR:-$HOME}}\"\n\
export ZDOTDIR\n\
unset LUNA_MUX_USER_ZDOTDIR\n",
        source_user_file(".zlogin")
    );
    let mut bootstrap = String::new();
    for name in ["codex", "claude"] {
        let shim = root.join(name);
        if shim.is_file() {
            bootstrap.push_str(&format!(
                "unalias {name} 2>/dev/null\n\
function {name} {{\n\
  {} \"$@\"\n\
}}\n",
                shell_quote(&shim.to_string_lossy())
            ));
        }
    }
    for (name, contents) in [
        (".zshenv", zshenv),
        (".zprofile", zprofile),
        (".zshrc", zshrc),
        (".zlogin", zlogin),
        ("bootstrap.zsh", bootstrap),
    ] {
        fs::write(root.join(name), contents).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
pub fn cleanup(runtime_id: &str) {
    let path = std::env::temp_dir().join("luna-mux").join(runtime_id);
    let _ = fs::remove_dir_all(path);
}

pub fn managed_command(
    command: &str,
    target_id: &str,
    inject_inline_hooks: bool,
    hook_command: Option<&str>,
    mcp_endpoint: &str,
    browser_command: Option<&str>,
    browser_credentials_file: Option<&str>,
    mux_session_id: &str,
    existing_developer_instructions: Option<&str>,
) -> Result<String, String> {
    let mut command = command.trim().to_string();
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut overrides = vec![
        "features.network_proxy=true".to_string(),
        "network_proxy.domains.\"127.0.0.1\"=\"allow\"".to_string(),
    ];
    if target_id.starts_with("local:wsl:") {
        let proxy = hook_executable_for_target(&executable, target_id)?;
        overrides.extend([
            format!("mcp_servers.luna_mux.command={}", toml_string(&proxy)),
            "mcp_servers.luna_mux.args=['mcp','luna']".into(),
        ]);
    } else {
        overrides.extend([
            format!("mcp_servers.luna_mux.url={}", toml_string(mcp_endpoint)),
            format!(
                "mcp_servers.luna_mux.bearer_token_env_var={}",
                toml_string(MCP_AUTHORIZATION_ENV)
            ),
        ]);
    }
    let local_browser_command = if target_id.starts_with("ssh-bookmark:") {
        None
    } else {
        Some(hook_executable_for_target(&executable, target_id)?)
    };
    if let Some(browser_command) = browser_command.or(local_browser_command.as_deref()) {
        let developer_instructions = if target_id.starts_with("ssh-bookmark:") {
            merge_developer_instructions(existing_developer_instructions.unwrap_or_default())
        } else {
            merged_developer_instructions()
        };
        overrides.extend([
            format!(
                "mcp_servers.agent_browser.command={}",
                toml_string(browser_command)
            ),
            if target_id.starts_with("ssh-bookmark:") {
                "mcp_servers.agent_browser.args=[]".into()
            } else {
                "mcp_servers.agent_browser.args=['mcp','browser']".into()
            },
            format!(
                "mcp_servers.agent_browser.env.LUNA_MUX_SESSION_ID={}",
                toml_string(mux_session_id)
            ),
            "mcp_servers.agent_browser.startup_timeout_sec=30".into(),
            "mcp_servers.agent_browser.disabled_tools=['agent_browser_close','agent_browser_connect','agent_browser_dashboard_start','agent_browser_dashboard_stop','agent_browser_install','agent_browser_upgrade','agent_browser_plugin_add','agent_browser_plugin_run','agent_browser_chat']".into(),
            "mcp_servers.agent_browser.enabled=true".into(),
            "plugins.\"browser@openai-bundled\".enabled=false".into(),
            format!(
                "developer_instructions={}",
                toml_string(&developer_instructions)
            ),
        ]);
        if let Some(path) = browser_credentials_file {
            overrides.push(format!(
                "mcp_servers.agent_browser.env.LUNA_MUX_BROWSER_BRIDGE_CREDENTIALS={}",
                toml_string(path)
            ));
        }
        if !target_id.starts_with("ssh-bookmark:")
            && !target_id.starts_with("local:wsl:")
            && let Some(override_value) = bundled_browser_skill_override()
        {
            overrides.push(override_value);
        }
    }
    for override_value in overrides {
        command = format!(
            "{command} --config {}",
            shell_argument_quote(&override_value, target_id)
        );
    }
    if !inject_inline_hooks {
        return Ok(command);
    }
    let hook = match hook_command {
        Some(command) => command.to_string(),
        None => {
            let hook_executable = hook_executable_for_target(&executable, target_id)?;
            format!("{} hook", executable_command_quote(&hook_executable))
        }
    };
    let hook_command_value = toml_string(&hook);
    let handler = if crate::local_pty_backend::is_powershell_target(target_id) {
        let windows_hook_command_value = toml_string(&format!("& {hook}"));
        format!(
            "[{{hooks=[{{type=\"command\",command={hook_command_value},commandWindows={windows_hook_command_value}}}]}}]"
        )
    } else {
        format!("[{{hooks=[{{type=\"command\",command={hook_command_value}}}]}}]")
    };
    Ok(HOOK_EVENTS.into_iter().fold(command, |value, event| {
        format!(
            "{value} --config {}",
            shell_argument_quote(&format!("hooks.{event}={handler}"), target_id)
        )
    }))
}

#[cfg(windows)]
pub fn install_wsl_manual_bootstrap(
    context: &TerminalRuntimeContext,
    target_id: &str,
    mcp_endpoint: &str,
    environment_file: Option<&str>,
) -> Result<String, String> {
    if !target_id.starts_with("local:wsl:") {
        return Err("WSL Codex 启动脚本只能安装到 WSL 终端".into());
    }
    let root = std::env::temp_dir()
        .join("luna-mux")
        .join(&context.runtime_id)
        .join("bin");
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let env_source = environment_file
        .map(Path::new)
        .map(|path| hook_executable_for_target(path, target_id))
        .transpose()?
        .map(|path| {
            format!(
                "luna_mux_env_file={}\nif [ -r \"$luna_mux_env_file\" ]; then . \"$luna_mux_env_file\"; fi\n",
                crate::shell_quoting::shell_quote(&path)
            )
        })
        .unwrap_or_default();
    let command = managed_command(
        "command codex",
        target_id,
        true,
        None,
        mcp_endpoint,
        None,
        None,
        &context.mux_session_id,
        None,
    )?;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let forwarder = executable_command_quote(&hook_executable_for_target(&executable, target_id)?);
    let script = format!(
        "codex() (\n\
{env_source}\
LUNA_MUX_AGENT_PROCESS_ID=\"$$-$(date +%s)\"\n\
LUNA_MUX_AGENT_ADAPTER=\"codex\"\n\
export LUNA_MUX_AGENT_PROCESS_ID LUNA_MUX_AGENT_ADAPTER\n\
export WSLENV=\"LUNA_MUX_AGENT_ADAPTER/w:LUNA_MUX_AGENT_PROCESS_ID/w${{WSLENV:+:$WSLENV}}\"\n\
printf '%s' '{{\"hook_event_name\":\"AgentProcessStart\"}}' | {forwarder} hook >/dev/null 2>&1 || true\n\
{command} \"$@\"\n\
luna_mux_codex_exit_code=$?\n\
printf '%s' '{{\"hook_event_name\":\"AgentProcessExit\"}}' | {forwarder} hook >/dev/null 2>&1 || true\n\
exit \"$luna_mux_codex_exit_code\"\n\
)\n"
    );
    let path = root.join("wsl-bootstrap.sh");
    fs::write(&path, script).map_err(|error| error.to_string())?;
    let wsl_path = hook_executable_for_target(&path, target_id)?;
    Ok(format!(". {}", shell_argument_quote(&wsl_path, target_id)))
}

pub(crate) fn hook_executable_for_target(
    executable: &Path,
    target_id: &str,
) -> Result<String, String> {
    let value = executable.to_string_lossy().into_owned();
    if cfg!(windows) && target_id.starts_with("local:wsl:") {
        let bytes = value.as_bytes();
        if bytes.len() < 3 || bytes[1] != b':' || !matches!(bytes[2], b'\\' | b'/') {
            return Err("WSL Agent Hook 需要位于 Windows 本地盘的 Luna Mux 可执行文件".into());
        }
        let drive = (bytes[0] as char).to_ascii_lowercase();
        return Ok(format!("/mnt/{drive}/{}", value[3..].replace('\\', "/")));
    }
    Ok(value)
}

#[cfg(windows)]
pub(crate) fn powershell_native_arg_quote_script() -> String {
    r#"function ConvertTo-LunaMuxNativeArg {
  param([string]$Value)
  if ([string]::IsNullOrEmpty($Value)) { return '""' }
  if ($Value -notmatch '[\s"]') { return $Value }
  $quoted = '"'
  $pending = 0
  for ($i = 0; $i -lt $Value.Length; $i++) {
    $ch = $Value[$i]
    if ($ch -eq '\') {
      $pending++
    } elseif ($ch -eq '"') {
      $quoted += ('\' * ($pending * 2 + 1)) + '"'
      $pending = 0
    } else {
      if ($pending -gt 0) { $quoted += ('\' * $pending); $pending = 0 }
      $quoted += $ch
    }
  }
  if ($pending -gt 0) { $quoted += ('\' * $pending) }
  return $quoted + '"'
}
"#.replace('\n', "\r\n")
}

fn resolve_codex() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        if let Ok(output) = Command::new("where.exe").arg("codex.exe").output()
            && output.status.success()
            && let Some(path) = first_existing_path(&output.stdout)
        {
            return Some(path);
        }

        let output = Command::new("where.exe").arg("codex.cmd").output().ok()?;
        if !output.status.success() {
            return None;
        }
        let (package, target) = if cfg!(target_arch = "aarch64") {
            ("codex-win32-arm64", "aarch64-pc-windows-msvc")
        } else {
            ("codex-win32-x64", "x86_64-pc-windows-msvc")
        };
        return String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .filter_map(|line| {
                let launcher = canonicalize_windows_path(Path::new(line));
                let bin = launcher.parent()?;
                [
                    bin.join("node_modules")
                        .join("@openai")
                        .join("codex")
                        .join("node_modules")
                        .join("@openai")
                        .join(package)
                        .join("vendor")
                        .join(target)
                        .join("bin")
                        .join("codex.exe"),
                    bin.join("node_modules")
                        .join("@openai")
                        .join("codex")
                        .join("vendor")
                        .join(target)
                        .join("bin")
                        .join("codex.exe"),
                ]
                .into_iter()
                .find(|candidate| candidate.is_file())
            })
            .next();
    }
    #[cfg(not(windows))]
    {
        let output = Command::new("which").arg("codex").output().ok()?;
        if !output.status.success() {
            return None;
        }
        first_existing_path(&output.stdout)
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

fn bundled_browser_skill_override() -> Option<String> {
    let home = codex_home()?;
    bundled_browser_skill_override_in(&home)
}

fn bundled_browser_skill_override_in(home: &Path) -> Option<String> {
    let browser_cache = home
        .join("plugins")
        .join("cache")
        .join("openai-bundled")
        .join("browser");
    let mut bundled_paths = fs::read_dir(browser_cache)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| {
            entry
                .path()
                .join("skills")
                .join("control-in-app-browser")
                .join("SKILL.md")
        })
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    bundled_paths.sort();
    if bundled_paths.is_empty() {
        return None;
    }

    let mut entries = existing_skill_config(home);
    for bundled_path in bundled_paths {
        let bundled_path = bundled_path.to_string_lossy().into_owned();
        if let Some(entry) = entries
            .iter_mut()
            .find(|entry| skill_paths_equal(&entry.path, &bundled_path))
        {
            entry.enabled = false;
        } else {
            entries.push(SkillConfigEntry {
                path: bundled_path,
                enabled: false,
            });
        }
    }

    let entries = entries
        .into_iter()
        .map(|entry| {
            format!(
                "{{path={},enabled={}}}",
                toml_string(&entry.path),
                entry.enabled
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    Some(format!("skills.config=[{entries}]"))
}

fn existing_skill_config(home: &Path) -> Vec<SkillConfigEntry> {
    let Ok(config) = fs::read_to_string(home.join("config.toml")) else {
        return Vec::new();
    };
    let Ok(config) = toml::from_str::<toml::Value>(&config) else {
        return Vec::new();
    };
    let Some(entries) = config
        .get("skills")
        .and_then(|value| value.get("config"))
        .and_then(toml::Value::as_array)
    else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let path = entry.get("path")?.as_str()?.to_owned();
            let enabled = entry
                .get("enabled")
                .and_then(toml::Value::as_bool)
                .unwrap_or(true);
            Some(SkillConfigEntry { path, enabled })
        })
        .collect()
}

fn merged_developer_instructions() -> String {
    let existing = codex_home()
        .and_then(|home| existing_developer_instructions(&home))
        .unwrap_or_default();
    merge_developer_instructions(&existing)
}

fn existing_developer_instructions(home: &Path) -> Option<String> {
    let config = fs::read_to_string(home.join("config.toml")).ok()?;
    toml::from_str::<toml::Value>(&config)
        .ok()?
        .get("developer_instructions")?
        .as_str()
        .map(str::to_owned)
}

fn merge_developer_instructions(existing: &str) -> String {
    let existing = existing.trim();
    if existing.is_empty() {
        LUNA_MUX_BROWSER_INSTRUCTIONS.into()
    } else {
        format!("{existing}\n\n{LUNA_MUX_BROWSER_INSTRUCTIONS}")
    }
}

fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|path| path.join(".codex")))
}

fn skill_paths_equal(left: &str, right: &str) -> bool {
    let normalize = |value: &str| {
        let value = value.trim_end_matches(['/', '\\']).replace('\\', "/");
        if cfg!(windows) {
            value.to_ascii_lowercase()
        } else {
            value
        }
    };
    normalize(left) == normalize(right)
}

#[cfg(windows)]
fn canonicalize_windows_path(path: &Path) -> PathBuf {
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let value = path.to_string_lossy();
    value
        .strip_prefix(r"\\?\")
        .map(PathBuf::from)
        .unwrap_or(path)
}

fn quote_path(path: &Path) -> String {
    format!("\"{}\"", path.to_string_lossy().replace('"', "\\\""))
}

fn toml_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    use crate::terminal_runtime_contract::TerminalRuntimeContext;

    #[test]
    fn browser_skill_override_disables_the_bundled_skill_and_preserves_user_entries() {
        let home = std::env::temp_dir().join(format!("luna-mux-skills-{}", uuid::Uuid::new_v4()));
        let bundled_skill_dir = home
            .join("plugins")
            .join("cache")
            .join("openai-bundled")
            .join("browser")
            .join("test-version")
            .join("skills")
            .join("control-in-app-browser");
        let bundled_skill = bundled_skill_dir.join("SKILL.md");
        std::fs::create_dir_all(&bundled_skill_dir).unwrap();
        std::fs::write(&bundled_skill, "# Browser").unwrap();
        std::fs::write(
            home.join("config.toml"),
            "[[skills.config]]\npath = 'C:\\\\custom-skill'\nenabled = false\n",
        )
        .unwrap();

        let override_value = bundled_browser_skill_override_in(&home).unwrap();
        assert!(override_value.starts_with("skills.config=["));
        let parsed = toml::from_str::<toml::Value>(&format!(
            "[skills]\n{}",
            override_value.trim_start_matches("skills.")
        ))
        .unwrap();
        let entries = parsed["skills"]["config"].as_array().unwrap();
        assert!(entries.iter().any(|entry| {
            entry["path"].as_str() == Some(r"C:\\custom-skill")
                && entry["enabled"].as_bool() == Some(false)
        }));
        assert!(entries.iter().any(|entry| {
            entry["path"].as_str() == Some(bundled_skill.to_string_lossy().as_ref())
                && entry["enabled"].as_bool() == Some(false)
        }));

        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn browser_instructions_preserve_existing_developer_instructions() {
        let merged = merge_developer_instructions("Keep my existing rule.");
        assert!(merged.starts_with("Keep my existing rule.\n\n"));
        assert!(merged.contains("Luna Mux tool routing contract"));
        assert!(
            merged.contains("The unqualified Chinese word “窗格” always means a Luna Mux Pane")
        );
        assert!(merged.contains("mux.pane.create"));
        assert!(merged.contains("mux.layout.set"));
        assert!(merged.contains("Web automation: use agent_browser only for web-page concepts"));
        assert!(merged.contains("General development and host work"));
        assert!(merged.contains("Do not use luna_mux merely because"));
        assert!(merged.contains("settings.theme.set"));
        assert!(merged.contains("Never launch Chrome"));
        assert!(merged.contains("agent_browser_get_url"));
        assert!(merged.contains("Omit the session argument"));
        assert!(merged.contains("A named session creates a separate page"));
        assert!(merged.contains("the user does not need to prescribe tool-level steps"));
        assert!(merged.contains("Create a new tab or window only when"));
    }

    #[test]
    #[cfg(windows)]
    fn installed_codex_accepts_the_generated_windows_shim() {
        if resolve_codex().is_none() {
            return;
        }
        let Some(powershell) =
            crate::local_pty_backend::windows_powershell5_executable()
                .or_else(crate::local_pty_backend::windows_powershell7_executable)
        else {
            return;
        };
        let runtime_id = format!("shim-test-{}", uuid::Uuid::new_v4());
        let context = TerminalRuntimeContext {
            mux_session_id: "session-1".into(),
            pane_id: "pane-1".into(),
            runtime_id: runtime_id.clone(),
        };
        let registry = std::env::temp_dir().join(format!("luna-mux-{runtime_id}.json"));
        let helper = std::env::temp_dir().join(format!("luna-mux-{runtime_id}.cmd"));
        std::fs::write(
            &helper,
            "@echo off\r\nif \"%1 %2 %3\"==\"mcp browser available\" exit /b 0\r\nexit /b 0\r\n",
        )
        .expect("write available Browser helper fixture");
        std::fs::write(
            &registry,
            serde_json::json!([{
                "muxSessionId": "session-1",
                "runtimeId": "browser-runtime-1",
                "cdpPort": 43129,
                "processId": 1,
                "status": "running"
            }])
            .to_string(),
        )
        .expect("write browser registry fixture");
        let root = install_with_executable(&context, Some("http://127.0.0.1:43128/mcp"), &helper)
            .expect("install Codex shim")
            .expect("installed Codex is available");
        let output = Command::new(&powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(root.join("codex.ps1"))
        .args(["mcp", "list"])
        .env("LUNA_MUX_SESSION_ID", "session-1")
        .env("LUNA_MUX_BROWSER_CDP_PORT", "43129")
        .env("LUNA_MUX_BROWSER_REGISTRY_PATH", &registry)
        .output()
        .expect("run generated Codex shim");
        assert!(
            output.status.success(),
            "Codex rejected generated shim configuration: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("agent_browser"), "MCP list was: {stdout}");
        assert!(stdout.contains("mcp browser"), "MCP list was: {stdout}");
        let generated_shim =
            std::fs::read_to_string(root.join("codex.ps1")).expect("read generated Codex shim");
        assert!(
            generated_shim
                .contains("mcp_servers.agent_browser.env.LUNA_MUX_SESSION_ID=\"session-1\""),
            "generated shim did not explicitly forward its Session identity"
        );
        assert!(
            !generated_shim.contains("LUNA_MUX_NPX_PATH"),
            "generated shim still depends on npx"
        );
        assert!(
            generated_shim.contains("mcp_servers.agent_browser.env.LUNA_MUX_BROWSER_CDP_PORT="),
            "generated shim did not explicitly forward the reserved Browser CDP port"
        );
        assert!(
            generated_shim.contains("plugins.\"browser@openai-bundled\".enabled=false"),
            "generated shim did not disable the incompatible bundled Browser plugin"
        );
        assert!(
            !generated_shim.contains("mcp_servers.node_repl"),
            "generated shim unexpectedly changed the general Node REPL"
        );
        assert!(
            !generated_shim.contains("mcp_servers.chrome_devtools"),
            "generated shim still configures the superseded Chrome DevTools MCP"
        );
        assert!(
            generated_shim
                .contains("mcp_servers.agent_browser.disabled_tools=[''agent_browser_close''"),
            "generated shim did not disable Browser Resource lifecycle tools"
        );
        assert!(!generated_shim.contains("agent_browser_tab_close"));
        assert!(!generated_shim.contains("agent_browser_window_new"));
        assert!(
            generated_shim.contains("Luna Mux tool routing contract"),
            "generated shim did not add the Browser routing contract"
        );

        std::fs::write(&helper, "@echo off\r\nexit /b 1\r\n")
            .expect("rewrite unavailable Browser helper fixture");
        std::fs::write(
            &registry,
            serde_json::json!([{
                "muxSessionId": "session-1",
                "runtimeId": "browser-runtime-1",
                "cdpPort": 43129,
                "processId": 1,
                "status": "stopped"
            }])
            .to_string(),
        )
        .expect("rewrite stopped browser registry fixture");
        let output_without_browser = Command::new(&powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(root.join("codex.ps1"))
        .args(["mcp", "list"])
        .env("LUNA_MUX_SESSION_ID", "session-1")
        .env("LUNA_MUX_BROWSER_CDP_PORT", "43129")
        .env("LUNA_MUX_BROWSER_REGISTRY_PATH", &registry)
        .output()
        .expect("run generated Codex shim without browser");
        assert!(
            output_without_browser.status.success(),
            "Codex rejected shim without browser: {}{}",
            String::from_utf8_lossy(&output_without_browser.stdout),
            String::from_utf8_lossy(&output_without_browser.stderr)
        );
        let stdout_without_browser = String::from_utf8_lossy(&output_without_browser.stdout);
        assert!(
            stdout_without_browser
                .lines()
                .find(|line| line.contains("agent_browser"))
                .is_some_and(|line| line.contains("disabled")),
            "agent-browser MCP was not disabled without a Browser Runtime: {stdout_without_browser}"
        );
        cleanup(&runtime_id);
        let _ = std::fs::remove_file(registry);
        let _ = std::fs::remove_file(helper);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn zsh_startup_keeps_user_profiles_but_routes_agents_through_runtime_shims() {
        let root = std::env::temp_dir().join(format!("luna-mux-zsh-{}", uuid::Uuid::new_v4()));
        let user_home = root.join("home");
        let real_bin = root.join("real-bin");
        let startup_root = root.join("startup");
        fs::create_dir_all(&user_home).unwrap();
        fs::create_dir_all(&real_bin).unwrap();
        fs::create_dir_all(&startup_root).unwrap();

        fs::write(
            user_home.join(".zprofile"),
            format!(
                "export PATH={}:$PATH\n",
                shell_quote(&real_bin.to_string_lossy())
            ),
        )
        .unwrap();
        fs::write(
            user_home.join(".zshrc"),
            "export LUNA_MUX_TEST_USER_ZSHRC=loaded\n",
        )
        .unwrap();
        for name in ["codex", "claude"] {
            let real = real_bin.join(name);
            fs::write(&real, format!("#!/bin/sh\necho real-{name}\n")).unwrap();
            fs::set_permissions(&real, fs::Permissions::from_mode(0o700)).unwrap();
            let shim = startup_root.join(name);
            fs::write(&shim, format!("#!/bin/sh\necho shim-{name}:$1\n")).unwrap();
            fs::set_permissions(&shim, fs::Permissions::from_mode(0o700)).unwrap();
        }
        write_macos_zsh_startup_files(&startup_root).unwrap();

        let output = Command::new("/bin/zsh")
            .args([
                "-lic",
                "whence -w codex; codex marker; whence -w claude; claude marker; printf 'user-rc:%s\\n' \"$LUNA_MUX_TEST_USER_ZSHRC\"; printf 'zdotdir:%s\\n' \"$ZDOTDIR\"",
            ])
            .env("HOME", &user_home)
            .env("ZDOTDIR", &startup_root)
            .env("LUNA_MUX_USER_ZDOTDIR", &user_home)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(stdout.contains("codex: function"), "{stdout}");
        assert!(stdout.contains("shim-codex:marker"), "{stdout}");
        assert!(!stdout.contains("real-codex"), "{stdout}");
        assert!(stdout.contains("claude: function"), "{stdout}");
        assert!(stdout.contains("shim-claude:marker"), "{stdout}");
        assert!(!stdout.contains("real-claude"), "{stdout}");
        assert!(stdout.contains("user-rc:loaded"), "{stdout}");
        assert!(
            stdout.contains(&format!("zdotdir:{}", user_home.display())),
            "{stdout}"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn installed_codex_accepts_targeted_browser_overrides_without_changing_node_repl() {
        if resolve_codex().is_none() {
            return;
        }
        let runtime_id = format!("transport-test-{}", uuid::Uuid::new_v4());
        let context = TerminalRuntimeContext {
            mux_session_id: "session-1".into(),
            pane_id: "pane-1".into(),
            runtime_id: runtime_id.clone(),
        };
        let helper = std::env::temp_dir().join(format!("luna-mux-{runtime_id}.sh"));
        fs::write(
            &helper,
            "#!/bin/sh\nif [ \"$1 $2 $3\" = \"mcp browser available\" ]; then exit 1; fi\nexit 0\n",
        )
        .unwrap();
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
        let root = install_with_executable(&context, Some("http://127.0.0.1:43128/mcp"), &helper)
            .unwrap()
            .expect("installed Codex is available");

        let output = Command::new(root.join("codex"))
            .args(["mcp", "list"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "Codex rejected generated MCP configuration: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let generated_shim = fs::read_to_string(root.join("codex")).unwrap();
        assert!(
            generated_shim.contains("plugins.\"browser@openai-bundled\".enabled=false"),
            "generated shim did not disable the bundled Browser plugin"
        );
        assert!(
            !generated_shim.contains("mcp_servers.node_repl"),
            "generated shim unexpectedly changed the general Node REPL"
        );
        assert!(
            generated_shim.contains("mcp_servers.agent_browser.command="),
            "generated shim did not configure Luna Mux agent-browser"
        );

        cleanup(&runtime_id);
        let _ = fs::remove_file(helper);
    }
}
