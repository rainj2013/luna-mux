use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    agent_adapters::{GROK_BUILD_ADAPTER_ID, ManagedAgentLaunch, runtime_root, runtime_shim_root},
    shell_quoting::shell_quote,
    terminal_runtime_contract::TerminalRuntimeContext,
};

pub fn install(
    context: &TerminalRuntimeContext,
    _hook_endpoint: Option<&str>,
    _mcp_endpoint: Option<&str>,
    resolved_command: Option<&Path>,
) -> Result<Option<PathBuf>, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    install_with_executable(context, resolved_command, &executable)
}

fn install_with_executable(
    context: &TerminalRuntimeContext,
    resolved_command: Option<&Path>,
    executable: &Path,
) -> Result<Option<PathBuf>, String> {
    let Some(real) = resolved_command else {
        return Ok(None);
    };
    let root = runtime_shim_root(context);
    crate::runtime_env::write_runtime_owner(&context.runtime_id)?;
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;

    #[cfg(windows)]
    {
        let script = format!(
            "$env:LUNA_MUX_AGENT_ADAPTER='{}'\r\n$env:LUNA_MUX_AGENT_PROCESS_ID=[guid]::NewGuid().ToString('N')\r\n$forwarder='{}'\r\ntry {{ '{{\"hook_event_name\":\"AgentProcessStart\",\"agent_adapter\":\"{}\"}}' | & $forwarder hook | Out-Null; & '{}' @args; $code=$LASTEXITCODE }} finally {{ '{{\"hook_event_name\":\"AgentProcessExit\",\"agent_adapter\":\"{}\"}}' | & $forwarder hook | Out-Null }}\r\nexit $code\r\n",
            GROK_BUILD_ADAPTER_ID,
            powershell_literal(&executable.to_string_lossy()),
            GROK_BUILD_ADAPTER_ID,
            powershell_literal(&real.to_string_lossy()),
            GROK_BUILD_ADAPTER_ID,
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
            "#!/bin/sh\nexport LUNA_MUX_AGENT_ADAPTER={}\nexport LUNA_MUX_AGENT_PROCESS_ID=\"$$-$(date +%s)\"\nprintf '%s' '{{\"hook_event_name\":\"AgentProcessStart\",\"agent_adapter\":\"{}\"}}' | {} hook >/dev/null 2>&1 || true\n{} \"$@\"\ncode=$?\nprintf '%s' '{{\"hook_event_name\":\"AgentProcessExit\",\"agent_adapter\":\"{}\"}}' | {} hook >/dev/null 2>&1 || true\nexit $code\n",
            shell_quote(GROK_BUILD_ADAPTER_ID),
            GROK_BUILD_ADAPTER_ID,
            shell_quote(&executable.to_string_lossy()),
            shell_quote(&real.to_string_lossy()),
            GROK_BUILD_ADAPTER_ID,
            shell_quote(&executable.to_string_lossy()),
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
    let command = launch.profile.command.trim();
    let hook = match launch.hook_command {
        Some(command) => command.to_string(),
        None if launch.target_id.starts_with("local:wsl:") => {
            let executable = std::env::current_exe().map_err(|error| error.to_string())?;
            let executable = executable_for_target(&executable, launch.target_id)?;
            format!("{} hook", shell_quote(&executable.to_string_lossy()))
        }
        None => return Ok(command.to_string()),
    };
    Ok(format!(
        "(printf '%s' '{{\"hook_event_name\":\"AgentProcessStart\",\"agent_adapter\":\"{}\"}}' | {} >/dev/null 2>&1 || true; {}; luna_mux_agent_exit_code=$?; printf '%s' '{{\"hook_event_name\":\"AgentProcessExit\",\"agent_adapter\":\"{}\"}}' | {} >/dev/null 2>&1 || true; exit \"$luna_mux_agent_exit_code\")",
        GROK_BUILD_ADAPTER_ID, hook, command, GROK_BUILD_ADAPTER_ID, hook,
    ))
}

pub fn requires_remote_hook_helper() -> bool {
    true
}

#[cfg(windows)]
pub fn install_wsl_manual_bootstrap(
    context: &TerminalRuntimeContext,
    target_id: &str,
    _hook_endpoint: &str,
    _mcp_endpoint: &str,
    environment_file: Option<&str>,
) -> Result<String, String> {
    if !target_id.starts_with("local:wsl:") {
        return Err("WSL Grok Build 启动脚本只能安装到 WSL 终端".into());
    }
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let browser = executable_for_target(&executable, target_id)?;
    let root = runtime_root(context);
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
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
        "grok() (\n{}export LUNA_MUX_AGENT_ADAPTER={}\nexport LUNA_MUX_AGENT_PROCESS_ID=\"$$-$(date +%s)\"\nprintf '%s' '{{\"hook_event_name\":\"AgentProcessStart\",\"agent_adapter\":\"{}\"}}' | {} hook >/dev/null 2>&1 || true\ncommand grok \"$@\"\ncode=$?\nprintf '%s' '{{\"hook_event_name\":\"AgentProcessExit\",\"agent_adapter\":\"{}\"}}' | {} hook >/dev/null 2>&1 || true\nexit $code\n)\n",
        env_source,
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
    fn managed_command_tracks_lifecycle_without_injecting_mcp_or_grok_home() {
        let context = crate::terminal_runtime_contract::TerminalManagedAgentContext {
            mux_session_id: "session".into(),
            pane_id: "pane".into(),
            runtime_id: "runtime".into(),
            agent_id: "agent".into(),
            launch_profile_id: "grok-build.default".into(),
        };
        let profile = crate::agent_profiles::AgentLaunchProfile {
            id: "grok-build.default".into(),
            label: "Grok Build".into(),
            adapter: GROK_BUILD_ADAPTER_ID.into(),
            command: "grok".into(),
            built_in: true,
        };
        let command = managed_command(&ManagedAgentLaunch {
            profile: &profile,
            target_id: "ssh-bookmark:test",
            hook_endpoint: "http://127.0.0.1:43127/v1/hooks",
            mcp_endpoint: "http://127.0.0.1:43128/mcp",
            context: &context,
            inject_inline_hooks: true,
            hook_command: Some("remote-agent hook"),
            browser_command: Some("remote-agent"),
            browser_credentials_file: Some("/tmp/browser-credentials"),
            existing_developer_instructions: None,
        })
        .unwrap();

        assert!(command.contains("AgentProcessStart"), "{command}");
        assert!(command.contains("AgentProcessExit"), "{command}");
        assert!(command.contains("remote-agent hook"), "{command}");
        assert!(command.contains("; grok;"), "{command}");
        assert!(!command.contains("GROK_HOME"));
        assert!(!command.contains("43128"));
    }

    #[cfg(windows)]
    #[test]
    fn wsl_bootstrap_preserves_the_user_grok_home() {
        let context = TerminalRuntimeContext {
            mux_session_id: "session".into(),
            pane_id: "pane".into(),
            runtime_id: format!("grok-wsl-bootstrap-{}", uuid::Uuid::new_v4()),
        };
        install_wsl_manual_bootstrap(
            &context,
            "local:wsl:Ubuntu",
            "http://127.0.0.1:43127/v1/hooks",
            "http://127.0.0.1:43128/mcp",
            Some(r"C:\Temp\luna-mux\agent.env"),
        )
        .expect("install WSL Grok bootstrap");
        let script = fs::read_to_string(runtime_root(&context).join("grok-wsl-bootstrap.sh"))
            .expect("read WSL Grok bootstrap");

        assert!(!script.contains("GROK_HOME"), "{script}");
        assert!(!script.contains("config.toml"), "{script}");
        assert!(script.contains("command grok \"$@\""), "{script}");

        crate::codex_shim::cleanup(&context.runtime_id);
    }

    #[cfg(windows)]
    #[test]
    fn powershell_shim_preserves_the_user_grok_home() {
        use std::process::Command;

        let fixture = std::env::temp_dir().join(format!(
            "luna-mux-grok-home-fixture-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&fixture).expect("create Grok shim fixture");
        let real = fixture.join("grok.cmd");
        let forwarder = fixture.join("luna-mux.cmd");
        fs::write(&real, "@echo off\r\necho %GROK_HOME%\r\n").expect("write fake Grok command");
        fs::write(&forwarder, "@echo off\r\nexit /b 0\r\n").expect("write fake hook forwarder");
        let context = TerminalRuntimeContext {
            mux_session_id: "session".into(),
            pane_id: "pane".into(),
            runtime_id: format!("grok-powershell-shim-{}", uuid::Uuid::new_v4()),
        };
        let root = install_with_executable(&context, Some(&real), &forwarder)
            .expect("install PowerShell Grok shim")
            .expect("fake Grok command is available");
        let expected_home = fixture.join("user-grok-home");
        let shells = [
            crate::local_pty_backend::windows_powershell5_executable(),
            crate::local_pty_backend::windows_powershell7_executable(),
        ];

        for shell in shells.into_iter().flatten() {
            let output = Command::new(&shell)
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                ])
                .arg(root.join("grok.ps1"))
                .env("GROK_HOME", &expected_home)
                .output()
                .expect("run generated Grok shim");
            let stdout = crate::local_pty_backend::decode_windows_command_output(&output.stdout);
            assert!(output.status.success(), "{shell} failed");
            assert!(
                stdout.contains(&expected_home.to_string_lossy().to_string()),
                "{} changed GROK_HOME: {stdout:?}",
                shell
            );
        }

        crate::codex_shim::cleanup(&context.runtime_id);
        fs::remove_dir_all(fixture).expect("remove Grok shim fixture");
    }
}
