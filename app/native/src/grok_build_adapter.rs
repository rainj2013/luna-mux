use std::{
    fs,
    path::{Path, PathBuf},
};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use serde_json::json;

use crate::{
    agent_adapters::{GROK_BUILD_ADAPTER_ID, ManagedAgentLaunch, runtime_root, runtime_shim_root},
    luna_mcp::MCP_AUTHORIZATION_ENV,
    shell_quoting::{posix_shell_quote, shell_argument_quote, shell_quote},
    terminal_runtime_contract::TerminalRuntimeContext,
};

const HOOK_EVENTS: [&str; 13] = [
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PermissionDenied",
    "PostToolUse",
    "PostToolUseFailure",
    "SubagentStart",
    "SubagentStop",
    "Stop",
    "StopFailure",
    "Notification",
    "PreCompact",
];

pub fn install(
    context: &TerminalRuntimeContext,
    _hook_endpoint: Option<&str>,
    mcp_endpoint: Option<&str>,
    resolved_command: Option<&Path>,
) -> Result<Option<PathBuf>, String> {
    let Some(real) = resolved_command else {
        return Ok(None);
    };
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let root = runtime_shim_root(context);
    let home = runtime_root(context).join("grok");
    crate::runtime_env::write_runtime_owner(&context.runtime_id)?;
    #[cfg(windows)]
    let hook_command = format!(
        "powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -Command \"& '{}' hook\"",
        powershell_literal(&executable.to_string_lossy())
    );
    #[cfg(not(windows))]
    let hook_command = format!("{} hook", shell_quote(&executable.to_string_lossy()));
    write_config_files(
        &home,
        mcp_endpoint.unwrap_or("http://127.0.0.1:0/mcp"),
        &hook_command,
        Some(&executable.to_string_lossy()),
        None,
        context,
    )?;

    #[cfg(windows)]
    {
        let script = format!(
            "$lunaMuxGrokHome='{}'\r\n$originalGrokHome=if ($env:GROK_HOME) {{ $env:GROK_HOME }} else {{ Join-Path $HOME '.grok' }}\r\nNew-Item -ItemType Directory -Force -Path $lunaMuxGrokHome | Out-Null\r\nforeach ($file in @('auth.json','mcp_credentials.json')) {{ $source=Join-Path $originalGrokHome $file; $target=Join-Path $lunaMuxGrokHome $file; if ((Test-Path $source) -and -not (Test-Path $target)) {{ Copy-Item $source $target -Force }} }}\r\n$env:GROK_HOME=$lunaMuxGrokHome\r\n$env:LUNA_MUX_AGENT_ADAPTER='{}'\r\n$env:LUNA_MUX_AGENT_PROCESS_ID=[guid]::NewGuid().ToString('N')\r\n$forwarder='{}'\r\ntry {{ '{{\"hook_event_name\":\"AgentProcessStart\",\"agent_adapter\":\"{}\"}}' | & $forwarder hook | Out-Null; & '{}' @args; $code=$LASTEXITCODE }} finally {{ '{{\"hook_event_name\":\"AgentProcessExit\",\"agent_adapter\":\"{}\"}}' | & $forwarder hook | Out-Null }}\r\nexit $code\r\n",
            powershell_literal(&home.to_string_lossy()), GROK_BUILD_ADAPTER_ID,
            powershell_literal(&executable.to_string_lossy()), GROK_BUILD_ADAPTER_ID,
            powershell_literal(&real.to_string_lossy()), GROK_BUILD_ADAPTER_ID,
        );
        fs::write(root.join("grok.ps1"), script).map_err(|error| error.to_string())?;
        let bootstrap_path = root.join("bootstrap.ps1");
        let mut bootstrap = fs::read_to_string(&bootstrap_path).unwrap_or_default();
        if !bootstrap.contains("function global:grok") {
            bootstrap.push_str(&format!(
                "function global:grok {{ & '{}' @args }}\r\n",
                powershell_literal(&root.join("grok.ps1").to_string_lossy())
            ));
            fs::write(bootstrap_path, bootstrap).map_err(|error| error.to_string())?;
        }
        fs::write(root.join("grok.cmd"), "@echo off\r\npowershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File \"%~dp0grok.ps1\" %*\r\n").map_err(|error| error.to_string())?;
    }
    #[cfg(not(windows))]
    {
        let script = format!(
            "#!/bin/sh\nluna_mux_user_grok_home=\"${{GROK_HOME:-$HOME/.grok}}\"\nexport GROK_HOME={}\nfor file in auth.json mcp_credentials.json; do if [ -r \"$luna_mux_user_grok_home/$file\" ] && [ ! -r \"$GROK_HOME/$file\" ]; then cp \"$luna_mux_user_grok_home/$file\" \"$GROK_HOME/$file\" 2>/dev/null || true; fi; done\nexport LUNA_MUX_AGENT_ADAPTER={}\nexport LUNA_MUX_AGENT_PROCESS_ID=\"$$-$(date +%s)\"\nprintf '%s' '{{\"hook_event_name\":\"AgentProcessStart\",\"agent_adapter\":\"{}\"}}' | {} hook >/dev/null 2>&1 || true\n{} \"$@\"\ncode=$?\nprintf '%s' '{{\"hook_event_name\":\"AgentProcessExit\",\"agent_adapter\":\"{}\"}}' | {} hook >/dev/null 2>&1 || true\nexit $code\n",
            shell_quote(&home.to_string_lossy()), shell_quote(GROK_BUILD_ADAPTER_ID),
            GROK_BUILD_ADAPTER_ID, shell_quote(&executable.to_string_lossy()), shell_quote(&real.to_string_lossy()),
            GROK_BUILD_ADAPTER_ID, shell_quote(&executable.to_string_lossy()),
        );
        let path = root.join("grok");
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
    let browser_command = launch.browser_command.map(str::to_string).or_else(|| {
        executable_for_target(&executable, launch.target_id)
            .ok()
            .map(|path| path.to_string_lossy().into_owned())
    });
    let hook_command = launch.hook_command.map(str::to_string).or_else(|| {
        executable_for_target(&executable, launch.target_id)
            .ok()
            .map(|path| format!("{} hook", shell_quote(&path.to_string_lossy())))
    });
    let context = TerminalRuntimeContext {
        mux_session_id: launch.context.mux_session_id.clone(),
        pane_id: launch.context.pane_id.clone(),
        runtime_id: launch.context.runtime_id.clone(),
    };
    let (config, hooks) = config_contents(
        launch.mcp_endpoint,
        hook_command.as_deref().unwrap_or("true"),
        browser_command.as_deref(),
        launch.browser_credentials_file,
        &context,
    )?;
    if launch.target_id.starts_with("ssh-bookmark:") {
        let root = format!(
            "${{TMPDIR:-/tmp}}/luna-mux-{}/grok",
            posix_shell_quote(&launch.context.runtime_id)
        );
        return Ok(format!("luna_mux_user_grok_home=\"${{GROK_HOME:-$HOME/.grok}}\"; GROK_HOME=\"{root}\"; export GROK_HOME; mkdir -p \"$GROK_HOME/hooks\"; for file in auth.json mcp_credentials.json; do if [ -r \"$luna_mux_user_grok_home/$file\" ]; then cp \"$luna_mux_user_grok_home/$file\" \"$GROK_HOME/$file\" 2>/dev/null || true; fi; done; printf %s {} | (base64 -d 2>/dev/null || base64 -D) > \"$GROK_HOME/config.toml\"; printf %s {} | (base64 -d 2>/dev/null || base64 -D) > \"$GROK_HOME/hooks/luna-mux.json\"; {}", posix_shell_quote(&BASE64.encode(config)), posix_shell_quote(&BASE64.encode(hooks)), launch.profile.command.trim(), root = root));
    }
    let home = runtime_root(&context).join("grok");
    write_config_files(
        &home,
        launch.mcp_endpoint,
        hook_command.as_deref().unwrap_or("true"),
        browser_command.as_deref(),
        launch.browser_credentials_file,
        &context,
    )?;
    let home = executable_for_target(&home, launch.target_id)?;
    let home = shell_argument_quote(&home.to_string_lossy(), launch.target_id);
    let command = shell_argument_quote(launch.profile.command.trim(), launch.target_id);
    if crate::local_pty_backend::is_powershell_target(launch.target_id) {
        Ok(format!("$env:GROK_HOME={home}; {command}"))
    } else {
        Ok(format!("GROK_HOME={home} {command}"))
    }
}

pub fn requires_remote_hook_helper() -> bool {
    true
}

#[cfg(windows)]
pub fn install_wsl_manual_bootstrap(
    context: &TerminalRuntimeContext,
    target_id: &str,
    _hook_endpoint: &str,
    mcp_endpoint: &str,
    environment_file: Option<&str>,
) -> Result<String, String> {
    if !target_id.starts_with("local:wsl:") {
        return Err("WSL Grok Build 启动脚本只能安装到 WSL 终端".into());
    }
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let browser = executable_for_target(&executable, target_id)?;
    let root = runtime_root(context);
    let home = root.join("grok");
    let hook = format!("{} hook", shell_quote(&browser.to_string_lossy()));
    write_config_files(
        &home,
        mcp_endpoint,
        &hook,
        Some(&browser.to_string_lossy()),
        None,
        context,
    )?;
    let wsl_home = executable_for_target(&home, target_id)?;
    let wsl_env = environment_file
        .map(Path::new)
        .map(|path| executable_for_target(path, target_id))
        .transpose()?;
    let env_source = wsl_env
        .as_ref()
        .map(|path| {
            format!(
                "if [ -r {} ]; then . {}; fi\n",
                shell_quote(&path.to_string_lossy()),
                shell_quote(&path.to_string_lossy())
            )
        })
        .unwrap_or_default();
    let script = format!(
        "grok() (\n{}luna_mux_user_grok_home=\"${{GROK_HOME:-$HOME/.grok}}\"\nexport GROK_HOME={}\nfor file in auth.json mcp_credentials.json; do if [ -r \"$luna_mux_user_grok_home/$file\" ]; then cp \"$luna_mux_user_grok_home/$file\" \"$GROK_HOME/$file\" 2>/dev/null || true; fi; done\nexport LUNA_MUX_AGENT_ADAPTER={}\nexport LUNA_MUX_AGENT_PROCESS_ID=\"$$-$(date +%s)\"\nprintf '%s' '{{\"hook_event_name\":\"AgentProcessStart\",\"agent_adapter\":\"{}\"}}' | {} hook >/dev/null 2>&1 || true\ncommand grok \"$@\"\ncode=$?\nprintf '%s' '{{\"hook_event_name\":\"AgentProcessExit\",\"agent_adapter\":\"{}\"}}' | {} hook >/dev/null 2>&1 || true\nexit $code\n)\n",
        env_source,
        shell_quote(&wsl_home.to_string_lossy()),
        shell_quote(GROK_BUILD_ADAPTER_ID),
        GROK_BUILD_ADAPTER_ID,
        shell_quote(&browser.to_string_lossy()),
        GROK_BUILD_ADAPTER_ID,
        shell_quote(&browser.to_string_lossy()),
    );
    let path = root.join("grok-wsl-bootstrap.sh");
    fs::write(&path, script).map_err(|error| error.to_string())?;
    let wsl_path = executable_for_target(&path, target_id)?;
    Ok(format!(". {}", shell_quote(&wsl_path.to_string_lossy())))
}

fn write_config_files(
    home: &Path,
    mcp_endpoint: &str,
    hook_command: &str,
    browser_command: Option<&str>,
    browser_credentials_file: Option<&str>,
    context: &TerminalRuntimeContext,
) -> Result<(), String> {
    fs::create_dir_all(home.join("hooks")).map_err(|error| error.to_string())?;
    let (config, hooks) = config_contents(
        mcp_endpoint,
        hook_command,
        browser_command,
        browser_credentials_file,
        context,
    )?;
    fs::write(home.join("config.toml"), config).map_err(|error| error.to_string())?;
    fs::write(home.join("hooks/luna-mux.json"), hooks).map_err(|error| error.to_string())?;
    Ok(())
}

fn config_contents(
    mcp_endpoint: &str,
    hook_command: &str,
    browser_command: Option<&str>,
    browser_credentials_file: Option<&str>,
    context: &TerminalRuntimeContext,
) -> Result<(String, String), String> {
    let mut config = format!("[mcp_servers.luna_mux]\nurl = {}\nheaders = {{ Authorization = \"Bearer ${{{}}}\" }}\nstartup_timeout_sec = 30\n\n", toml_string(mcp_endpoint), MCP_AUTHORIZATION_ENV);
    if let Some(command) = browser_command {
        let mut env = format!(
            "LUNA_MUX_SESSION_ID = {}",
            toml_string(&context.mux_session_id)
        );
        if let Some(path) = browser_credentials_file {
            env.push_str(&format!(
                ", LUNA_MUX_BROWSER_BRIDGE_CREDENTIALS = {}",
                toml_string(path)
            ));
        }
        config.push_str(&format!("[mcp_servers.agent_browser]\ncommand = {}\nargs = [\"mcp\", \"browser\"]\nenv = {{ {} }}\nstartup_timeout_sec = 30\n\n", toml_string(command), env));
    }
    let hooks = HOOK_EVENTS.iter().map(|event| (event.to_string(), json!([{"hooks":[{"type":"command","command":hook_command,"timeout":if *event == "PreToolUse" {25} else {5}}]}]))).collect::<serde_json::Map<_, _>>();
    let hooks =
        serde_json::to_string(&json!({"hooks": hooks})).map_err(|error| error.to_string())?;
    Ok((config, hooks))
}

fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn executable_for_target(executable: &Path, target_id: &str) -> Result<PathBuf, String> {
    let value = executable.to_string_lossy().into_owned();
    if cfg!(windows) && target_id.starts_with("local:wsl:") {
        let bytes = value.as_bytes();
        if bytes.len() < 3 || bytes[1] != b':' {
            return Err("无法将 Windows 路径转换为 WSL 路径".into());
        }
        return Ok(PathBuf::from(format!(
            "/mnt/{}/{}",
            (bytes[0] as char).to_ascii_lowercase(),
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
    #[test]
    fn grok_config_contains_mcp_and_hooks() {
        let context = TerminalRuntimeContext {
            mux_session_id: "session".into(),
            pane_id: "pane".into(),
            runtime_id: "runtime".into(),
        };
        let (config, hooks) = config_contents(
            "http://127.0.0.1:2/mcp",
            "/tmp/luna-mux hook",
            None,
            None,
            &context,
        )
        .unwrap();
        assert!(config.contains("[mcp_servers.luna_mux]"));
        assert!(config.contains(MCP_AUTHORIZATION_ENV));
        assert!(hooks.contains("PermissionDenied"));
        assert!(hooks.contains("StopFailure"));
    }
}
