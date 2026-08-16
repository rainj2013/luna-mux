use std::{
    collections::BTreeMap,
    fs,
    net::{SocketAddr, TcpStream},
    path::Path,
    process::Command,
    time::Duration,
};

use serde::Serialize;

#[derive(Serialize)]
struct AgentCheckReport {
    ok: bool,
    checks: Vec<AgentCheck>,
}

#[derive(Serialize)]
struct AgentCheck {
    name: String,
    status: String,
    detail: String,
}

pub fn try_run_agent_check(args: &[String]) -> Option<i32> {
    let subcommand = args.get(1).map(String::as_str)?;
    if subcommand != "agent-check" && subcommand != "doctor" {
        return None;
    }
    let filter = args.get(2).map(String::as_str);
    let checks = run(filter);
    let ok = checks.iter().all(|check| check.status != "error");
    let report = AgentCheckReport { ok, checks };
    println!(
        "{}",
        serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".into())
    );
    Some(if ok { 0 } else { 1 })
}

fn run(filter: Option<&str>) -> Vec<AgentCheck> {
    let mut checks = Vec::new();
    checks.push(check_executable());
    checks.push(check_local_agents());
    checks.push(check_runtime_environment_files(filter));
    if cfg!(windows) {
        checks.push(check_wsl_distributions(filter));
    }
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
    let mut missing = Vec::new();
    for command in ["codex", "claude"] {
        match find_command(command) {
            Some(path) => available.push(format!("{command}={path}")),
            None => missing.push(command.to_string()),
        }
    }
    if missing.is_empty() {
        AgentCheck {
            name: "local_agents".into(),
            status: "ok".into(),
            detail: available.join("; "),
        }
    } else {
        AgentCheck {
            name: "local_agents".into(),
            status: "warn".into(),
            detail: format!("missing {}; found {}", missing.join(", "), available.join("; ")),
        }
    }
}

fn check_runtime_environment_files(filter: Option<&str>) -> AgentCheck {
    let root = std::env::temp_dir().join("luna-mux");
    let Ok(entries) = fs::read_dir(&root) else {
        return AgentCheck {
            name: "runtime_env_files".into(),
            status: "warn".into(),
            detail: "no luna-mux temp directory".into(),
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
        return AgentCheck {
            name: "runtime_env_files".into(),
            status: "warn".into(),
            detail: "no persistent runtime environment files found; this is expected when no agent runtime is active".into(),
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

fn check_wsl_distributions(filter: Option<&str>) -> AgentCheck {
    let output = match Command::new("wsl.exe").args(["--list", "--quiet"]).output() {
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
            detail: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        };
    }
    let raw = decode_wsl_stdout(&output.stdout);
    let distributions = raw
        .split('\0')
        .map(|value| value.trim().replace('\r', ""))
        .filter(|value| !value.is_empty())
        .filter(|value| filter.is_none_or(|filter| value.contains(filter)))
        .collect::<Vec<_>>();
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

fn decode_wsl_stdout(bytes: &[u8]) -> String {
    let bytes = bytes
        .strip_prefix(&[0xff, 0xfe])
        .or_else(|| bytes.strip_prefix(&[0xfe, 0xff]))
        .unwrap_or(bytes);
    let mut units = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks(2) {
        if chunk.len() == 2 {
            units.push(u16::from_le_bytes([chunk[0], chunk[1]]));
        }
    }
    String::from_utf16_lossy(&units)
}

fn find_command(command: &str) -> Option<String> {
    #[cfg(windows)]
    {
        let candidates: &[&str] = match command {
            "codex" => &["codex.cmd", "codex.exe"],
            "claude" => &["claude.cmd", "claude.exe"],
            _ => &[],
        };
        for candidate in candidates {
            let output = Command::new("where.exe").arg(candidate).output().ok()?;
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty())?
                    .to_string();
                return Some(path);
            }
        }
        None
    }
    #[cfg(not(windows))]
    {
        let output = Command::new("sh")
            .args(["-lc", &format!("command -v {}", crate::shell_quoting::posix_shell_quote(command))])
            .output()
            .ok()?;
        output.status.success().then(|| {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .unwrap_or_default()
                .to_string()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
