use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtimes: Vec<DoctorRuntimeReport>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentCheck {
    pub name: String,
    pub status: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorRuntimeCheck {
    pub name: String,
    pub status: String,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<DoctorEvidence>,
    #[serde(default)]
    pub repairable: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorEvidence {
    pub kind: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorRuntimeReport {
    pub runtime_id: String,
    pub target_id: String,
    pub title: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<DoctorRuntimeCheck>,
}

/// Runtime diagnostic input collected by the application. Secrets are used
/// only for probes and are never copied into the serialized report.
#[derive(Clone, Debug, Default)]
pub struct DoctorRuntimeInput {
    pub runtime_id: String,
    pub target_id: String,
    pub title: String,
    pub status: String,
    pub pane_id: Option<String>,
    pub pane_title: Option<String>,
    pub mux_session_id: Option<String>,
    pub hook_endpoint: Option<String>,
    pub hook_token: Option<String>,
    pub mcp_endpoint: Option<String>,
    pub mcp_token: Option<String>,
    pub remote_helper_exists: Option<bool>,
    pub remote_helper_log: Option<String>,
    pub remote_bridge_log: Option<String>,
    pub integration_enabled: bool,
    pub browser_runtime: Option<String>,
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

pub fn try_run_wsl_interop_probe(args: &[String]) -> Option<i32> {
    if args.get(1).map(String::as_str) != Some("wsl-interop-probe") {
        return None;
    }
    println!("luna-mux-wsl-interop-ok");
    Some(0)
}

pub fn run_report(filter: Option<&str>) -> AgentCheckReport {
    run_report_with_agents(filter, &[])
}

pub fn run_report_with_agents(
    filter: Option<&str>,
    managed_agents: &[DoctorManagedAgent],
) -> AgentCheckReport {
    run_report_with_runtime_inputs(filter, managed_agents, &[])
}

pub fn run_report_with_runtime_inputs(
    filter: Option<&str>,
    managed_agents: &[DoctorManagedAgent],
    runtime_inputs: &[DoctorRuntimeInput],
) -> AgentCheckReport {
    let checks = run(filter, managed_agents);
    let ok = checks.iter().all(|check| check.status != "error");
    let managed_agents = managed_agents
        .iter()
        .filter(|agent| managed_agent_matches_filter(agent, filter))
        .cloned()
        .collect::<Vec<_>>();
    let runtimes = runtime_inputs
        .iter()
        // Terminal backends retain exited records briefly so their output can
        // still be read. They are historical entries, not active terminals,
        // and must not inflate the diagnostics terminal count or fail the
        // current health summary.
        .filter(|runtime| !runtime.status.eq_ignore_ascii_case("exited"))
        .filter(|runtime| {
            filter.is_none_or(|value| {
                runtime.runtime_id.contains(value)
                    || runtime.target_id.contains(value)
                    || runtime
                        .pane_id
                        .as_deref()
                        .is_some_and(|id| id.contains(value))
            })
        })
        .map(runtime_report)
        .collect::<Vec<_>>();
    let runtime_ok = runtimes
        .iter()
        .flat_map(|runtime| runtime.checks.iter())
        .all(|check| check.status != "error");
    AgentCheckReport {
        ok: ok && runtime_ok,
        checks,
        managed_agents,
        runtimes,
    }
}

fn runtime_report(input: &DoctorRuntimeInput) -> DoctorRuntimeReport {
    let mut checks = Vec::new();
    let running = input.status.eq_ignore_ascii_case("running");
    checks.push(DoctorRuntimeCheck {
        name: "runtime".into(),
        status: if running { "ok" } else { "error" }.into(),
        detail: if running {
            "runtime is running".into()
        } else {
            format!("runtime status is {}", input.status)
        },
        code: (!running).then(|| "runtime_not_running".into()),
        phase: Some("runtime".into()),
        evidence: vec![DoctorEvidence {
            kind: "runtimeStatus".into(),
            detail: input.status.clone(),
        }],
        repairable: false,
    });
    if input.integration_enabled {
        checks.push(probe_runtime_endpoint(
            "luna_mcp",
            "luna_mcp",
            input.mcp_endpoint.as_deref(),
            input.mcp_token.as_deref(),
            true,
        ));
        checks.push(probe_runtime_endpoint(
            "hook",
            "hook",
            input.hook_endpoint.as_deref(),
            input.hook_token.as_deref(),
            false,
        ));
    }
    // A Browser Resource belongs to the session, but this Runtime may already
    // have exited. Do not attach the session's current Browser status to a
    // historical Runtime report.
    if running && let Some(browser) = input.browser_runtime.as_deref() {
        let lower = browser.to_ascii_lowercase();
        let (status, code, repairable) = if lower.starts_with("running") {
            ("ok", None, false)
        } else if lower.starts_with("error") {
            ("error", Some("browser_runtime_error"), true)
        } else {
            ("warn", Some("browser_runtime_not_running"), true)
        };
        checks.push(DoctorRuntimeCheck {
            name: "agent_browser".into(),
            status: status.into(),
            detail: browser.into(),
            code: code.map(str::to_string),
            phase: Some("agent_browser".into()),
            evidence: vec![],
            repairable,
        });
    }
    if let Some(exists) = input.remote_helper_exists {
        checks.push(DoctorRuntimeCheck {
            name: "remote_helper".into(),
            status: if exists { "ok" } else { "error" }.into(),
            detail: if exists {
                "remote helper is present".into()
            } else {
                "remote helper is missing".into()
            },
            code: (!exists).then(|| "remote_helper_missing".into()),
            phase: Some("remote_helper".into()),
            evidence: vec![],
            repairable: !exists,
        });
    }
    if let Some(log) = input.remote_helper_log.as_deref() {
        checks.push(remote_log_check("remote_helper_log", log));
    }
    if let Some(log) = input.remote_bridge_log.as_deref() {
        checks.push(remote_log_check("browser_bridge", log));
    }
    DoctorRuntimeReport {
        runtime_id: input.runtime_id.clone(),
        target_id: input.target_id.clone(),
        title: input.title.clone(),
        status: input.status.clone(),
        pane_id: input.pane_id.clone(),
        pane_title: input.pane_title.clone(),
        checks,
    }
}

fn probe_runtime_endpoint(
    name: &str,
    phase: &str,
    endpoint: Option<&str>,
    token: Option<&str>,
    mcp: bool,
) -> DoctorRuntimeCheck {
    let Some(endpoint) = endpoint.filter(|value| !value.trim().is_empty()) else {
        return DoctorRuntimeCheck {
            name: name.into(),
            status: "error".into(),
            detail: "endpoint is missing".into(),
            code: Some(
                if mcp {
                    "luna_mcp_endpoint_missing"
                } else {
                    "hook_endpoint_missing"
                }
                .into(),
            ),
            phase: Some(phase.into()),
            evidence: vec![],
            repairable: true,
        };
    };
    let Some(token) = token.filter(|value| !value.trim().is_empty()) else {
        return DoctorRuntimeCheck {
            name: name.into(),
            status: "error".into(),
            detail: "authorization token is missing".into(),
            code: Some(
                if mcp {
                    "luna_mcp_authorization_missing"
                } else {
                    "hook_authorization_missing"
                }
                .into(),
            ),
            phase: Some(phase.into()),
            evidence: vec![],
            repairable: true,
        };
    };
    match http_probe(endpoint, token, mcp) {
        Ok(detail) => DoctorRuntimeCheck {
            name: name.into(),
            status: "ok".into(),
            detail,
            code: None,
            phase: Some(phase.into()),
            evidence: vec![DoctorEvidence {
                kind: "http".into(),
                detail: "authenticated probe succeeded".into(),
            }],
            repairable: false,
        },
        Err((code, detail)) => DoctorRuntimeCheck {
            name: name.into(),
            status: "error".into(),
            detail,
            code: Some(code.into()),
            phase: Some(phase.into()),
            evidence: vec![],
            repairable: true,
        },
    }
}

fn http_probe(endpoint: &str, token: &str, mcp: bool) -> Result<String, (&'static str, String)> {
    let parsed = url::Url::parse(endpoint).map_err(|e| ("endpoint_invalid", e.to_string()))?;
    let host = parsed
        .host_str()
        .ok_or(("endpoint_invalid", "missing host".into()))?;
    if host != "127.0.0.1" && host != "localhost" {
        return Err((
            "endpoint_unexpected_host",
            format!("unexpected host {host}"),
        ));
    }
    let port = parsed
        .port_or_known_default()
        .ok_or(("endpoint_invalid", "missing port".into()))?;
    let address = format!("127.0.0.1:{port}")
        .parse::<SocketAddr>()
        .map_err(|e| ("endpoint_invalid", e.to_string()))?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(1200))
        .map_err(|e| ("endpoint_tcp_unreachable", e.to_string()))?;
    stream
        .set_read_timeout(Some(Duration::from_millis(1500)))
        .ok();
    stream
        .set_write_timeout(Some(Duration::from_millis(1500)))
        .ok();
    let body = if mcp {
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"luna-mux-doctor","version":"1"}}}"#
    } else {
        r#"{"hook_event_name":"__luna_mux_diagnostic__"}"#
    };
    let path = if parsed.path().is_empty() {
        "/"
    } else {
        parsed.path()
    };
    let diagnostic_header = if mcp {
        ""
    } else {
        "X-Luna-Mux-Diagnostic: 1\r\n"
    };
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nAuthorization: Bearer {token}\r\n{diagnostic_header}Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|e| ("endpoint_write_failed", e.to_string()))?;
    let mut response = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => {
                response.extend_from_slice(&buffer[..size]);
                if response.len() >= 32 * 1024
                    || (response.windows(4).any(|value| value == b"\r\n\r\n")
                        && response.contains(&b'}'))
                {
                    break;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                break;
            }
            Err(error) => return Err(("endpoint_read_failed", error.to_string())),
        }
    }
    let text = String::from_utf8_lossy(&response);
    let status = text
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(0);
    if !(200..300).contains(&status) {
        return Err((
            match status {
                401 | 403 => "endpoint_unauthorized",
                404 => "endpoint_not_found",
                405 => "endpoint_method_rejected",
                0 => "endpoint_no_http_response",
                _ => "endpoint_http_error",
            },
            format!("HTTP status {status}"),
        ));
    }
    if mcp && !text.contains("\"result\"") {
        return Err((
            "mcp_initialize_failed",
            "MCP initialize response did not contain a result".into(),
        ));
    }
    Ok(format!(
        "HTTP {status}; {} probe succeeded",
        if mcp { "MCP initialize" } else { "hook" }
    ))
}

fn remote_log_check(name: &str, log: &str) -> DoctorRuntimeCheck {
    let lower = log.to_ascii_lowercase();
    let (status, code, detail, repairable) = if name == "browser_bridge"
        && lower.contains("browser reverse forward registered")
        && !lower.contains("connection received")
    {
        (
            "warn",
            "browser_reverse_forward_not_connected",
            "Browser reverse forward is registered but the remote helper has not connected",
            true,
        )
    } else if name == "browser_bridge"
        && (lower.contains("stdout closed")
            || lower.contains("spawn failed")
            || lower.contains("agent-browser exited"))
    {
        (
            "error",
            "browser_sidecar_start_failed",
            "agent-browser sidecar closed or failed during MCP startup",
            true,
        )
    } else if lower.contains("no_transport_succeeded")
        || lower.contains("missing_credentials")
        || lower.contains("no_tcp_transport")
    {
        (
            "error",
            "remote_helper_transport_failed",
            "remote helper could not send its request",
            true,
        )
    } else if lower.contains("authentication failed")
        || lower.contains("auth_failed")
        || lower.contains("认证失败")
    {
        (
            "error",
            "browser_authentication_failed",
            "remote Browser bridge authentication failed",
            true,
        )
    } else if lower.contains("connection received")
        || lower.contains("transport=curl success")
        || lower.contains("transport=wget success")
        || lower.contains("transport=socat")
        || lower.contains("transport=nc")
        || lower.contains("transport=ncat")
        || lower.contains("transport=bash-dev-tcp")
    {
        (
            "ok",
            "",
            "remote helper/bridge has established a transport",
            false,
        )
    } else {
        (
            "warn",
            "remote_helper_no_recent_success",
            "no recent successful helper transport was recorded",
            true,
        )
    };
    let evidence = log
        .lines()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(redact_log_line)
        .collect::<Vec<_>>()
        .join("\n");
    DoctorRuntimeCheck {
        name: name.into(),
        status: status.into(),
        detail: detail.into(),
        code: (!code.is_empty()).then(|| code.into()),
        phase: Some(
            if name == "browser_bridge" {
                "browser_bridge"
            } else {
                "remote_helper"
            }
            .into(),
        ),
        evidence: vec![DoctorEvidence {
            kind: "logTail".into(),
            detail: evidence,
        }],
        repairable,
    }
}

fn redact_log_line(line: &str) -> String {
    let mut redacted = line.to_string();
    for marker in ["lmxh_", "lmxb_", "lmxbm_", "Bearer "] {
        loop {
            let Some(start) = redacted.find(marker) else {
                break;
            };
            let end = redacted[start..]
                .find(|character: char| {
                    character.is_whitespace() || matches!(character, '"' | '\'' | ',' | ';')
                })
                .map(|offset| start + offset)
                .unwrap_or(redacted.len());
            redacted.replace_range(start..end, "<redacted>");
        }
    }
    redacted
}

fn run(filter: Option<&str>, managed_agents: &[DoctorManagedAgent]) -> Vec<AgentCheck> {
    let mut checks = Vec::new();
    checks.push(check_executable());
    checks.push(check_local_agents());
    checks.push(check_runtime_environment_files(filter, managed_agents));
    checks.push(check_managed_agents(filter, managed_agents));
    #[cfg(windows)]
    {
        checks.push(check_wsl_distributions(filter));
        checks.push(check_wsl_interop_executable(filter));
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

fn check_managed_agents(filter: Option<&str>, managed_agents: &[DoctorManagedAgent]) -> AgentCheck {
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
        let last_activity = agent.last_activity.as_deref().unwrap_or("unknown");
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
    let hook_token = values
        .get("LUNA_MUX_HOOK_AUTHORIZATION")
        .map(String::as_str);
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
fn check_wsl_interop_executable(filter: Option<&str>) -> AgentCheck {
    let executable = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            return AgentCheck {
                name: "wsl_interop_exe".into(),
                status: "error".into(),
                detail: format!("current executable unavailable: {error}"),
            };
        }
    };
    let wsl_path = match windows_executable_wsl_path(&executable) {
        Ok(path) => path,
        Err(error) => {
            return AgentCheck {
                name: "wsl_interop_exe".into(),
                status: "error".into(),
                detail: error,
            };
        }
    };
    let distributions = match wsl_distributions(filter) {
        Ok(distributions) if distributions.is_empty() => {
            return AgentCheck {
                name: "wsl_interop_exe".into(),
                status: "warn".into(),
                detail: "no WSL distributions found".into(),
            };
        }
        Ok(distributions) => distributions,
        Err(error) => {
            return AgentCheck {
                name: "wsl_interop_exe".into(),
                status: "warn".into(),
                detail: error,
            };
        }
    };

    let mut details = Vec::new();
    let mut errors = Vec::new();
    for distribution in distributions {
        match probe_wsl_path_exists(&distribution, &wsl_path) {
            Ok(true) => {}
            Ok(false) => {
                errors.push(format!(
                    "{distribution}: Windows executable not visible in WSL: {wsl_path}"
                ));
                continue;
            }
            Err(error) => {
                errors.push(format!("{distribution}: {error}"));
                continue;
            }
        }
        match probe_wsl_windows_executable(&distribution, &wsl_path) {
            Ok(()) => details.push(format!(
                "{distribution}: exists and runs from WSL ({wsl_path})"
            )),
            Err(error) => errors.push(format!(
                "{distribution}: exists but could not be executed from WSL ({wsl_path}): {error}"
            )),
        }
    }
    if errors.is_empty() {
        AgentCheck {
            name: "wsl_interop_exe".into(),
            status: "ok".into(),
            detail: details.join("; "),
        }
    } else {
        AgentCheck {
            name: "wsl_interop_exe".into(),
            status: "error".into(),
            detail: errors.join("\n"),
        }
    }
}

#[cfg(windows)]
fn wsl_distributions(filter: Option<&str>) -> Result<Vec<String>, String> {
    let output = match crate::local_pty_backend::windows_no_window_command("wsl.exe")
        .args(["--list", "--quiet"])
        .output()
    {
        Ok(output) => output,
        Err(error) => return Err(format!("wsl.exe unavailable: {error}")),
    };
    if !output.status.success() {
        return Err(wsl_failure_detail(&output.stderr, &output.stdout));
    }
    let distributions = parse_wsl_distributions(&output.stdout, filter);
    if distributions.is_empty() {
        return Err("no WSL distributions found".into());
    }
    Ok(distributions)
}

#[cfg(windows)]
fn probe_wsl_path_exists(distribution: &str, wsl_path: &str) -> Result<bool, String> {
    let command = format!(
        "test -e {}",
        crate::shell_quoting::posix_shell_quote(wsl_path)
    );
    let output = crate::local_pty_backend::windows_no_window_command("wsl.exe")
        .args([
            "--distribution",
            distribution,
            "--",
            "/bin/sh",
            "-lc",
            command.as_str(),
        ])
        .output()
        .map_err(|error| format!("failed to run wsl.exe: {error}"))?;
    Ok(output.status.success())
}

#[cfg(windows)]
fn probe_wsl_windows_executable(distribution: &str, wsl_path: &str) -> Result<(), String> {
    let command = format!(
        "{} wsl-interop-probe",
        crate::shell_quoting::posix_shell_quote(wsl_path)
    );
    let output = crate::local_pty_backend::windows_no_window_command("wsl.exe")
        .args([
            "--distribution",
            distribution,
            "--",
            "/bin/sh",
            "-lc",
            command.as_str(),
        ])
        .output()
        .map_err(|error| format!("failed to run wsl.exe: {error}"))?;
    if output.status.success() {
        return Ok(());
    }

    let stdout = crate::local_pty_backend::decode_windows_command_output(&output.stdout);
    let stderr = crate::local_pty_backend::decode_windows_command_output(&output.stderr);
    let status = output
        .status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "unknown".into());
    let mut detail = String::new();
    for value in [stdout, stderr] {
        let value = value.trim();
        if !value.is_empty() {
            if !detail.is_empty() {
                detail.push(' ');
            }
            detail.push_str(value);
        }
    }
    if detail.is_empty() {
        detail = format!("WSL interop probe exited with status {status}");
    } else {
        detail = format!("{detail} (status {status})");
    }
    Err(normalize_wsl_detail(&detail))
}

#[cfg(windows)]
fn windows_executable_wsl_path(executable: &Path) -> Result<String, String> {
    let value = executable.to_string_lossy();
    let bytes = value.as_bytes();
    if bytes.len() < 3 || bytes[1] != b':' || !matches!(bytes[2], b'\\' | b'/') {
        return Err("current executable is not on a Windows local drive".into());
    }
    let drive = (bytes[0] as char).to_ascii_lowercase();
    Ok(format!("/mnt/{drive}/{}", value[3..].replace('\\', "/")))
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
        let message =
            "适用于 Linux 的 Windows 子系统未安装。请运行 wsl.exe --install\r\r\n有关详细信息";
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

    #[test]
    fn runtime_report_exposes_actionable_endpoint_codes_without_secrets() {
        let report = runtime_report(&DoctorRuntimeInput {
            runtime_id: "runtime-1".into(),
            target_id: "local:shell".into(),
            title: "Shell".into(),
            status: "Running".into(),
            hook_endpoint: None,
            hook_token: None,
            mcp_endpoint: Some("http://127.0.0.1:1/mcp".into()),
            mcp_token: Some("secret-token".into()),
            integration_enabled: true,
            ..Default::default()
        });
        let hook = report
            .checks
            .iter()
            .find(|check| check.name == "hook")
            .unwrap();
        assert_eq!(hook.code.as_deref(), Some("hook_endpoint_missing"));
        let encoded = serde_json::to_string(&report).unwrap();
        assert!(!encoded.contains("secret-token"));
    }

    #[test]
    fn exited_runtime_is_not_reported_as_an_active_terminal() {
        let report = run_report_with_runtime_inputs(
            None,
            &[],
            &[DoctorRuntimeInput {
                runtime_id: "runtime-exited".into(),
                target_id: "local:powershell".into(),
                title: "PowerShell 7".into(),
                status: "Exited".into(),
                browser_runtime: Some("Running cdp=51860".into()),
                integration_enabled: true,
                ..Default::default()
            }],
        );

        assert!(report.ok);
        assert!(report.runtimes.is_empty());
    }

    #[test]
    fn exited_runtime_report_does_not_attach_browser_status() {
        let report = runtime_report(&DoctorRuntimeInput {
            runtime_id: "runtime-exited".into(),
            target_id: "local:powershell".into(),
            title: "PowerShell 7".into(),
            status: "Exited".into(),
            browser_runtime: Some("Running cdp=51860".into()),
            ..Default::default()
        });

        assert!(
            report
                .checks
                .iter()
                .all(|check| check.name != "agent_browser")
        );
    }

    #[test]
    fn remote_log_classification_distinguishes_transport_and_bridge_auth() {
        assert_eq!(
            remote_log_check(
                "remote_helper_log",
                "start mode=hook\nhook no_transport_succeeded"
            )
            .code
            .as_deref(),
            Some("remote_helper_transport_failed")
        );
        assert_eq!(
            remote_log_check(
                "browser_bridge",
                "remote forward bridge failed: authentication failed"
            )
            .code
            .as_deref(),
            Some("browser_authentication_failed")
        );
        assert_eq!(
            remote_log_check("remote_helper_log", "browser transport=socat port=1234").status,
            "ok"
        );
    }
}
