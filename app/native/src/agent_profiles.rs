use serde::{Deserialize, Serialize};

pub const AGENT_LAUNCH_SETTINGS_KEY: &str = "agentLaunchSettings";

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct AgentLaunchSettings {
    pub codex_yolo: bool,
    pub claude_code_yolo: bool,
}

impl AgentLaunchSettings {
    fn flag(&self, adapter: &str) -> Option<&'static str> {
        match adapter {
            "codex" if self.codex_yolo => Some("--yolo"),
            "claude-code" if self.claude_code_yolo => Some("--dangerously-skip-permissions"),
            _ => None,
        }
    }

    pub fn apply(&self, adapter: &str, command: String) -> String {
        match self.flag(adapter) {
            Some(flag) => format!("{command} {flag}"),
            None => command,
        }
    }

    /// A generated launch artifact, rebuilt from the database at app startup
    /// and on save. Local/WSL shims read it on each invocation.
    pub fn write_launch_defaults(&self) -> Result<(), String> {
        let id = format!("launch-defaults-{}", std::process::id());
        crate::runtime_env::write_runtime_owner(&id)?;
        let contents = ["codex", "claude-code"]
            .into_iter()
            .filter(|adapter| self.flag(adapter).is_some())
            .map(|adapter| format!("{adapter}\n"))
            .collect::<String>();
        write_launch_defaults_at(&launch_defaults_path(), &contents)
    }
}

fn write_launch_defaults_at(path: &std::path::Path, contents: &str) -> Result<(), String> {
    use std::io::Write;
    let temporary = path.with_extension(uuid::Uuid::new_v4().to_string());
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(contents.as_bytes())?;
        drop(file);
        // Rust's rename replaces the destination on both Windows and Unix.
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result.map_err(|error: std::io::Error| error.to_string())
}

pub fn launch_defaults_path() -> std::path::PathBuf {
    std::env::temp_dir()
        .join("luna-mux")
        .join(format!("launch-defaults-{}", std::process::id()))
        .join("enabled")
}

// Explicit permission arguments win; utility names are checked only at the
// command position, never in option values or after the -- prompt boundary.
fn excluded_launch_arguments(adapter: &str) -> &'static [&'static str] {
    match adapter {
        "codex" => &[
            "--help",
            "-h",
            "--version",
            "-V",
            "--yolo",
            "--dangerously-bypass-approvals-and-sandbox",
            "--full-auto",
            "--approve-for-me",
            "--sandbox",
            "--sandbox=*",
            "-s*",
            "--ask-for-approval",
            "--ask-for-approval=*",
            "-a*",
            "--config",
            "--config=*",
            "-c*",
        ],
        "claude-code" => &[
            "--help",
            "-h",
            "--version",
            "-v",
            "--permission-mode",
            "--permission-mode=*",
            "--restricted",
            "--permission-prompts",
            "--permission-prompts=*",
            "--dangerously-skip-permissions",
            "--allow-dangerously-skip-permissions",
            "--settings",
            "--settings=*",
        ],
        _ => &[],
    }
}

fn utility_commands(adapter: &str) -> &'static [&'static str] {
    match adapter {
        "codex" => &[
            "help",
            "login",
            "logout",
            "mcp",
            "mcp-server",
            "app",
            "app-server",
            "completion",
            "debug",
            "exec",
            "e",
            "review",
            "sandbox",
            "apply",
            "a",
            "cloud",
            "features",
            "agents",
            "plugin",
            "remote-control",
            "update",
            "doctor",
            "queue",
            "archive",
            "delete",
            "migrate-rollouts",
            "migrate-nfs-db",
            "unarchive",
            "exec-server",
        ],
        "claude-code" => &[
            "help",
            "auth",
            "login",
            "logout",
            "mcp",
            "doctor",
            "install",
            "update",
            "upgrade",
            "setup-token",
            "plugin",
            "plugins",
            "agents",
            "attach",
            "auto-mode",
            "gateway",
            "import",
            "logs",
            "purge",
            "respawn",
            "rm",
            "stop",
            "kill",
            "ultrareview",
        ],
        _ => &[],
    }
}

fn valued_launch_options(adapter: &str) -> &'static [&'static str] {
    match adapter {
        "codex" => &[
            "--model",
            "-m",
            "--profile",
            "-p",
            "--cd",
            "-C",
            "--add-dir",
            "--image",
            "-i",
            "--enable",
            "--disable",
            "--remote",
            "--remote-auth-token-env",
        ],
        "claude-code" => &[
            "--model",
            "--name",
            "-n",
            "--resume",
            "-r",
            "--agent",
            "--agents",
            "--mcp-config",
            "--append-system-prompt",
            "--system-prompt",
            "--tools",
            "--allowedTools",
            "--disallowedTools",
            "--output-format",
            "--input-format",
            "--add-dir",
            "--session-id",
            "--permission-prompt-tool",
            "--effort",
            "--debug-file",
        ],
        _ => &[],
    }
}

fn flag_launch_options(adapter: &str) -> &'static [&'static str] {
    match adapter {
        "codex" => &["--search", "--no-alt-screen", "--oss", "--last", "--all"],
        "claude-code" => &[
            "--continue",
            "-c",
            "--fork-session",
            "--verbose",
            "--chrome",
            "--no-chrome",
            "--ide",
            "--strict-mcp-config",
            "--no-session-persistence",
        ],
        _ => &[],
    }
}

fn inline_launch_options(adapter: &str) -> Vec<String> {
    valued_launch_options(adapter)
        .iter()
        .map(|option| {
            // Long --option=value and short -mVALUE already contain their value.
            if option.starts_with("--") {
                format!("{option}=*")
            } else {
                format!("{option}*")
            }
        })
        .chain(
            flag_launch_options(adapter)
                .iter()
                .map(|option| (*option).to_string()),
        )
        .collect()
}

/// Prepend a default flag to positional parameters without evaluating input.
/// Remote shells receive a snapshot; native/WSL shells read the generated file.
pub fn posix_launch_defaults(adapter: &str, enabled: &str) -> String {
    let flag = match adapter {
        "codex" => "--yolo",
        "claude-code" => "--dangerously-skip-permissions",
        _ => return String::new(),
    };
    let patterns = excluded_launch_arguments(adapter).join("|");
    let utilities = utility_commands(adapter).join("|");
    let values = valued_launch_options(adapter).join("|");
    let flags = inline_launch_options(adapter).join("|");
    let print = if adapter == "claude-code" {
        "--print|-p) luna_mux_command_position=0 ;;"
    } else {
        ""
    };
    format!(
        r#"luna_mux_use_yolo=1
luna_mux_skip_value=0
luna_mux_command_position=1
for luna_mux_arg in "$@"; do
  if [ "$luna_mux_skip_value" = 1 ]; then
    luna_mux_skip_value=0
    case "$luna_mux_arg" in -*) ;; *) continue ;; esac
  fi
  case "$luna_mux_arg" in
    --) break ;;
    {patterns}) luna_mux_use_yolo=0 ;;
    {values}) luna_mux_skip_value=1 ;;
    {print}
    {flags}) ;;
    -*) luna_mux_use_yolo=0 ;;
    *)
      if [ "$luna_mux_command_position" = 1 ]; then
        case "$luna_mux_arg" in {utilities}) luna_mux_use_yolo=0 ;; esac
      fi
      luna_mux_command_position=0 ;;
  esac
done
if [ "$luna_mux_use_yolo" = 1 ] && {enabled}; then set -- {flag} "$@"; fi
"#
    )
}

pub fn local_posix_launch_defaults(adapter: &str, target_id: &str) -> Result<String, String> {
    let path = crate::codex_shim::hook_executable_for_target(&launch_defaults_path(), target_id)?;
    Ok(posix_launch_defaults(
        adapter,
        &format!(
            "grep -Fxq -- {} {} 2>/dev/null",
            crate::shell_quoting::shell_quote(adapter),
            crate::shell_quoting::shell_quote(&path)
        ),
    ))
}

#[cfg(windows)]
pub fn powershell_launch_defaults(adapter: &str) -> String {
    powershell_launch_defaults_at(adapter, &launch_defaults_path())
}

#[cfg(windows)]
fn powershell_launch_defaults_at(adapter: &str, settings_path: &std::path::Path) -> String {
    let flag = match adapter {
        "codex" => "--yolo",
        "claude-code" => "--dangerously-skip-permissions",
        _ => return String::new(),
    };
    let path = crate::shell_quoting::powershell_quote(&settings_path.to_string_lossy());
    let cases = excluded_launch_arguments(adapter)
        .iter()
        .map(|pattern| format!("'{pattern}' {{ $lunaMuxUseYolo = $false; break }}\n"))
        .collect::<String>();
    let values = valued_launch_options(adapter)
        .iter()
        .map(|option| format!("'{option}' {{ $lunaMuxSkipValue = $true; break }}\n"))
        .collect::<String>();
    let utilities = utility_commands(adapter)
        .iter()
        .map(|name| format!("'{name}'"))
        .collect::<Vec<_>>()
        .join(",");
    let flags = inline_launch_options(adapter)
        .iter()
        .map(|option| format!("'{option}' {{ break }}\n"))
        .collect::<String>();
    let print = if adapter == "claude-code" {
        "'--print' { $lunaMuxCommandPosition = $false; break }\n'-p' { $lunaMuxCommandPosition = $false; break }"
    } else {
        ""
    };
    format!(
        r#"$lunaMuxDefaultArguments = @()
$lunaMuxUseYolo = @(Get-Content -LiteralPath {path} -ErrorAction SilentlyContinue) -contains '{adapter}'
$lunaMuxSkipValue = $false
$lunaMuxCommandPosition = $true
foreach ($lunaMuxArg in $args) {{
  $lunaMuxArg = [string]$lunaMuxArg
  if ($lunaMuxSkipValue) {{
    $lunaMuxSkipValue = $false
    if (!$lunaMuxArg.StartsWith('-')) {{ continue }}
  }}
  if ($lunaMuxArg -ceq '--') {{ break }}
  switch -Wildcard -CaseSensitive ($lunaMuxArg) {{
    {cases}
    {values}
    {print}
    {flags}
    '-*' {{ $lunaMuxUseYolo = $false; break }}
    default {{
      if ($lunaMuxCommandPosition -and @({utilities}) -ccontains $lunaMuxArg) {{ $lunaMuxUseYolo = $false }}
      $lunaMuxCommandPosition = $false
    }}
  }}
}}
if ($lunaMuxUseYolo) {{ $lunaMuxDefaultArguments = @('{flag}') }}
"#
    )
}

pub const DEFAULT_CODEX_PROFILE_ID: &str = "codex.default";
pub const DEFAULT_CLAUDE_CODE_PROFILE_ID: &str = "claude-code.default";
pub const DEFAULT_GROK_BUILD_PROFILE_ID: &str = "grok-build.default";
pub const DEFAULT_DSH_TUI_PROFILE_ID: &str = "dsh-tui.default";

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

    fn permission_cases(adapter: &str) -> Vec<(Vec<&'static str>, bool)> {
        let mut cases = vec![
            (vec![], true),
            (vec!["hello with spaces and 'quotes'"], true),
            (vec!["--help"], false),
            (vec!["mcp", "list"], false),
        ];
        cases.extend(if adapter == "codex" {
            vec![
                (vec!["resume", "--last"], true),
                (vec!["--sandbox", "read-only"], false),
                (vec!["--sandbox=read-only"], false),
                (vec!["-sread-only"], false),
                (vec!["--ask-for-approval=on-request"], false),
                (vec!["--approve-for-me"], false),
                (vec!["--", "help"], true),
                (vec!["--model", "help"], true),
                (vec!["--model", "test", "mcp", "list"], false),
                (
                    vec!["--remote", "ws://127.0.0.1:1234", "mcp", "list"],
                    false,
                ),
                (vec!["--future-option", "test", "mcp", "list"], false),
                (vec!["--model=test", "hello"], true),
                (vec!["agents", "list"], false),
                (vec!["doctor"], false),
                (vec!["--config", "approval_policy=\"on-request\""], false),
                (vec!["--yolo"], false),
                (vec!["login"], false),
            ]
        } else {
            vec![
                (vec!["--resume", "session-1"], true),
                (vec!["--permission-mode", "plan"], false),
                (vec!["--permission-mode=manual"], false),
                (vec!["--restricted"], false),
                (vec!["--permission-prompts", "none"], false),
                (vec!["--print", "help"], true),
                (vec!["--name", "logs"], true),
                (vec!["--effort", "high", "auth", "status"], false),
                (vec!["--continue"], true),
                (vec!["--resume", "--permission-mode", "plan"], false),
                (vec!["--", "--restricted"], true),
                (vec!["--dangerously-skip-permissions"], false),
                (vec!["auth", "status"], false),
                (vec!["agents", "list"], false),
            ]
        });
        cases
    }

    #[test]
    #[cfg(windows)]
    fn manual_yolo_arguments_work_in_powershell_5_and_7() {
        use std::{
            fs,
            process::{Command, Stdio},
        };
        let root = std::env::temp_dir().join(format!("yolo-argv-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let defaults = root.join("launch defaults 'quoted'");
        let script_path = root.join("verify.ps1");
        let output_path = root.join("arguments.jsonl");
        let mut script = String::new();
        let mut expected = Vec::<Vec<String>>::new();
        for adapter in ["codex", "claude-code"] {
            script.push_str(&format!("function Test-Launch {{\n{}ConvertTo-Json -InputObject @(@($lunaMuxDefaultArguments) + @($args)) -Compress | Add-Content -Encoding UTF8 -LiteralPath {}\n}}\n", powershell_launch_defaults_at(adapter, &defaults), crate::shell_quoting::powershell_quote(&output_path.to_string_lossy())));
            for enabled in [false, true] {
                script.push_str(&format!(
                    "[System.IO.File]::WriteAllText({}, {})\n",
                    crate::shell_quoting::powershell_quote(&defaults.to_string_lossy()),
                    crate::shell_quoting::powershell_quote(if enabled { adapter } else { "" })
                ));
                for (arguments, inherits) in permission_cases(adapter) {
                    let quoted = arguments
                        .iter()
                        .map(|arg| crate::shell_quoting::powershell_quote(arg))
                        .collect::<Vec<_>>()
                        .join(",");
                    script.push_str(&format!(
                        "$testArguments = @({quoted}); Test-Launch @testArguments\n"
                    ));
                    let mut argv = arguments.into_iter().map(str::to_owned).collect::<Vec<_>>();
                    if enabled && inherits {
                        argv.insert(
                            0,
                            if adapter == "codex" {
                                "--yolo"
                            } else {
                                "--dangerously-skip-permissions"
                            }
                            .into(),
                        );
                    }
                    expected.push(argv);
                }
            }
        }
        fs::write(&script_path, script).unwrap();
        let shells = [
            crate::local_pty_backend::windows_powershell5_executable(),
            crate::local_pty_backend::windows_powershell7_executable(),
        ];
        assert!(shells.iter().any(Option::is_some));
        for shell in shells.into_iter().flatten() {
            let _ = fs::remove_file(&output_path);
            let status = Command::new(&shell)
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                ])
                .arg(&script_path)
                .stdin(Stdio::null())
                .status()
                .unwrap();
            assert!(status.success(), "{shell}: {status}");
            let contents = fs::read_to_string(&output_path).unwrap();
            let actual = contents
                .trim_start_matches('\u{feff}')
                .lines()
                .map(|line| serde_json::from_str::<Vec<String>>(line).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "{shell}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn manual_yolo_arguments_work_in_posix_shell() {
        use std::{
            fs,
            process::{Command, Stdio},
        };
        #[cfg(windows)]
        let shell = std::path::PathBuf::from("C:/Program Files/Git/usr/bin/sh.exe");
        #[cfg(not(windows))]
        let shell = std::path::PathBuf::from("/bin/sh");
        if !shell.is_file() {
            return;
        }
        let root = std::env::temp_dir().join(format!("yolo-posix-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let script_path = root.join("verify.sh");
        let output_path = root.join("arguments.txt");
        let mut script = String::new();
        let mut expected = String::new();
        for adapter in ["codex", "claude-code"] {
            for enabled in [false, true] {
                script.push_str(&format!("test_launch() {{\n{}printf '%s\\n' BEGIN\nfor arg in \"$@\"; do printf '%s\\n' \"$arg\"; done\n}}\n", posix_launch_defaults(adapter, if enabled { "true" } else { "false" })));
                for (arguments, inherits) in permission_cases(adapter) {
                    script.push_str(&format!(
                        "test_launch {}\n",
                        arguments
                            .iter()
                            .map(|arg| crate::shell_quoting::shell_quote(arg))
                            .collect::<Vec<_>>()
                            .join(" ")
                    ));
                    expected.push_str("BEGIN\n");
                    if enabled && inherits {
                        expected.push_str(if adapter == "codex" {
                            "--yolo\n"
                        } else {
                            "--dangerously-skip-permissions\n"
                        });
                    }
                    for arg in arguments {
                        expected.push_str(arg);
                        expected.push('\n');
                    }
                }
            }
        }
        fs::write(&script_path, script).unwrap();
        let status = Command::new(shell)
            .arg(&script_path)
            .stdin(Stdio::null())
            .stdout(fs::File::create(&output_path).unwrap())
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(
            fs::read_to_string(output_path)
                .unwrap()
                .replace("\r\n", "\n"),
            expected
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn launch_defaults_replace_complete_files_and_preserve_previous_on_failure() {
        let root = std::env::temp_dir().join(format!("launch-artifact-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("enabled");
        write_launch_defaults_at(&path, "codex\n").unwrap();
        write_launch_defaults_at(&path, "claude-code\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "claude-code\n");
        let directory = root.join("directory");
        std::fs::create_dir(&directory).unwrap();
        assert!(write_launch_defaults_at(&directory, "codex\n").is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "claude-code\n");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn launch_settings_preserve_old_defaults_and_persist_independently() {
        let path = std::env::temp_dir().join(format!("agent-launch-{}.db", uuid::Uuid::new_v4()));
        let db = crate::database::Database::open(&path, "agent-launch-test").unwrap();
        assert_eq!(
            db.get_setting(AGENT_LAUNCH_SETTINGS_KEY, AgentLaunchSettings::default()),
            AgentLaunchSettings::default()
        );
        let partial: AgentLaunchSettings = serde_json::from_str(r#"{"codexYolo":true}"#).unwrap();
        assert!(partial.codex_yolo);
        assert!(!partial.claude_code_yolo);
        db.set_setting(AGENT_LAUNCH_SETTINGS_KEY, &partial).unwrap();
        drop(db);
        let db = crate::database::Database::open(&path, "agent-launch-test").unwrap();
        assert_eq!(
            db.get_setting(AGENT_LAUNCH_SETTINGS_KEY, AgentLaunchSettings::default()),
            partial
        );
        let both = AgentLaunchSettings {
            codex_yolo: true,
            claude_code_yolo: true,
        };
        db.set_setting(AGENT_LAUNCH_SETTINGS_KEY, &both).unwrap();
        db.set_setting(AGENT_LAUNCH_SETTINGS_KEY, &AgentLaunchSettings::default())
            .unwrap();
        assert_eq!(
            db.get_setting(AGENT_LAUNCH_SETTINGS_KEY, both),
            AgentLaunchSettings::default()
        );
        drop(db);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn launch_settings_only_add_the_selected_adapters_permission_flag() {
        for settings in [
            AgentLaunchSettings::default(),
            AgentLaunchSettings {
                codex_yolo: true,
                claude_code_yolo: false,
            },
            AgentLaunchSettings {
                codex_yolo: false,
                claude_code_yolo: true,
            },
            AgentLaunchSettings {
                codex_yolo: true,
                claude_code_yolo: true,
            },
        ] {
            for profile in profiles() {
                let command = settings.apply(&profile.adapter, profile.command.clone());
                assert_eq!(
                    command.contains(" --yolo"),
                    profile.adapter == "codex" && settings.codex_yolo
                );
                assert_eq!(
                    command.contains(" --dangerously-skip-permissions"),
                    profile.adapter == "claude-code" && settings.claude_code_yolo
                );
                if profile.adapter == "dsh-tui" || profile.adapter == "grok-build" {
                    assert_eq!(command, profile.command);
                }
            }
        }
    }

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
        assert_eq!(
            resolve(DEFAULT_GROK_BUILD_PROFILE_ID).unwrap().adapter,
            "grok-build"
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
