use std::{
    collections::BTreeMap,
    fs,
    net::{SocketAddr, TcpStream},
    path::Path,
    time::Duration,
};

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCheckReport {
    pub ok: bool,
    pub checks: Vec<AgentCheck>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub managed_agents: Vec<DoctorManagedAgent>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentCheck {
    pub name: String,
    pub status: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorManagedAgent {
    pub agent_id: String,
    pub adapter: String,
    pub runtime_id: String,
    pub pane_id: String,
    pub pane_title: String,
    pub mux_session_id: String,
    pub session_name: String,
    pub status: String,
    pub last_activity: Option<String>,
}

pub fn try_run_agent_check(args: &[String]) -> Option<i32> {
    let subcommand = args.get(1).map(String::as_str)?;
    if subcommand != "agent-check" && subcommand != "doctor" {
        return None;
    }
    let filter = args.get(2).map(String::as_str);
    let report = run_report(filter);
    println!(
        "{}",
        serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".into())
    );
    Some(if report.ok { 0 } else { 1 })
}

pub fn run_report(filter: Option<&str>) -> AgentCheckReport {
    run_report_with_agents(filter, &[])
}

pub fn run_report_with_agents(
    filter: Option<&str>,
    managed_agents: &[DoctorManagedAgent],
) -> AgentCheckReport {
    let checks = run(filter, managed_agents);
    let ok = checks.iter().all(|check| check.status != "error");
    let managed_agents = managed_agents
        .iter()
        .filter(|agent| managed_agent_matches_filter(agent, filter))
        .cloned()
        .collect::<Vec<_>>();
    AgentCheckReport { ok, checks, managed_agents }
}

fn run(filter: Option<&str>, managed_agents: &[DoctorManagedAgent]) -> Vec<AgentCheck> {
    let mut checks = Vec::new();
    checks.push(check_executable());
    checks.push(check_local_agents());
    checks.push(check_runtime_environment_files(filter, managed_agents));
    checks.push(check_managed_agents(filter, managed_agents));
    #[cfg(windows)]
    checks.push(check_wsl_distributions(filter));
    checks
}

fn check_executable() -> AgentCheck {
    match std::env::current_exe() {
        Ok(path) => AgentCheck {
            name: "executable".into(),
            status: "ok".into(),
            detail: path.to_string_lossy().into_owned(),
        },
        Err(error) => AgentCheck {
            name: "executable".into(),
            status: "error".into(),
            detail: error.to_string(),
        },
    }
}

fn check_local_agents() -> AgentCheck {
    let mut available = Vec::new();
    let mut warnings = Vec::new();
    let targets = crate::agent_command::default_local_target_ids();
    for target_id in &targets {
        let discovery = crate::agent_command::discover(&["codex", "claude"], target_id);
        for (command, path) in discovery.paths {
            available.push(format!("{command}[{target_id}]={}", path.to_string_lossy()));
        }
        if let Some(warning) = discovery.warning {
            warnings.push(format!("{target_id}: {warning}"));
        }
    }
    if available.is_empty() {
        AgentCheck {
            name: "local_agents".into(),
            status: "warn".into(),
            detail: if warnings.is_empty() {
                "no agent executables found in local terminal environments".into()
            } else {
                format!("no agent executables found; {}", warnings.join(" | "))
            },
        }
    } else {
        AgentCheck {
            name: "local_agents".into(),
            status: "ok".into(),
            detail: available.join("; "),
        }
    }
}

fn check_runtime_environment_files(
    filter: Option<&str>,
    managed_agents: &[DoctorManagedAgent],
) -> AgentCheck {
    let root = std::env::temp_dir().join("luna-mux");
    let Ok(entries) = fs::read_dir(&root) else {
        return if managed_agents
            .iter()
            .any(|agent| managed_agent_matches_filter(agent, filter))
        {
            AgentCheck {
                name: "runtime_env_files".into(),
                status: "ok".into(),
                detail: "active managed agents are present; persistent environment files are not required for managed agents".into(),
            }
        } else {
            AgentCheck {
                name: "runtime_env_files".into(),
                status: "warn".into(),
                detail: "no luna-mux temp directory".into(),
            }
        };
    };
    let mut details = Vec::new();
    let mut errors = Vec::new();
    let mut found = false;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(runtime_id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if filter.is_some_and(|filter| !runtime_id.contains(filter)) {
            continue;
        }
        for file_name in ["agent-env.sh", "agent-env.ps1"] {
            let file = path.join(file_name);
            if !file.is_file() {
                continue;
            }
            found = true;
            match inspect_environment_file(&file) {
                Ok(detail) => details.push(format!("{runtime_id}: {detail}")),
                Err(error) => {
                    errors.push(format!("{runtime_id}/{file_name}: {error}"));
                }
            }
        }
    }
    if !found {
        return if managed_agents
            .iter()
            .any(|agent| managed_agent_matches_filter(agent, filter))
        {
            AgentCheck {
                name: "runtime_env_files".into(),
                status: "ok".into(),
                detail: "no persistent environment files found; active managed agents receive hook/MCP configuration through their launch environment instead".into(),
            }
        } else {
            AgentCheck {
                name: "runtime_env_files".into(),
                status: "warn".into(),
                detail: "no persistent runtime environment files found; this is expected when no agent runtime is active".into(),
            }
        };
    }
    if errors.is_empty() {
        AgentCheck {
            name: "runtime_env_files".into(),
            status: "ok".into(),
            detail: details.join(" | "),
        }
    } else {
        AgentCheck {
            name: "runtime_env_files".into(),
            status: "error".into(),
            detail: errors.join(" | "),
        }
    }
}

fn check_managed_agents(
    filter: Option<&str>,
    managed_agents: &[DoctorManagedAgent],
) -> AgentCheck {
    let matching = managed_agents
        .iter()
        .filter(|agent| managed_agent_matches_filter(agent, filter))
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return AgentCheck {
            name: "managed_agents".into(),
            status: "ok".into(),
            detail: "no managed agent snapshots found".into(),
        };
    }
    let mut details = Vec::new();
    let mut errors = Vec::new();
    for agent in matching {
        let last_activity = agent
            .last_activity
            .as_deref()
            .unwrap_or("unknown");
        let detail = format!(
            "{}: adapter={}, status={}, runtime={}, pane={}, pane_title={}, session={}, session_name={}, last_activity={}",
            agent.agent_id,
            agent.adapter,
            agent.status,
            agent.runtime_id,
            agent.pane_id,
            agent.pane_title,
            agent.mux_session_id,
            agent.session_name,
            last_activity
        );
        if agent.status == "Error" {
            errors.push(detail);
        } else {
            details.push(detail);
        }
    }
    if errors.is_empty() {
        AgentCheck {
            name: "managed_agents".into(),
            status: "ok".into(),
            detail: details.join(" | "),
        }
    } else {
        AgentCheck {
            name: "managed_agents".into(),
            status: "error".into(),
            detail: errors.join(" | "),
        }
    }
}

fn managed_agent_matches_filter(agent: &DoctorManagedAgent, filter: Option<&str>) -> bool {
    filter.is_none_or(|filter| {
        agent.agent_id.contains(filter)
            || agent.runtime_id.contains(filter)
            || agent.pane_id.contains(filter)
            || agent.mux_session_id.contains(filter)
    })
}

fn inspect_environment_file(path: &Path) -> Result<String, String> {
    let contents = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let values = parse_environment_file(&contents);
    let hook_endpoint = values.get("LUNA_MUX_HOOK_ENDPOINT").map(String::as_str);
    let mcp_endpoint = values.get("LUNA_MUX_MCP_ENDPOINT").map(String::as_str);
    let hook_token = values.get("LUNA_MUX_HOOK_AUTHORIZATION").map(String::as_str);
    let mcp_token = values.get("LUNA_MUX_MCP_AUTHORIZATION").map(String::as_str);
    let mut checks = Vec::new();
    for (label, endpoint) in [("hook", hook_endpoint), ("mcp", mcp_endpoint)] {
        let Some(endpoint) = endpoint else {
            checks.push(format!("{label}_endpoint=missing"));
            continue;
        };
        match endpoint_status(endpoint) {
            Ok(()) => checks.push(format!("{label}_endpoint=reachable")),
            Err(error) => checks.push(format!("{label}_endpoint={error}")),
        }
    }
    if hook_token.is_none_or(str::is_empty) || mcp_token.is_none_or(str::is_empty) {
        checks.push("tokens=missing".into());
    }
    Ok(checks.join(","))
}

fn parse_environment_file(contents: &str) -> BTreeMap<String, String> {
    let mut values = BTreeMap::new();
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        let (key, value) = if let Some(value) = line.strip_prefix("export ") {
            split_assignment(value)
        } else if let Some(value) = line.strip_prefix("$env:") {
            split_assignment(value)
        } else {
            split_assignment(line)
        };
        if let (Some(key), Some(value)) = (key, value) {
            values.insert(key.to_string(), unquote_value(value));
        }
    }
    values
}

fn split_assignment(value: &str) -> (Option<&str>, Option<&str>) {
    if let Some((key, value)) = value.split_once('=') {
        (Some(key.trim()), Some(value.trim()))
    } else {
        (None, None)
    }
}

fn unquote_value(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2
        && ((value.starts_with('\'') && value.ends_with('\''))
            || (value.starts_with('"') && value.ends_with('"')))
    {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

fn endpoint_status(endpoint: &str) -> Result<(), String> {
    let host_port = endpoint
        .strip_prefix("http://")
        .unwrap_or(endpoint)
        .split('/')
        .next()
        .unwrap_or_default();
    let (host, port) = host_port
        .rsplit_once(':')
        .ok_or_else(|| "invalid endpoint".to_string())?;
    if host != "127.0.0.1" && host != "localhost" {
        return Err(format!("unexpected host {host}"));
    }
    let port = port.parse::<u16>().map_err(|error| error.to_string())?;
    let address = format!("127.0.0.1:{port}")
        .parse::<SocketAddr>()
        .map_err(|error| error.to_string())?;
    TcpStream::connect_timeout(&address, Duration::from_millis(800))
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
fn check_wsl_distributions(filter: Option<&str>) -> AgentCheck {
    let output = match crate::local_pty_backend::windows_no_window_command("wsl.exe")
        .args(["--list", "--quiet"])
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            return AgentCheck {
                name: "wsl_distributions".into(),
                status: "warn".into(),
                detail: format!("wsl.exe unavailable: {error}"),
            };
        }
    };
    if !output.status.success() {
        return AgentCheck {
            name: "wsl_distributions".into(),
            status: "warn".into(),
            detail: wsl_failure_detail(&output.stderr, &output.stdout),
        };
    }
    let distributions = parse_wsl_distributions(&output.stdout, filter);
    if distributions.is_empty() {
        AgentCheck {
            name: "wsl_distributions".into(),
            status: "warn".into(),
            detail: "no WSL distributions found".into(),
        }
    } else {
        AgentCheck {
            name: "wsl_distributions".into(),
            status: "ok".into(),
            detail: distributions.join(", "),
        }
    }
}

#[cfg(windows)]
fn parse_wsl_distributions(bytes: &[u8], filter: Option<&str>) -> Vec<String> {
    crate::local_pty_backend::decode_windows_command_output(bytes)
        .split(|character| matches!(character, '\r' | '\n' | '\0'))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| filter.is_none_or(|filter| value.contains(filter)))
        .map(str::to_owned)
        .collect()
}

#[cfg(windows)]
fn wsl_failure_detail(stderr: &[u8], stdout: &[u8]) -> String {
    let stderr = crate::local_pty_backend::decode_windows_command_output(stderr);
    let detail = stderr.trim();
    if !detail.is_empty() {
        return normalize_wsl_detail(detail);
    }
    let stdout = crate::local_pty_backend::decode_windows_command_output(stdout);
    normalize_wsl_detail(stdout.trim())
}

#[cfg(windows)]
fn normalize_wsl_detail(detail: &str) -> String {
    detail
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    fn utf16le(value: &str) -> Vec<u8> {
        value.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    #[cfg(windows)]
    #[test]
    fn decodes_utf16_wsl_install_guidance() {
        let message = "适用于 Linux 的 Windows 子系统未安装。请运行 wsl.exe --install\r\r\n有关详细信息";
        assert_eq!(
            wsl_failure_detail(&utf16le(message), &[]),
            "适用于 Linux 的 Windows 子系统未安装。请运行 wsl.exe --install\n有关详细信息"
        );
    }

    #[cfg(windows)]
    #[test]
    fn parses_utf16_wsl_distributions_by_line() {
        assert_eq!(
            parse_wsl_distributions(&utf16le("Ubuntu\r\nDebian\r\n"), None),
            ["Ubuntu", "Debian"]
        );
    }

    #[test]
    fn active_managed_agents_suppress_missing_env_file_warning() {
        let agents = vec![DoctorManagedAgent {
            agent_id: "agent-1".into(),
            adapter: "codex".into(),
            runtime_id: "runtime-1".into(),
            pane_id: "pane-1".into(),
            pane_title: "My Pane".into(),
            mux_session_id: "session-1".into(),
            session_name: "My Session".into(),
            status: "Working".into(),
            last_activity: None,
        }];
        let checks = run(None, &agents);
        let runtime_env = checks
            .iter()
            .find(|check| check.name == "runtime_env_files")
            .unwrap();
        assert_eq!(runtime_env.status, "ok");
        let managed = checks
            .iter()
            .find(|check| check.name == "managed_agents")
            .unwrap();
        assert_eq!(managed.status, "ok");
        assert!(managed.detail.contains("agent-1"));
    }

    #[test]
    fn environment_parser_accepts_posix_and_powershell_forms() {
        let values = parse_environment_file(
            "export LUNA_MUX_HOOK_ENDPOINT='http://127.0.0.1:43127/v1/hooks'\n$env:LUNA_MUX_MCP_AUTHORIZATION = 'secret'\n",
        );
        assert_eq!(
            values.get("LUNA_MUX_HOOK_ENDPOINT").map(String::as_str),
            Some("http://127.0.0.1:43127/v1/hooks")
        );
        assert_eq!(
            values.get("LUNA_MUX_MCP_AUTHORIZATION").map(String::as_str),
            Some("secret")
        );
    }
}
