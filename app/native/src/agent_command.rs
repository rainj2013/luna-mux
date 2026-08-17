use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use uuid::Uuid;

use crate::shell_quoting::posix_shell_quote;

const OUTPUT_LIMIT: u64 = 64 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_secs(4);
const PATH_MARKER: &str = "__LUNA_MUX_AGENT_COMMAND__";

#[derive(Debug, Default)]
pub(crate) struct CommandDiscovery {
    pub paths: BTreeMap<String, PathBuf>,
    pub warning: Option<String>,
}

pub(crate) fn discover(commands: &[&str], target_id: &str) -> CommandDiscovery {
    let mut result = match discover_in_target(commands, target_id) {
        Ok(paths) => CommandDiscovery {
            paths,
            warning: None,
        },
        Err(error) => CommandDiscovery {
            paths: BTreeMap::new(),
            warning: Some(error),
        },
    };

    if !target_id.starts_with("local:wsl:") {
        for command in commands {
            if !result.paths.contains_key(*command)
                && let Some(path) = find_in_path(command, std::env::var_os("PATH").as_deref())
            {
                result.paths.insert((*command).to_string(), path);
            }
        }
    }
    result
}

pub(crate) fn default_local_target_ids() -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        return vec!["local:macos-shell".into()];
    }
    #[cfg(windows)]
    {
        let mut targets = Vec::new();
        if crate::local_pty_backend::windows_powershell7_executable().is_some() {
            targets.push("local:powershell".into());
        }
        if crate::local_pty_backend::windows_powershell5_executable().is_some() {
            targets.push("local:powershell5".into());
        }
        return targets;
    }
    #[allow(unreachable_code)]
    Vec::new()
}

fn discover_in_target(
    commands: &[&str],
    target_id: &str,
) -> Result<BTreeMap<String, PathBuf>, String> {
    validate_commands(commands)?;

    #[cfg(target_os = "macos")]
    if target_id == "local:macos-shell" {
        let shell = crate::local_pty_backend::macos_supported_shell()
            .ok_or_else(|| "no supported macOS login shell was found".to_string())?;
        return run_posix_probe(Path::new(&shell), commands, None);
    }

    #[cfg(windows)]
    {
        if crate::local_pty_backend::is_powershell_target(target_id) {
            return run_powershell_probe(commands, target_id);
        }
        if let Some(distribution) = target_id.strip_prefix("local:wsl:") {
            return run_wsl_probe(commands, distribution);
        }
    }

    Err(format!("unsupported local Agent target: {target_id}"))
}

fn validate_commands(commands: &[&str]) -> Result<(), String> {
    for command in commands {
        if command.is_empty()
            || !command
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(format!("invalid Agent command name: {command}"));
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_posix_probe(
    shell: &Path,
    commands: &[&str],
    environment: Option<&[(OsString, OsString)]>,
) -> Result<BTreeMap<String, PathBuf>, String> {
    let shell_name = shell
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default();
    let script = posix_probe_script(commands, shell_name);
    let mut command = Command::new(shell);
    command.args(["-lic", &script]);
    if let Some(environment) = environment {
        command.env_clear();
        command.envs(environment.iter().cloned());
    }
    let output = run_bounded(command, PROBE_TIMEOUT)?;
    parse_probe_output(
        commands,
        &output.stdout,
        true,
        output.success,
        &output.stderr,
    )
}

#[cfg(windows)]
fn run_powershell_probe(
    commands: &[&str],
    target_id: &str,
) -> Result<BTreeMap<String, PathBuf>, String> {
    let powershell = if target_id == "local:powershell5" {
        crate::local_pty_backend::windows_powershell5_executable()
            .ok_or_else(|| "Windows PowerShell 5.1 is unavailable".to_string())?
    } else {
        crate::local_pty_backend::windows_powershell7_executable()
            .ok_or_else(|| "PowerShell 7 is unavailable".to_string())?
    };
    let mut script = powershell_profile_load_script(target_id == "local:powershell5");
    for name in commands {
        let quoted = crate::shell_quoting::powershell_quote(name);
        script.push_str(&format!(
            "$lunaMuxCommand = Get-Command -Name {quoted} -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1; if ($null -ne $lunaMuxCommand) {{ [Console]::Out.WriteLine('{PATH_MARKER}{name}=' + $lunaMuxCommand.Source) }}\r\n"
        ));
    }
    let mut command = Command::new(powershell);
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &script,
    ]);
    let output = run_bounded(command, PROBE_TIMEOUT)?;
    parse_probe_output(
        commands,
        &output.stdout,
        true,
        output.success,
        &output.stderr,
    )
}

#[cfg(windows)]
fn run_wsl_probe(
    commands: &[&str],
    distribution: &str,
) -> Result<BTreeMap<String, PathBuf>, String> {
    if distribution.trim().is_empty() {
        return Err("WSL distribution name is empty".into());
    }
    let inner = posix_probe_script(commands, "");
    let outer = format!(
        "exec \"${{SHELL:-/bin/sh}}\" -lic {}",
        posix_shell_quote(&inner)
    );
    let mut command = Command::new("wsl.exe");
    command.args(["--distribution", distribution, "--", "sh", "-lc", &outer]);
    let output = run_bounded(command, PROBE_TIMEOUT)?;
    parse_probe_output(
        commands,
        &output.stdout,
        false,
        output.success,
        &output.stderr,
    )
}

fn posix_probe_script(commands: &[&str], shell_name: &str) -> String {
    let mut script = String::new();
    for name in commands {
        let quoted = posix_shell_quote(name);
        let resolver = match shell_name {
            "zsh" => format!("whence -p -- {quoted}"),
            "bash" => format!("type -P -- {quoted}"),
            _ => format!("command -v -- {quoted}"),
        };
        script.push_str(&format!(
            "luna_mux_command_path=$({resolver} 2>/dev/null); if [ -n \"$luna_mux_command_path\" ]; then printf '%s%s\\n' '{PATH_MARKER}{name}=' \"$luna_mux_command_path\"; fi\n"
        ));
    }
    script
}

#[cfg(windows)]
pub(crate) fn powershell_profile_load_script(legacy_console: bool) -> String {
    let encoding = if legacy_console {
        "$OutputEncoding = [Console]::OutputEncoding = [Console]::InputEncoding = [System.Text.UTF8Encoding]::new($false)\r\n& chcp.com 65001 | Out-Null\r\n"
    } else {
        ""
    };
    format!(
        "{encoding}$lunaMuxProfilePaths = @($PROFILE.AllUsersAllHosts, $PROFILE.AllUsersCurrentHost, $PROFILE.CurrentUserAllHosts, $PROFILE.CurrentUserCurrentHost)\r\n\
foreach ($lunaMuxProfilePath in $lunaMuxProfilePaths) {{ if (Test-Path -LiteralPath $lunaMuxProfilePath) {{ . $lunaMuxProfilePath }} }}\r\n\
Remove-Variable lunaMuxProfilePaths,lunaMuxProfilePath -ErrorAction SilentlyContinue\r\n"
    )
}

fn parse_probe_output(
    commands: &[&str],
    stdout: &str,
    require_local_file: bool,
    success: bool,
    stderr: &str,
) -> Result<BTreeMap<String, PathBuf>, String> {
    let mut paths = BTreeMap::new();
    for name in commands {
        let prefix = format!("{PATH_MARKER}{name}=");
        let path = stdout
            .lines()
            .rev()
            .find_map(|line| line.trim().strip_prefix(&prefix))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        if let Some(path) = path
            && (!require_local_file || path.is_file())
        {
            paths.insert((*name).to_string(), path);
        }
    }
    if success || !paths.is_empty() {
        return Ok(paths);
    }
    let detail = stderr.trim();
    Err(if detail.is_empty() {
        "target shell exited before reporting its Agent PATH".into()
    } else {
        format!("target shell probe failed: {detail}")
    })
}

fn find_in_path(command: &str, path: Option<&OsStr>) -> Option<PathBuf> {
    let path = path?;
    for directory in std::env::split_paths(path) {
        #[cfg(windows)]
        let names = windows_command_names(command);
        #[cfg(not(windows))]
        let names = vec![command.to_string()];
        for name in names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(windows)]
fn windows_command_names(command: &str) -> Vec<String> {
    if Path::new(command).extension().is_some() {
        vec![command.to_string()]
    } else {
        [".exe", ".cmd", ".bat", ".com", ""]
            .into_iter()
            .map(|extension| format!("{command}{extension}"))
            .collect()
    }
}

struct BoundedOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

fn run_bounded(mut command: Command, timeout: Duration) -> Result<BoundedOutput, String> {
    let root = std::env::temp_dir()
        .join("luna-mux")
        .join("command-discovery");
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let id = Uuid::new_v4().simple().to_string();
    let stdout_path = root.join(format!("{id}.stdout"));
    let stderr_path = root.join(format!("{id}.stderr"));
    let stdout = File::create(&stdout_path).map_err(|error| error.to_string())?;
    let stderr = File::create(&stderr_path).map_err(|error| error.to_string())?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }

    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let deadline = Instant::now() + timeout;
    let success = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = fs::remove_file(&stdout_path);
                let _ = fs::remove_file(&stderr_path);
                return Err(format!(
                    "target shell probe timed out after {} seconds",
                    timeout.as_secs()
                ));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = fs::remove_file(&stdout_path);
                let _ = fs::remove_file(&stderr_path);
                return Err(error.to_string());
            }
        }
    };

    let stdout = read_tail(&stdout_path).unwrap_or_default();
    let stderr = read_tail(&stderr_path).unwrap_or_default();
    let _ = fs::remove_file(stdout_path);
    let _ = fs::remove_file(stderr_path);
    Ok(BoundedOutput {
        success,
        stdout,
        stderr,
    })
}

fn read_tail(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let length = file.metadata().map_err(|error| error.to_string())?.len();
    if length > OUTPUT_LIMIT {
        file.seek(SeekFrom::Start(length - OUTPUT_LIMIT))
            .map_err(|error| error.to_string())?;
    }
    let mut bytes = Vec::with_capacity(length.min(OUTPUT_LIMIT) as usize);
    file.take(OUTPUT_LIMIT)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_ignores_profile_noise_and_uses_the_last_marker() {
        let root = std::env::temp_dir();
        let known = root.join("known-agent");
        fs::write(&known, "test").unwrap();
        let output = format!(
            "profile output\n{PATH_MARKER}codex=/missing\n{PATH_MARKER}codex={}\n",
            known.display()
        );
        let paths = parse_probe_output(&["codex"], &output, true, true, "").unwrap();
        assert_eq!(paths.get("codex"), Some(&known));
        let _ = fs::remove_file(known);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_probe_loads_profile_with_a_minimal_gui_path() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "luna-mux-agent-path-test-{}",
            Uuid::new_v4().simple()
        ));
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let agent = bin.join("codex");
        fs::write(&agent, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&agent, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(
            root.join(".zprofile"),
            format!(
                "export PATH={}:$PATH\n",
                posix_shell_quote(&bin.to_string_lossy())
            ),
        )
        .unwrap();
        let environment = vec![
            (OsString::from("HOME"), root.clone().into_os_string()),
            (OsString::from("ZDOTDIR"), root.clone().into_os_string()),
            (OsString::from("PATH"), OsString::from("/usr/bin:/bin")),
            (OsString::from("SHELL"), OsString::from("/bin/zsh")),
        ];
        let paths = run_posix_probe(Path::new("/bin/zsh"), &["codex"], Some(&environment))
            .expect("probe login zsh");
        assert_eq!(paths.get("codex"), Some(&agent));
        let _ = fs::remove_dir_all(root);
    }
}
