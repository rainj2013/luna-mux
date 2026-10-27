use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::json;

use crate::{
    agent_adapters::{ManagedAgentLaunch, runtime_shim_root},
    codex_shim::LUNA_MUX_BROWSER_INSTRUCTIONS,
    shell_quoting::shell_quote,
    terminal_runtime_contract::TerminalRuntimeContext,
};

pub const PATCH_NAME: &str = "dsh-tui.patch.yml";
pub const BRIDGE_NAME: &str = "dsh-tui-bridge.mjs";
pub const LAUNCH_NAME: &str = "dsh-tui-launch";
pub const BRIDGE: &str = include_str!("dsh_tui_bridge.mjs");
#[cfg(windows)]
const TERMINAL_COMPAT: &str = include_str!("dsh_tui_terminal_compat.mjs");

pub fn task_payload(latest: Option<&str>, task: &str) -> Result<String, &'static str> {
    if !matches!(latest, Some("DshTuiIdle" | "DshTuiDone" | "DshTuiError")) {
        return Err(
            "DSH TUI is not ready for a task; finish the current prompt or operation first",
        );
    }
    if task
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t'))
    {
        return Err("DSH tasks cannot contain terminal control characters");
    }
    let text = task.replace("\r\n", "\n").replace('\r', "\n");
    Ok(format!("\x1b[200~{text}\x1b[201~\r"))
}

// --patch is a DSH host option and must precede app arguments. These non-secret
// files live under the existing Runtime owner; credentials are evaluated at boot.
pub fn patch(
    bridge: &str,
    browser: &str,
    remote: bool,
    context: &TerminalRuntimeContext,
) -> String {
    let explicit_env = [
        "LUNA_MUX_MCP_ENDPOINT",
        "LUNA_MUX_MCP_AUTHORIZATION",
        "LUNA_MUX_SESSION_ID",
        "LUNA_MUX_BROWSER_REGISTRY_PATH",
        "LUNA_MUX_BROWSER_CDP_PORT",
        "LUNA_MUX_BROWSER_BRIDGE_CREDENTIALS",
        "LUNA_MUX_BROWSER_BRIDGE_PORT",
        "LUNA_MUX_BROWSER_BRIDGE_TOKEN",
    ];
    let env = format!(
        "Object.fromEntries({}.filter(k => process.env[k] !== undefined).map(k => [k, process.env[k]]))",
        json!(explicit_env)
    );
    let luna = if cfg!(windows) && !remote && browser.starts_with("/mnt/") {
        format!(
            "({{serverName:'luna_mux',transport:'stdio',command:{},args:['mcp','luna'],env:{env}}})",
            json!(browser)
        )
    } else {
        "({serverName:'luna_mux',transport:'streamable-http',url:process.env.LUNA_MUX_MCP_ENDPOINT,headers:{Authorization:'Bearer '+process.env.LUNA_MUX_MCP_AUTHORIZATION}})".into()
    };
    let args: &[&str] = if remote { &[] } else { &["mcp", "browser"] };
    let hook_command = if !remote && browser.starts_with("/mnt/") {
        json!([browser, "hook"])
    } else {
        json!(null)
    };
    let entries = format!(
        "- id: luna-mux-mcp\n  name: '@deepseek-ai/dsh-mcp-client'\n  config: !!js {luna}\n\
- id: luna-mux-browser\n  name: '@deepseek-ai/dsh-mcp-client'\n  disabled: !!js (!process.env.LUNA_MUX_BROWSER_CDP_PORT && !process.env.LUNA_MUX_BROWSER_BRIDGE_PORT)\n  config: !!js ({{serverName:'agent_browser',transport:'stdio',command:{browser},args:{args},env:{{...{env},LUNA_MUX_SESSION_ID:{session}}}}})\n\
- id: luna-mux-routing\n  name: {bridge}\n  config:\n    hookCommand: {hook_command}\n    instructions: {instructions}\n",
        browser = json!(browser),
        args = json!(args),
        session = json!(context.mux_session_id),
        bridge = json!(bridge),
        instructions = json!(LUNA_MUX_BROWSER_INSTRUCTIONS),
    );
    // --patch consumes PatchOptions, not a root entry list. Insert new rows
    // without relying on IDs being present in the user's selected profile.
    format!(
        "- insert:\n{}",
        entries
            .lines()
            .map(|line| format!("    {line}\n"))
            .collect::<String>()
    )
}

fn write_support(context: &TerminalRuntimeContext, target: &str) -> Result<PathBuf, String> {
    let root = runtime_shim_root(context);
    crate::runtime_env::write_runtime_owner(&context.runtime_id)?;
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let bridge =
        crate::claude_code_adapter::executable_for_target(&root.join(BRIDGE_NAME), target)?;
    let executable = crate::claude_code_adapter::executable_for_target(
        &std::env::current_exe().map_err(|e| e.to_string())?,
        target,
    )?;
    fs::write(root.join(BRIDGE_NAME), BRIDGE).map_err(|e| e.to_string())?;
    fs::write(
        root.join(PATCH_NAME),
        patch(
            &bridge.to_string_lossy(),
            &executable.to_string_lossy(),
            false,
            context,
        ),
    )
    .map_err(|e| e.to_string())?;
    crate::claude_code_adapter::executable_for_target(&root.join(PATCH_NAME), target)
}

pub fn managed_command(launch: &ManagedAgentLaunch<'_>) -> Result<String, String> {
    let context = TerminalRuntimeContext {
        mux_session_id: launch.context.mux_session_id.clone(),
        pane_id: launch.context.pane_id.clone(),
        runtime_id: launch.context.runtime_id.clone(),
    };
    if launch.target_id.starts_with("ssh-bookmark:") {
        let helper = launch
            .browser_command
            .ok_or("DSH remote Browser helper is missing")?;
        let bin = helper
            .rsplit_once('/')
            .ok_or("Invalid DSH remote helper path")?
            .0;
        return Ok(shell_quote(&format!("{bin}/{LAUNCH_NAME}")));
    }
    let patch = write_support(&context, launch.target_id)?;
    let executable = crate::claude_code_adapter::executable_for_target(
        &std::env::current_exe().map_err(|e| e.to_string())?,
        launch.target_id,
    )?;
    let hook = format!("{} hook", shell_quote(&executable.to_string_lossy()));
    let script = posix_script(
        launch.profile.command.trim(),
        &patch.to_string_lossy(),
        &hook,
        "",
    );
    let path = runtime_shim_root(&context).join(LAUNCH_NAME);
    fs::write(&path, script).map_err(|e| e.to_string())?;
    let path = crate::claude_code_adapter::executable_for_target(&path, launch.target_id)?;
    Ok(format!("sh {}", shell_quote(&path.to_string_lossy())))
}

pub fn install(
    context: &TerminalRuntimeContext,
    real: Option<&Path>,
) -> Result<Option<PathBuf>, String> {
    install_with_executable(
        context,
        real,
        &std::env::current_exe().map_err(|e| e.to_string())?,
    )
}

fn install_with_executable(
    context: &TerminalRuntimeContext,
    real: Option<&Path>,
    executable: &Path,
) -> Result<Option<PathBuf>, String> {
    let Some(real) = real else { return Ok(None) };
    let root = runtime_shim_root(context);
    let patch = write_support(context, "local:native")?;
    #[cfg(windows)]
    {
        let quote = |s: &str| s.replace('\'', "''");
        let terminal_compat = root.join("dsh-tui-terminal-compat.mjs");
        fs::write(&terminal_compat, TERMINAL_COMPAT).map_err(|e| e.to_string())?;
        let terminal_compat_url = url::Url::from_file_path(&terminal_compat)
            .map_err(|_| "Invalid DSH terminal compatibility preload path")?;
        let invocation = crate::codex_shim::powershell_command_invocation(
            real,
            "lunaMuxDshArguments",
            "lunaMuxDshExitCode",
        );
        let script = format!(
            r#"{quote_fn}
$lunaMuxDshArguments = @($args)
# Noninteractive CLI operations must retain their own argument semantics.
$lunaMuxDshInteractive = $true
if ($args.Count -gt 0 -and $args[0] -in @('run','cli','dsh','session','models','skills','mcp','auth','capabilities','update','doctor','migrate','safe','rescue','help','version','--help','-h','--version','-v','-V','--print','-p','--dump-config','--dump-default-config','--dump-config-schema')) {{ $lunaMuxDshInteractive = $false }}
$lunaMuxDshExitCode = 1
$lunaMuxPreviousProcessId = $env:LUNA_MUX_AGENT_PROCESS_ID
$lunaMuxPreviousAdapter = $env:LUNA_MUX_AGENT_ADAPTER
$lunaMuxPreviousNodeOptions = $env:NODE_OPTIONS
$lunaMuxPreviousWin32Input = $env:DSH_TUI_DISABLE_WIN32_INPUT_MODE
try {{
  if ($lunaMuxDshInteractive) {{
    $env:DSH_TUI_DISABLE_WIN32_INPUT_MODE = '1'
    $env:NODE_OPTIONS = (($env:NODE_OPTIONS + ' --import={terminal_compat}').Trim())
    $env:LUNA_MUX_AGENT_PROCESS_ID = [guid]::NewGuid().ToString('N')
    $env:LUNA_MUX_AGENT_ADAPTER = 'dsh-tui'
    '{{"hook_event_name":"AgentProcessStart","agent_adapter":"dsh-tui"}}' | & '{forwarder}' hook | Out-Null
    $lunaMuxDshArguments = @('--patch', '{patch}', '--backend', 'dsh') + $lunaMuxDshArguments
  }}
{invocation}
}} finally {{
  if ($lunaMuxDshInteractive) {{ '{{"hook_event_name":"AgentProcessExit","agent_adapter":"dsh-tui"}}' | & '{forwarder}' hook | Out-Null }}
  $env:LUNA_MUX_AGENT_PROCESS_ID = $lunaMuxPreviousProcessId
  $env:LUNA_MUX_AGENT_ADAPTER = $lunaMuxPreviousAdapter
  $env:NODE_OPTIONS = $lunaMuxPreviousNodeOptions
  $env:DSH_TUI_DISABLE_WIN32_INPUT_MODE = $lunaMuxPreviousWin32Input
}}
$global:LASTEXITCODE = $lunaMuxDshExitCode
"#,
            quote_fn = crate::codex_shim::powershell_native_arg_quote_script(),
            forwarder = quote(&executable.to_string_lossy()),
            patch = quote(&patch.to_string_lossy()),
            terminal_compat = quote(terminal_compat_url.as_str())
        );
        fs::write(root.join("dsh-tui.ps1"), script).map_err(|e| e.to_string())?;
        let bootstrap_path = root.join("bootstrap.ps1");
        let mut bootstrap = fs::read_to_string(&bootstrap_path).unwrap_or_default();
        for name in ["dsh-tui", "dst"] {
            if !bootstrap.contains(&format!("function global:{name} ")) {
                bootstrap.push_str(&format!(
                    "function global:{name} {{ & '{}' @args }}\r\n",
                    quote(&root.join("dsh-tui.ps1").to_string_lossy())
                ));
            }
            fs::write(root.join(format!("{name}.cmd")), "@echo off\r\npowershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File \"%~dp0dsh-tui.ps1\" %*\r\n").map_err(|e| e.to_string())?;
        }
        fs::write(bootstrap_path, bootstrap).map_err(|e| e.to_string())?;
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        let script = posix_script(
            &shell_quote(&real.to_string_lossy()),
            &patch.to_string_lossy(),
            &format!("{} hook", shell_quote(&executable.to_string_lossy())),
            "",
        );
        for name in ["dsh-tui", "dst"] {
            let path = root.join(name);
            fs::write(&path, &script).map_err(|e| e.to_string())?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        #[cfg(target_os = "macos")]
        crate::codex_shim::write_macos_zsh_startup_files(&root)?;
    }
    Ok(Some(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> TerminalRuntimeContext {
        TerminalRuntimeContext {
            mux_session_id: "session".into(),
            pane_id: "pane".into(),
            runtime_id: format!("dsh-test-{}", uuid::Uuid::new_v4()),
        }
    }

    #[test]
    fn tasks_do_not_answer_modals_or_inject_terminal_sequences() {
        for status in [
            None,
            Some("AgentProcessStart"),
            Some("DshTuiWorking"),
            Some("DshTuiPermission"),
            Some("DshTuiQuestion"),
            Some("DshTuiAuth"),
        ] {
            assert!(task_payload(status, "hello").is_err());
        }
        assert!(task_payload(Some("DshTuiIdle"), "\x1b[201~yes").is_err());
        assert_eq!(
            task_payload(Some("DshTuiDone"), "one\r\ntwo\rthree").unwrap(),
            "\x1b[200~one\ntwo\nthree\x1b[201~\r"
        );
    }

    #[test]
    fn patches_keep_credentials_in_environment_and_remote_commands_on_remote_host() {
        let c = context();
        let text = patch(
            "/home/user/a b/bridge.mjs",
            "/home/user/a b/remote-agent",
            true,
            &c,
        );
        assert!(text.starts_with("- insert:\n    - id: luna-mux-mcp\n"));
        assert!(text.contains("transport:'streamable-http'"));
        assert!(text.contains("args:[]"));
        assert!(text.contains("LUNA_MUX_BROWSER_BRIDGE_TOKEN"));
        assert!(text.contains("process.env.LUNA_MUX_MCP_AUTHORIZATION"));
        assert!(!text.contains("127.0.0.1"));
        assert!(!text.contains("lmxbm_"));
        assert!(!text.contains("danger-full-access"));
        assert!(text.contains("Luna Mux tool routing contract"));
    }

    #[cfg(unix)]
    #[test]
    fn posix_shim_preserves_mcp_environment_arguments_and_process_lifecycle() {
        use std::{os::unix::fs::PermissionsExt, process::Command};
        let root = std::env::temp_dir().join(format!("dsh posix {}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let real = root.join("real dsh-tui");
        let hook = root.join("hook");
        let environment = root.join("agent.env");
        let args_file = root.join("args");
        let hooks_file = root.join("hooks");
        fs::write(
            &real,
            r#"#!/bin/sh
[ "$LUNA_MUX_MCP_ENDPOINT" = 'http://127.0.0.1:43128/mcp' ] || exit 90
[ "$LUNA_MUX_MCP_AUTHORIZATION" = 'fixture-only' ] || exit 91
printf '%s\n' "$@" > "$DSH_TEST_ARGS"
exit 17
"#,
        )
        .unwrap();
        fs::write(&hook, "#!/bin/sh\ncat >> \"$DSH_TEST_HOOKS\"\n").unwrap();
        for path in [&real, &hook] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::write(&environment, "LUNA_MUX_MCP_ENDPOINT='http://127.0.0.1:43128/mcp'\nLUNA_MUX_MCP_AUTHORIZATION='fixture-only'\n").unwrap();
        let shim = root.join("dst");
        // WSL loads exported values before boot; SSH sources its private file.
        for from_file in [false, true] {
            let source = if from_file {
                format!(
                    "set -a; . {}; set +a",
                    shell_quote(&environment.to_string_lossy())
                )
            } else {
                String::new()
            };
            fs::write(
                &shim,
                posix_script(
                    &shell_quote(&real.to_string_lossy()),
                    &root.join(PATCH_NAME).to_string_lossy(),
                    &shell_quote(&hook.to_string_lossy()),
                    &source,
                ),
            )
            .unwrap();
            for (arguments, interactive) in [
                (
                    vec!["--resume", "session with spaces", "literal $(false)"],
                    true,
                ),
                (vec!["doctor"], false),
            ] {
                fs::write(&hooks_file, "").unwrap();
                let mut command = Command::new("/bin/sh");
                command
                    .arg(&shim)
                    .args(&arguments)
                    .env("DSH_TEST_ARGS", &args_file)
                    .env("DSH_TEST_HOOKS", &hooks_file)
                    .env_remove("LUNA_MUX_MCP_ENDPOINT")
                    .env_remove("LUNA_MUX_MCP_AUTHORIZATION");
                if !from_file {
                    command
                        .env("LUNA_MUX_MCP_ENDPOINT", "http://127.0.0.1:43128/mcp")
                        .env("LUNA_MUX_MCP_AUTHORIZATION", "fixture-only");
                }
                assert_eq!(command.status().unwrap().code(), Some(17));
                let actual = fs::read_to_string(&args_file).unwrap();
                let mut expected = Vec::new();
                if interactive {
                    expected.extend([
                        "--patch".to_string(),
                        root.join(PATCH_NAME).to_string_lossy().into_owned(),
                        "--backend".into(),
                        "dsh".into(),
                    ]);
                }
                expected.extend(arguments.into_iter().map(str::to_string));
                assert_eq!(actual.lines().collect::<Vec<_>>(), expected);
                let events = fs::read_to_string(&hooks_file).unwrap();
                if interactive {
                    let events = events
                        .lines()
                        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
                        .collect::<Vec<_>>();
                    assert_eq!(events.len(), 2);
                    assert_eq!(events[0]["hook_event_name"], "AgentProcessStart");
                    assert_eq!(events[1]["hook_event_name"], "AgentProcessExit");
                    assert_eq!(
                        events[0]["luna_mux_process_id"],
                        events[1]["luna_mux_process_id"]
                    );
                } else {
                    assert!(events.is_empty());
                }
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    // Opt-in fixture export for exercising the real isolated upstream loader.
    #[test]
    #[ignore]
    fn export_upstream_smoke_fixture() {
        let directory =
            PathBuf::from(std::env::var_os("LUNA_MUX_DSH_FIXTURE").expect("fixture directory"));
        fs::create_dir_all(&directory).unwrap();
        let bridge = directory.join(BRIDGE_NAME);
        fs::write(&bridge, BRIDGE).unwrap();
        fs::write(
            directory.join(PATCH_NAME),
            patch(
                &bridge.to_string_lossy(),
                "unused-browser",
                false,
                &context(),
            ),
        )
        .unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn powershell_shim_preserves_arguments_exit_code_environment_and_cli_passthrough() {
        use std::process::Command;
        let c = context();
        let root = crate::agent_adapters::runtime_root(&c);
        fs::create_dir_all(&root).unwrap();
        let real = root.join("fake dsh.ps1");
        let hook = root.join("fake hook.ps1");
        fs::write(&real, "$args | ConvertTo-Json -Compress | Set-Content -LiteralPath $env:DSH_TEST_ARGS; @{nodeOptions=$env:NODE_OPTIONS; disableWin32Input=$env:DSH_TUI_DISABLE_WIN32_INPUT_MODE} | ConvertTo-Json -Compress | Set-Content -LiteralPath ($env:DSH_TEST_ARGS + '.env'); $global:LASTEXITCODE=17\n").unwrap();
        fs::write(
            &hook,
            "$input | Add-Content -LiteralPath $env:DSH_TEST_HOOKS; $global:LASTEXITCODE=0\n",
        )
        .unwrap();
        install_with_executable(&c, Some(&real), &hook).unwrap();
        let shim = runtime_shim_root(&c)
            .join("dsh-tui.ps1")
            .to_string_lossy()
            .replace('\'', "''");
        let args_file = root.join("args.json");
        let hooks_file = root.join("hooks.jsonl");
        let shells = [
            crate::local_pty_backend::windows_powershell5_executable(),
            crate::local_pty_backend::windows_powershell7_executable(),
        ];
        for shell in shells.into_iter().flatten() {
            let script = format!(
                "$env:LUNA_MUX_AGENT_PROCESS_ID='parent'; $env:LUNA_MUX_AGENT_ADAPTER='codex'; $env:NODE_OPTIONS='--stack-trace-limit=42'; $env:DSH_TUI_DISABLE_WIN32_INPUT_MODE='0'; & '{shim}' --resume 'session with spaces'; if ($LASTEXITCODE -ne 17 -or $env:LUNA_MUX_AGENT_PROCESS_ID -ne 'parent' -or $env:LUNA_MUX_AGENT_ADAPTER -ne 'codex' -or $env:NODE_OPTIONS -ne '--stack-trace-limit=42' -or $env:DSH_TUI_DISABLE_WIN32_INPUT_MODE -ne '0') {{ exit 1 }}; exit 0"
            );
            let result = Command::new(&shell)
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-Command",
                    &script,
                ])
                .env("DSH_TEST_ARGS", &args_file)
                .env("DSH_TEST_HOOKS", &hooks_file)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let args: Vec<String> = serde_json::from_str(
                &fs::read_to_string(&args_file)
                    .unwrap()
                    .trim_start_matches('\u{feff}'),
            )
            .unwrap();
            assert_eq!(args[0], "--patch");
            assert_eq!(
                &args[2..],
                ["--backend", "dsh", "--resume", "session with spaces"]
            );
            let environment: serde_json::Value = serde_json::from_str(
                fs::read_to_string(format!("{}.env", args_file.display()))
                    .unwrap()
                    .trim_start_matches('\u{feff}'),
            )
            .unwrap();
            assert_eq!(environment["disableWin32Input"], "1");
            let node_options = environment["nodeOptions"].as_str().unwrap();
            assert!(node_options.starts_with("--stack-trace-limit=42 --import=file:///"));
            assert!(node_options.ends_with("dsh-tui-terminal-compat.mjs"));
            let hooks = fs::read_to_string(&hooks_file).unwrap();
            assert!(hooks.contains("AgentProcessStart") && hooks.contains("AgentProcessExit"));
            fs::write(&hooks_file, "").unwrap();
            let result = Command::new(&shell)
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-Command",
                    &format!("& '{shim}' run -- 'hello world'; exit $LASTEXITCODE"),
                ])
                .env("DSH_TEST_ARGS", &args_file)
                .env("DSH_TEST_HOOKS", &hooks_file)
                .env("NODE_OPTIONS", "--stack-trace-limit=42")
                .env("DSH_TUI_DISABLE_WIN32_INPUT_MODE", "0")
                .output()
                .unwrap();
            assert_eq!(result.status.code(), Some(17));
            assert_eq!(fs::read_to_string(&hooks_file).unwrap(), "");
            let environment: serde_json::Value = serde_json::from_str(
                fs::read_to_string(format!("{}.env", args_file.display()))
                    .unwrap()
                    .trim_start_matches('\u{feff}'),
            )
            .unwrap();
            assert_eq!(environment["disableWin32Input"], "0");
            assert_eq!(environment["nodeOptions"], "--stack-trace-limit=42");
            for (tail, interactive) in [("--resume auth", true), ("migrate", false), ("-v", false)]
            {
                fs::write(&hooks_file, "").unwrap();
                let result = Command::new(&shell)
                    .args([
                        "-NoProfile",
                        "-NonInteractive",
                        "-ExecutionPolicy",
                        "Bypass",
                        "-Command",
                        &format!("& '{shim}' {tail}; exit $LASTEXITCODE"),
                    ])
                    .env("DSH_TEST_ARGS", &args_file)
                    .env("DSH_TEST_HOOKS", &hooks_file)
                    .output()
                    .unwrap();
                assert_eq!(result.status.code(), Some(17));
                let hooks = fs::read_to_string(&hooks_file).unwrap();
                assert_eq!(hooks.contains("AgentProcessStart"), interactive, "{tail}");
            }
        }
        crate::agent_adapters::cleanup(&c.runtime_id);
    }
}

pub fn posix_script(real: &str, patch: &str, hook: &str, environment: &str) -> String {
    format!(
        r#"#!/bin/sh
{environment}
case "${{1:-}}" in run|cli|dsh|session|models|skills|mcp|auth|capabilities|update|doctor|migrate|safe|rescue|help|version|--help|-h|--version|-v|-V|--print|-p|--dump-config|--dump-default-config|--dump-config-schema) {real} "$@"; exit $?;; esac
export LUNA_MUX_AGENT_ADAPTER=dsh-tui
export LUNA_MUX_AGENT_PROCESS_ID="$$-$(date +%s)"
export WSLENV="LUNA_MUX_AGENT_ADAPTER/w:LUNA_MUX_AGENT_PROCESS_ID/w${{WSLENV:+:$WSLENV}}"
printf '%s\n' "{{\"hook_event_name\":\"AgentProcessStart\",\"agent_adapter\":\"dsh-tui\",\"luna_mux_process_id\":\"$LUNA_MUX_AGENT_PROCESS_ID\"}}" | {hook} >/dev/null 2>&1 || true
{real} --patch {patch} --backend dsh "$@"
luna_mux_dsh_exit=$?
printf '%s\n' "{{\"hook_event_name\":\"AgentProcessExit\",\"agent_adapter\":\"dsh-tui\",\"luna_mux_process_id\":\"$LUNA_MUX_AGENT_PROCESS_ID\"}}" | {hook} >/dev/null 2>&1 || true
exit "$luna_mux_dsh_exit"
"#,
        patch = shell_quote(patch)
    )
}

#[cfg(windows)]
pub fn install_wsl_manual_bootstrap(
    context: &TerminalRuntimeContext,
    target: &str,
    environment: Option<&str>,
) -> Result<String, String> {
    let patch = write_support(context, target)?;
    let executable = crate::claude_code_adapter::executable_for_target(
        &std::env::current_exe().map_err(|e| e.to_string())?,
        target,
    )?;
    let env = environment
        .map(|path| crate::claude_code_adapter::executable_for_target(Path::new(path), target))
        .transpose()?;
    let env = env
        .map(|path| format!(". {}", shell_quote(&path.to_string_lossy())))
        .unwrap_or_default();
    let script = posix_script(
        "command dsh-tui",
        &patch.to_string_lossy(),
        &format!("{} hook", shell_quote(&executable.to_string_lossy())),
        &env,
    );
    let path = runtime_shim_root(context).join("dsh-tui-wsl.sh");
    fs::write(&path, script).map_err(|e| e.to_string())?;
    let path = crate::claude_code_adapter::executable_for_target(&path, target)?;
    Ok(format!(
        "dsh-tui() ( sh {} \"$@\" ); dst() {{ dsh-tui \"$@\"; }}",
        shell_quote(&path.to_string_lossy())
    ))
}
