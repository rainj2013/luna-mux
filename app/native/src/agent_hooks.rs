use std::{
    collections::{HashMap, HashSet, VecDeque},
    future::Future,
    io::Read,
    pin::Pin,
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    agent_adapters,
    terminal_runtime_contract::{
        TerminalManagedAgentContext, TerminalRuntimeContext, TerminalRuntimeOutputEvent,
    },
};

const EVENT_CAPACITY: usize = 2048;
const MAX_HOOK_BYTES: usize = 1024 * 1024;
const TERMINAL_ACTIVITY_INTERVAL: Duration = Duration::from_secs(3);
const TERMINAL_SIGNAL_INTERVAL: Duration = Duration::from_secs(1);
const AGENT_ADAPTER_HEADER: &str = "x-luna-mux-agent-adapter";
const AGENT_PROCESS_ID_HEADER: &str = "x-luna-mux-agent-process-id";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ManagedAgentStatus {
    Working,
    Waiting,
    Completed,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ManagedAgentWaitingReason {
    Input,
    Permission,
    External,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ManagedAgentEvidence {
    StructuredHook,
    TerminalHeuristic,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentEvent {
    pub sequence: u64,
    pub timestamp: String,
    pub context: TerminalManagedAgentContext,
    pub adapter_id: String,
    pub agent_session_id: Option<String>,
    pub agent_turn_id: Option<String>,
    pub hook_event_name: String,
    pub status: ManagedAgentStatus,
    pub waiting_reason: Option<ManagedAgentWaitingReason>,
    pub evidence: ManagedAgentEvidence,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ManagedAgentSnapshot {
    pub context: TerminalManagedAgentContext,
    pub status: ManagedAgentStatus,
    pub waiting_reason: Option<ManagedAgentWaitingReason>,
    pub last_activity: Option<String>,
    pub latest_sequence: Option<u64>,
    pub evidence: Option<ManagedAgentEvidence>,
}

type AgentEventSink = Arc<dyn Fn(ManagedAgentEvent) + Send + Sync>;
type BrowserEnsureFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;
type BrowserEnsure = Arc<dyn Fn(String) -> BrowserEnsureFuture + Send + Sync>;

#[derive(Default)]
struct AgentEventBuffer {
    next_sequence: u64,
    events: VecDeque<ManagedAgentEvent>,
}

pub struct AgentHookService {
    endpoint: RwLock<String>,
    agents: RwLock<HashMap<String, TerminalManagedAgentContext>>,
    bootstrap_tokens: RwLock<HashMap<String, TerminalRuntimeContext>>,
    agent_sessions: RwLock<HashMap<(String, String), String>>,
    events: Mutex<AgentEventBuffer>,
    structured_agents: Mutex<HashSet<String>>,
    terminal_signals: Mutex<HashMap<(String, String), Instant>>,
    event_sink: RwLock<Option<AgentEventSink>>,
    browser_ensure: RwLock<Option<BrowserEnsure>>,
}

impl AgentHookService {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            endpoint: RwLock::new(String::new()),
            agents: RwLock::new(HashMap::new()),
            bootstrap_tokens: RwLock::new(HashMap::new()),
            agent_sessions: RwLock::new(HashMap::new()),
            events: Mutex::new(AgentEventBuffer::default()),
            structured_agents: Mutex::new(HashSet::new()),
            terminal_signals: Mutex::new(HashMap::new()),
            event_sink: RwLock::new(None),
            browser_ensure: RwLock::new(None),
        })
    }

    pub fn start(self: &Arc<Self>) -> Result<(), String> {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .map_err(|error| error.to_string())?;
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        *self
            .endpoint
            .write()
            .map_err(|_| "Agent Hook 地址锁已损坏")? =
            format!("http://127.0.0.1:{}/v1/hooks", address.port());
        let service = self.clone();
        tauri::async_runtime::spawn(async move {
            let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
                return;
            };
            let app = Router::new()
                .route("/v1/hooks", post(receive_hook))
                .with_state(service);
            let _ = axum::serve(listener, app).await;
        });
        Ok(())
    }

    pub fn set_event_sink(&self, sink: AgentEventSink) {
        *self.event_sink.write().expect("agent event sink lock") = Some(sink);
    }

    pub fn set_browser_ensure(&self, ensure: BrowserEnsure) {
        *self.browser_ensure.write().expect("browser ensure lock") = Some(ensure);
    }

    async fn ensure_browser(&self, mux_session_id: &str) -> Result<(), String> {
        let ensure = self
            .browser_ensure
            .read()
            .map_err(|_| "Browser 自动启动状态已损坏".to_string())?
            .clone()
            .ok_or_else(|| "Browser 自动启动服务尚未就绪".to_string())?;
        ensure(mux_session_id.to_string()).await
    }

    pub fn issue_token(&self, context: TerminalManagedAgentContext) -> Result<String, String> {
        let token = format!("lmxh_{}", Uuid::new_v4().simple());
        self.agents
            .write()
            .map_err(|_| "Agent Hook 授权锁已损坏")?
            .insert(token.clone(), context);
        Ok(token)
    }

    pub fn issue_bootstrap_token(&self, context: TerminalRuntimeContext) -> Result<String, String> {
        let token = format!("lmxb_{}", Uuid::new_v4().simple());
        self.bootstrap_tokens
            .write()
            .map_err(|_| "Agent Hook 启动授权锁已损坏")?
            .insert(token.clone(), context);
        Ok(token)
    }

    pub fn revoke_token(&self, token: &str) {
        if let Ok(mut agents) = self.agents.write() {
            agents.retain(|key, _| key != token && !key.starts_with(&format!("{token}:")));
        }
        if let Ok(mut tokens) = self.bootstrap_tokens.write() {
            tokens.remove(token);
        }
    }

    pub fn revoke_runtime(&self, runtime_id: &str) {
        if let Ok(mut tokens) = self.bootstrap_tokens.write() {
            tokens.retain(|_, context| context.runtime_id != runtime_id);
        }
        let _session_agents = self
            .agent_sessions
            .write()
            .map(|mut sessions| {
                let keys = sessions
                    .keys()
                    .filter(|(runtime, _)| runtime == runtime_id)
                    .cloned()
                    .collect::<Vec<_>>();
                keys.into_iter()
                    .filter_map(|key| sessions.remove(&key))
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
        let contexts = if let Ok(mut agents) = self.agents.write() {
            let contexts = agents
                .values()
                .filter(|context| context.runtime_id == runtime_id)
                .cloned()
                .collect::<Vec<_>>();
            agents.retain(|_, context| context.runtime_id != runtime_id);
            contexts
        } else {
            Vec::new()
        };
        let mut recorded_agents = std::collections::HashSet::new();
        for context in contexts {
            if recorded_agents.insert(context.agent_id.clone()) {
                self.record(context, json!({ "hook_event_name": "RuntimeExit" }));
            }
        }
        if let Ok(mut structured) = self.structured_agents.lock() {
            structured.retain(|agent_id| !recorded_agents.contains(agent_id));
        }
        if let Ok(mut signals) = self.terminal_signals.lock() {
            signals.retain(|(id, _), _| id != runtime_id);
        }
    }

    pub fn endpoint(&self) -> Result<String, String> {
        let endpoint = self
            .endpoint
            .read()
            .map_err(|_| "Agent Hook 地址锁已损坏")?
            .clone();
        if endpoint.is_empty() {
            Err("Agent Hook 服务尚未启动".into())
        } else {
            Ok(endpoint)
        }
    }

    /// Returns a token for an active runtime only for an in-process diagnostic
    /// probe. The token is never serialized or logged.
    pub(crate) fn diagnostic_token_for_runtime(&self, runtime_id: &str) -> Option<String> {
        self.agents
            .read()
            .ok()?
            .iter()
            .find(|(_, context)| context.runtime_id == runtime_id)
            .map(|(token, _)| token.clone())
            .or_else(|| {
                self.bootstrap_tokens
                    .read()
                    .ok()?
                    .iter()
                    .find(|(_, context)| context.runtime_id == runtime_id)
                    .map(|(token, _)| token.clone())
            })
    }

    pub fn events(&self) -> Vec<ManagedAgentEvent> {
        self.events
            .lock()
            .map(|events| events.events.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn snapshots(&self) -> Vec<ManagedAgentSnapshot> {
        let contexts = self
            .agents
            .read()
            .map(|agents| agents.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let events = self.events();
        let mut seen_agents = HashSet::new();
        let mut snapshots = contexts
            .into_iter()
            .filter(|context| seen_agents.insert(context.agent_id.clone()))
            .map(|context| {
                let latest = events
                    .iter()
                    .rev()
                    .find(|event| event.context.agent_id == context.agent_id);
                ManagedAgentSnapshot {
                    context,
                    status: latest
                        .map(|event| event.status.clone())
                        .unwrap_or(ManagedAgentStatus::Working),
                    waiting_reason: latest.and_then(|event| event.waiting_reason.clone()),
                    last_activity: latest.map(|event| event.timestamp.clone()),
                    latest_sequence: latest.map(|event| event.sequence),
                    evidence: latest.map(|event| event.evidence.clone()),
                }
            })
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| left.context.agent_id.cmp(&right.context.agent_id));
        snapshots
    }

    fn authenticate(
        &self,
        headers: &HeaderMap,
        session_id: Option<&str>,
        process_id: Option<&str>,
    ) -> Option<HookAuthorization> {
        let token = headers
            .get(axum::http::header::AUTHORIZATION)?
            .to_str()
            .ok()?
            .strip_prefix("Bearer ")?;
        if let Some((token_key, context)) = self.authenticate_token(token, session_id, process_id) {
            return Some(HookAuthorization::Agent {
                token: token.to_string(),
                token_key,
                context,
            });
        }
        self.bootstrap_tokens
            .read()
            .ok()?
            .get(token)
            .cloned()
            .map(|context| HookAuthorization::Bootstrap {
                token: token.into(),
                context,
            })
    }

    fn authenticate_token(
        &self,
        token: &str,
        session_id: Option<&str>,
        process_id: Option<&str>,
    ) -> Option<(String, TerminalManagedAgentContext)> {
        let agents = self.agents.read().ok()?;
        let session_key = session_id.map(|session_id| format!("{token}:{session_id}"));
        if let Some(key) = session_key
            && let Some(context) = agents.get(&key)
        {
            return Some((key, context.clone()));
        }
        let process_key = process_id.map(|process_id| format!("{token}:process:{process_id}"));
        if let Some(key) = process_key
            && let Some(context) = agents.get(&key)
        {
            return Some((key, context.clone()));
        }
        agents
            .get(token)
            .cloned()
            .map(|context| (token.to_string(), context))
    }

    pub fn record_terminal_output(&self, output: &TerminalRuntimeOutputEvent) {
        let contexts = self
            .agents
            .read()
            .map(|agents| {
                agents
                    .values()
                    .filter(|context| context.runtime_id == output.runtime_id)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if contexts.is_empty() {
            return;
        }
        let structured = self
            .structured_agents
            .lock()
            .map(|agents| agents.clone())
            .unwrap_or_default();
        let signal = terminal_signal(&output.data);
        let interval = if signal == "TerminalActivity" {
            TERMINAL_ACTIVITY_INTERVAL
        } else {
            TERMINAL_SIGNAL_INTERVAL
        };
        let now = Instant::now();
        let should_record = self
            .terminal_signals
            .lock()
            .map(|mut signals| {
                let key = (output.runtime_id.clone(), signal.to_string());
                if signals
                    .get(&key)
                    .is_some_and(|previous| now.duration_since(*previous) < interval)
                {
                    false
                } else {
                    signals.insert(key, now);
                    true
                }
            })
            .unwrap_or(false);
        if !should_record {
            return;
        }
        let mut recorded_agents = HashSet::new();
        for context in contexts {
            if !structured.contains(&context.agent_id)
                && recorded_agents.insert(context.agent_id.clone())
            {
                self.record_event(
                    context,
                    signal.into(),
                    None,
                    None,
                    ManagedAgentEvidence::TerminalHeuristic,
                );
            }
        }
    }

    fn record(&self, context: TerminalManagedAgentContext, payload: Value) -> ManagedAgentEvent {
        let hook_event_name = payload
            .get("hook_event_name")
            .and_then(Value::as_str)
            .unwrap_or("InvalidHookPayload")
            .to_string();
        if let Ok(mut agents) = self.structured_agents.lock() {
            agents.insert(context.agent_id.clone());
        }
        self.record_event(
            context,
            hook_event_name,
            payload
                .get("session_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            payload
                .get("turn_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            ManagedAgentEvidence::StructuredHook,
        )
    }

    fn record_event(
        &self,
        context: TerminalManagedAgentContext,
        hook_event_name: String,
        agent_session_id: Option<String>,
        agent_turn_id: Option<String>,
        evidence: ManagedAgentEvidence,
    ) -> ManagedAgentEvent {
        let (status, waiting_reason) = status_for_hook(&hook_event_name);
        let mut events = self.events.lock().expect("agent event buffer lock");
        let adapter_id = agent_adapters::adapter_id_for_profile(&context.launch_profile_id)
            .unwrap_or(agent_adapters::CODEX_ADAPTER_ID)
            .to_string();
        let event = ManagedAgentEvent {
            sequence: events.next_sequence,
            timestamp: chrono::Utc::now().to_rfc3339(),
            context,
            adapter_id,
            agent_session_id,
            agent_turn_id,
            hook_event_name,
            status,
            waiting_reason,
            evidence,
        };
        events.next_sequence = events.next_sequence.saturating_add(1);
        events.events.push_back(event.clone());
        while events.events.len() > EVENT_CAPACITY {
            events.events.pop_front();
        }
        drop(events);
        if let Some(sink) = self
            .event_sink
            .read()
            .expect("agent event sink lock")
            .as_ref()
        {
            sink(event.clone());
        }
        event
    }
}

enum HookAuthorization {
    Agent {
        token: String,
        token_key: String,
        context: TerminalManagedAgentContext,
    },
    Bootstrap {
        token: String,
        context: TerminalRuntimeContext,
    },
}

fn terminal_signal(data: &str) -> &'static str {
    if data.contains("\x1b]") {
        "TerminalOsc"
    } else if data.contains('\x07') {
        "TerminalBell"
    } else {
        "TerminalActivity"
    }
}

fn browser_routing_hook_response(payload: &Value) -> Option<Value> {
    if payload.get("hook_event_name").and_then(Value::as_str) != Some("PreToolUse") {
        return None;
    }
    let tool_name = payload
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let is_resource_lifecycle_tool = tool_name.contains("agent_browser")
        && [
            "agent_browser_close",
            "agent_browser_connect",
            "agent_browser_dashboard_start",
            "agent_browser_dashboard_stop",
            "agent_browser_install",
            "agent_browser_upgrade",
            "agent_browser_plugin_add",
            "agent_browser_plugin_run",
            "agent_browser_chat",
        ]
        .iter()
        .any(|suffix| tool_name.ends_with(suffix));
    let uses_named_browser_session = tool_name.contains("agent_browser")
        && payload
            .get("tool_input")
            .and_then(|input| input.get("session"))
            .and_then(Value::as_str)
            .is_some_and(|session| !session.trim().is_empty());
    if uses_named_browser_session {
        return Some(json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": "Use the Luna Mux injected default browser session and omit the session argument. Ordinary navigation must reuse the current bound page; use a tab tool in the default session only when a new tab is explicitly required."
            }
        }));
    }
    let command = payload
        .get("tool_input")
        .and_then(|input| input.get("command"))
        .and_then(Value::as_str);
    if !is_resource_lifecycle_tool && !command.is_some_and(shell_command_launches_browser) {
        return None;
    }
    Some(json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": "Luna Mux owns Browser Resource process and connection lifecycle. Use the existing agent_browser connection and its page, tab, or window tools."
        }
    }))
}

fn is_agent_browser_tool(payload: &Value) -> bool {
    payload.get("hook_event_name").and_then(Value::as_str) == Some("PreToolUse")
        && payload
            .get("tool_name")
            .and_then(Value::as_str)
            .is_some_and(|name| name.to_ascii_lowercase().contains("agent_browser"))
}

fn browser_start_failure_response(error: &str) -> Value {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": format!("Luna Mux 无法按需启动 Browser Resource：{error}")
        }
    })
}

fn shell_command_launches_browser(command: &str) -> bool {
    let command = command.trim().to_ascii_lowercase();
    let names_browser = [
        "chrome",
        "chromium",
        "msedge",
        "google chrome",
        "google-chrome",
    ]
    .iter()
    .any(|name| command.contains(name));
    if !names_browser {
        return false;
    }
    command.contains("start-process")
        || command.contains("--remote-debugging-port")
        || command.contains("cmd /c start")
        || command.contains("open -a")
        || command.starts_with("start ")
        || command.starts_with('&')
        || ["chrome", "chromium", "msedge", "google-chrome"]
            .iter()
            .any(|name| command.starts_with(name))
}

async fn receive_hook(
    State(service): State<Arc<AgentHookService>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if body.len() > MAX_HOOK_BYTES {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({ "error": "payloadTooLarge" })),
        ));
    }
    let payload: Value = serde_json::from_slice(&body).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalidJson" })),
        )
    })?;
    let session_id = payload.get("session_id").and_then(Value::as_str);
    let process_id = payload
        .get("luna_mux_process_id")
        .and_then(Value::as_str)
        .or_else(|| header_value(&headers, AGENT_PROCESS_ID_HEADER));
    let adapter_id = payload
        .get("agent_adapter")
        .and_then(Value::as_str)
        .or_else(|| header_value(&headers, AGENT_ADAPTER_HEADER));
    let authorization = service
        .authenticate(&headers, session_id, process_id)
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "unauthorized" })),
            )
        })?;
    if !payload.is_object()
        || payload
            .get("hook_event_name")
            .and_then(Value::as_str)
            .is_none()
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalidHookPayload" })),
        ));
    }
    // The doctor uses a dedicated authenticated probe. It verifies the exact
    // token/context path without recording a fake agent lifecycle event.
    if header_value(&headers, "x-luna-mux-diagnostic") == Some("1") {
        return Ok(Json(json!({ "ok": true, "diagnostic": true })));
    }
    let mut hook_response = browser_routing_hook_response(&payload);
    let mux_session_id = match &authorization {
        HookAuthorization::Agent { context, .. } => context.mux_session_id.as_str(),
        HookAuthorization::Bootstrap { context, .. } => context.mux_session_id.as_str(),
    };
    if hook_response.is_none() && is_agent_browser_tool(&payload) {
        let ensure = service.ensure_browser(mux_session_id);
        match tokio::time::timeout(Duration::from_secs(20), ensure).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                hook_response = Some(browser_start_failure_response(&error));
            }
            Err(_) => {
                hook_response = Some(browser_start_failure_response(
                    "agent-browser warmup timed out after 20 seconds",
                ));
            }
        }
    }
    let event = match authorization {
        HookAuthorization::Agent {
            token,
            token_key,
            context,
        } => {
            let event = service.record(context.clone(), payload);
            if event.hook_event_name == "SessionStart"
                && let Some(session_id) = event.agent_session_id.as_deref()
            {
                if let Ok(mut agents) = service.agents.write() {
                    agents.insert(format!("{token}:{session_id}"), context.clone());
                }
                if let Ok(mut sessions) = service.agent_sessions.write() {
                    sessions.insert(
                        (context.runtime_id.clone(), session_id.to_string()),
                        context.agent_id.clone(),
                    );
                }
            }
            if event.hook_event_name == "SessionEnd" {
                if let Ok(mut agents) = service.agents.write() {
                    agents.remove(&token_key);
                }
                if let Ok(mut structured) = service.structured_agents.lock() {
                    structured.remove(&context.agent_id);
                }
                if let Some(session_id) = event.agent_session_id.as_deref() {
                    if let Ok(mut sessions) = service.agent_sessions.write() {
                        sessions.remove(&(context.runtime_id.clone(), session_id.to_string()));
                    }
                }
            }
            if event.hook_event_name == "AgentProcessExit" {
                if let Ok(mut agents) = service.agents.write() {
                    agents.retain(|_, value| value.agent_id != context.agent_id);
                }
                if let Ok(mut sessions) = service.agent_sessions.write() {
                    sessions.retain(|_, agent_id| agent_id != &context.agent_id);
                }
                if let Ok(mut structured) = service.structured_agents.lock() {
                    structured.remove(&context.agent_id);
                }
            }
            event
        }
        HookAuthorization::Bootstrap { token, context } => {
            let hook_event_name = payload.get("hook_event_name").and_then(Value::as_str);
            if !matches!(hook_event_name, Some("AgentProcessStart" | "SessionStart")) {
                return Err((
                    StatusCode::UNAUTHORIZED,
                    Json(json!({ "error": "agentNotStarted" })),
                ));
            }
            let identity = if hook_event_name == Some("AgentProcessStart") {
                process_id
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| (format!("{token}:process:{value}"), None))
                    .ok_or_else(|| {
                        (
                            StatusCode::BAD_REQUEST,
                            Json(json!({ "error": "missingProcessId" })),
                        )
                    })?
            } else {
                let agent_session_id = session_id
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| {
                        (
                            StatusCode::BAD_REQUEST,
                            Json(json!({ "error": "missingSessionId" })),
                        )
                    })?;
                (
                    format!("{token}:{agent_session_id}"),
                    Some(agent_session_id.to_string()),
                )
            };
            if service
                .agents
                .read()
                .is_ok_and(|agents| agents.contains_key(&identity.0))
            {
                return Err((
                    StatusCode::CONFLICT,
                    Json(json!({ "error": "agentAlreadyRegistered" })),
                ));
            }
            let agent = TerminalManagedAgentContext {
                mux_session_id: context.mux_session_id,
                pane_id: context.pane_id,
                runtime_id: context.runtime_id,
                agent_id: format!("agent_{}", Uuid::new_v4().simple()),
                launch_profile_id: agent_adapters::automatic_profile_id(
                    agent_adapters::normalize_adapter_id(adapter_id),
                ),
            };
            service
                .agents
                .write()
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": "agentAuthUnavailable" })),
                    )
                })?
                .insert(identity.0, agent.clone());
            if let Some(agent_session_id) = identity.1
                && let Ok(mut sessions) = service.agent_sessions.write()
            {
                sessions.insert(
                    (agent.runtime_id.clone(), agent_session_id),
                    agent.agent_id.clone(),
                );
            }
            service.record(agent, payload)
        }
    };
    Ok(Json(
        hook_response.unwrap_or_else(|| json!({ "sequence": event.sequence })),
    ))
}

fn status_for_hook(
    hook_event_name: &str,
) -> (ManagedAgentStatus, Option<ManagedAgentWaitingReason>) {
    match hook_event_name {
        "PermissionRequest" => (
            ManagedAgentStatus::Waiting,
            Some(ManagedAgentWaitingReason::Permission),
        ),
        "Stop" | "SessionEnd" | "SubagentStop" | "RuntimeExit" | "AgentProcessExit" => {
            (ManagedAgentStatus::Completed, None)
        }
        "HookError" | "InvalidHookPayload" => (ManagedAgentStatus::Error, None),
        _ => (ManagedAgentStatus::Working, None),
    }
}

pub fn try_run_hook_forwarder(args: &[String]) -> Option<i32> {
    if args.get(1).map(String::as_str) != Some("hook") {
        return None;
    }
    let endpoint =
        option_value(args, "--endpoint").or_else(|| std::env::var("LUNA_MUX_HOOK_ENDPOINT").ok());
    let token = option_value(args, "--token")
        .or_else(|| std::env::var("LUNA_MUX_HOOK_AUTHORIZATION").ok())
        .or_else(|| std::env::var("LUNA_MUX_HOOK_TOKEN").ok());
    let (Some(endpoint), Some(token)) = (endpoint, token) else {
        // A persistent user hook also runs for Codex processes that Luna Mux
        // does not manage. Those invocations must remain completely inert.
        return Some(0);
    };
    let mut limited = std::io::stdin().take((MAX_HOOK_BYTES + 1) as u64);
    let mut payload = match serde_json::Deserializer::from_reader(&mut limited)
        .into_iter::<Value>()
        .next()
    {
        Some(Ok(payload)) => payload,
        _ => return Some(2),
    };
    let process_id = std::env::var("LUNA_MUX_AGENT_PROCESS_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("LUNA_MUX_CODEX_PROCESS_ID")
                .ok()
                .filter(|value| !value.trim().is_empty())
        });
    let adapter_id = std::env::var("LUNA_MUX_AGENT_ADAPTER")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| agent_adapters::CODEX_ADAPTER_ID.into());
    let input = {
        if let Some(object) = payload.as_object_mut() {
            if let Some(process_id) = process_id.as_ref() {
                object.insert(
                    "luna_mux_process_id".into(),
                    Value::String(process_id.clone()),
                );
            }
            object.insert("agent_adapter".into(), Value::String(adapter_id.clone()));
        }
        serde_json::to_vec(&payload).unwrap_or_default()
    };
    let client = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(client) => client,
        Err(_) => return Some(0),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    let result = runtime.block_on(async move {
        let response = client
            .post(endpoint)
            .bearer_auth(token)
            .header(AGENT_ADAPTER_HEADER, adapter_id)
            .header(AGENT_PROCESS_ID_HEADER, process_id.unwrap_or_default())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(input)
            .send()
            .await?
            .error_for_status()?;
        let response = response.json::<Value>().await?;
        let has_hook_output =
            response.get("hookSpecificOutput").is_some() || response.get("decision").is_some();
        Ok::<_, reqwest::Error>(has_hook_output.then_some(response))
    });
    Some(match result {
        Ok(Some(response)) => {
            println!("{}", serde_json::to_string(&response).unwrap_or_default());
            0
        }
        // Fail open: Luna Mux tracking/browser routing must never block the
        // agent when the local hook service is unreachable or restarts.
        Ok(None) | Err(_) => 0,
    })
}

fn header_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
}

fn option_value(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    use std::{
        io::Write,
        path::PathBuf,
        process::{Command, Stdio},
    };

    #[test]
    fn hook_events_map_to_deterministic_agent_states() {
        assert_eq!(
            status_for_hook("PermissionRequest"),
            (
                ManagedAgentStatus::Waiting,
                Some(ManagedAgentWaitingReason::Permission)
            )
        );
        assert_eq!(
            status_for_hook("Stop"),
            (ManagedAgentStatus::Completed, None)
        );
        assert_eq!(
            status_for_hook("PostToolUse"),
            (ManagedAgentStatus::Working, None)
        );
        assert_eq!(
            status_for_hook("RuntimeExit"),
            (ManagedAgentStatus::Completed, None)
        );
    }

    #[test]
    fn browser_routing_hook_blocks_browser_launch_and_resource_lifecycle() {
        let shell = browser_routing_hook_response(&json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "shell_command",
            "tool_input": {
                "command": "Start-Process chrome.exe --remote-debugging-port=61993"
            }
        }))
        .expect("browser shell launch should be blocked");
        assert_eq!(shell["hookSpecificOutput"]["permissionDecision"], "deny");

        let new_page = browser_routing_hook_response(&json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "mcp__agent_browser__agent_browser_tab_new",
            "tool_input": { "url": "https://example.com" }
        }));
        assert!(new_page.is_none());

        let named_session = browser_routing_hook_response(&json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "mcp__agent_browser__agent_browser_open",
            "tool_input": {
                "session": "rainj-site",
                "url": "https://example.com"
            }
        }))
        .expect("named browser sessions should be blocked");
        assert_eq!(
            named_session["hookSpecificOutput"]["permissionDecision"],
            "deny"
        );
        assert!(
            named_session["hookSpecificOutput"]["permissionDecisionReason"]
                .as_str()
                .is_some_and(|reason| reason.contains("omit the session argument"))
        );

        let default_session = browser_routing_hook_response(&json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "mcp__agent_browser__agent_browser_open",
            "tool_input": { "url": "https://example.com" }
        }));
        assert!(default_session.is_none());

        let close_page = browser_routing_hook_response(&json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "mcp__agent_browser__agent_browser_tab_close",
            "tool_input": {}
        }));
        assert!(close_page.is_none());

        let close_browser = browser_routing_hook_response(&json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "mcp__agent_browser__agent_browser_close",
            "tool_input": {}
        }));
        assert!(close_browser.is_some());

        let install_browser = browser_routing_hook_response(&json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "mcp__agent_browser__agent_browser_install",
            "tool_input": {}
        }));
        assert!(install_browser.is_some());

        let process_query = browser_routing_hook_response(&json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "shell_command",
            "tool_input": { "command": "Get-Process chrome" }
        }));
        assert!(process_query.is_none());
    }

    #[tokio::test]
    async fn agent_browser_tool_hook_ensures_the_session_browser_before_allowing_the_call() {
        let service = AgentHookService::new();
        service.start().unwrap();
        let ensured_session = Arc::new(Mutex::new(String::new()));
        let observed = ensured_session.clone();
        service.set_browser_ensure(Arc::new(move |mux_session_id| {
            let observed = observed.clone();
            Box::pin(async move {
                *observed.lock().unwrap() = mux_session_id;
                Ok(())
            })
        }));
        let token = service
            .issue_token(TerminalManagedAgentContext {
                mux_session_id: "session-1".into(),
                pane_id: "pane-1".into(),
                runtime_id: "runtime-1".into(),
                agent_id: "agent-1".into(),
                launch_profile_id: "codex.auto".into(),
            })
            .unwrap();
        let response = reqwest::Client::new()
            .post(service.endpoint().unwrap())
            .bearer_auth(token)
            .json(&json!({
                "hook_event_name": "PreToolUse",
                "tool_name": "mcp__agent_browser__agent_browser_snapshot",
                "tool_input": {}
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(*ensured_session.lock().unwrap(), "session-1");
        let body = response.json::<Value>().await.unwrap();
        assert!(body.get("hookSpecificOutput").is_none());
    }

    #[test]
    fn terminal_fallback_signals_are_heuristic_and_store_no_output_text() {
        let service = AgentHookService::new();
        service
            .issue_token(TerminalManagedAgentContext {
                mux_session_id: "session-1".into(),
                pane_id: "pane-1".into(),
                runtime_id: "runtime-1".into(),
                agent_id: "agent-1".into(),
                launch_profile_id: "custom.agent".into(),
            })
            .unwrap();
        service.record_terminal_output(&TerminalRuntimeOutputEvent::new(
            "runtime-1",
            0,
            "private source text\x07",
        ));
        let events = service.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].hook_event_name, "TerminalBell");
        assert_eq!(events[0].status, ManagedAgentStatus::Working);
        assert_eq!(events[0].evidence, ManagedAgentEvidence::TerminalHeuristic);
        let serialized = serde_json::to_string(&events[0]).unwrap();
        assert!(!serialized.contains("private source text"));
    }

    #[test]
    fn structured_hook_supersedes_terminal_fallback_for_an_agent() {
        let service = AgentHookService::new();
        let context = TerminalManagedAgentContext {
            mux_session_id: "session-1".into(),
            pane_id: "pane-1".into(),
            runtime_id: "runtime-1".into(),
            agent_id: "agent-1".into(),
            launch_profile_id: "codex.default".into(),
        };
        service.issue_token(context.clone()).unwrap();
        service.record(context, json!({ "hook_event_name": "SessionStart" }));
        service.record_terminal_output(&TerminalRuntimeOutputEvent::new(
            "runtime-1",
            0,
            "later output\x07",
        ));
        let events = service.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].evidence, ManagedAgentEvidence::StructuredHook);
    }

    #[tokio::test]
    async fn loopback_receiver_authenticates_and_records_hook_json() {
        let service = AgentHookService::new();
        service.start().unwrap();
        let token = service
            .issue_token(TerminalManagedAgentContext {
                mux_session_id: "session-1".into(),
                pane_id: "pane-1".into(),
                runtime_id: "runtime-1".into(),
                agent_id: "agent-1".into(),
                launch_profile_id: "codex.default".into(),
            })
            .unwrap();
        let response = reqwest::Client::new()
            .post(service.endpoint().unwrap())
            .bearer_auth(&token)
            .json(&json!({
                "session_id": "codex-thread-1",
                "turn_id": "turn-1",
                "hook_event_name": "PermissionRequest"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let events = service.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].context.agent_id, "agent-1");
        assert_eq!(events[0].status, ManagedAgentStatus::Waiting);

        let unauthorized = reqwest::Client::new()
            .post(service.endpoint().unwrap())
            .bearer_auth("wrong")
            .json(&json!({ "hook_event_name": "Stop" }))
            .send()
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn revoking_a_runtime_invalidates_only_its_hook_tokens() {
        let service = AgentHookService::new();
        service.start().unwrap();
        let context = |runtime_id: &str, agent_id: &str| TerminalManagedAgentContext {
            mux_session_id: "session-1".into(),
            pane_id: format!("pane-{agent_id}"),
            runtime_id: runtime_id.into(),
            agent_id: agent_id.into(),
            launch_profile_id: "codex.default".into(),
        };
        let revoked = service
            .issue_token(context("runtime-1", "agent-1"))
            .unwrap();
        let retained = service
            .issue_token(context("runtime-2", "agent-2"))
            .unwrap();
        service.revoke_runtime("runtime-1");

        let client = reqwest::Client::new();
        let rejected = client
            .post(service.endpoint().unwrap())
            .bearer_auth(revoked)
            .json(&json!({ "hook_event_name": "Stop" }))
            .send()
            .await
            .unwrap();
        let accepted = client
            .post(service.endpoint().unwrap())
            .bearer_auth(retained)
            .json(&json!({ "hook_event_name": "Stop" }))
            .send()
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(accepted.status(), StatusCode::OK);
        let events = service.events();
        assert_eq!(events[0].hook_event_name, "RuntimeExit");
        assert_eq!(events[0].status, ManagedAgentStatus::Completed);
    }

    #[tokio::test]
    async fn stored_events_exclude_hook_prompt_and_tool_payloads() {
        let service = AgentHookService::new();
        service.start().unwrap();
        let token = service
            .issue_token(TerminalManagedAgentContext {
                mux_session_id: "session-1".into(),
                pane_id: "pane-1".into(),
                runtime_id: "runtime-1".into(),
                agent_id: "agent-1".into(),
                launch_profile_id: "codex.default".into(),
            })
            .unwrap();
        reqwest::Client::new()
            .post(service.endpoint().unwrap())
            .bearer_auth(token)
            .json(&json!({
                "session_id": "codex-session",
                "turn_id": "turn-1",
                "hook_event_name": "UserPromptSubmit",
                "prompt": "private source code",
                "tool_input": { "command": "secret command" }
            }))
            .send()
            .await
            .unwrap();
        let serialized = serde_json::to_value(&service.events()[0]).unwrap();
        assert_eq!(serialized["adapterId"], "codex");
        assert_eq!(serialized["agentSessionId"], "codex-session");
        assert_eq!(serialized["hookEventName"], "UserPromptSubmit");
        assert!(serialized.get("payload").is_none());
        assert!(!serialized.to_string().contains("private source code"));
        assert!(!serialized.to_string().contains("secret command"));
    }

    #[tokio::test]
    async fn runtime_bootstrap_registers_sequential_codex_sessions_without_stale_agents() {
        let service = AgentHookService::new();
        service.start().unwrap();
        let token = service
            .issue_bootstrap_token(TerminalRuntimeContext {
                mux_session_id: "session-1".into(),
                pane_id: "pane-1".into(),
                runtime_id: "runtime-1".into(),
            })
            .unwrap();
        let client = reqwest::Client::new();
        for session_id in ["codex-1", "codex-2"] {
            let started = client
                .post(service.endpoint().unwrap())
                .bearer_auth(&token)
                .json(&json!({ "hook_event_name": "SessionStart", "session_id": session_id }))
                .send()
                .await
                .unwrap();
            assert_eq!(started.status(), StatusCode::OK);
            assert_eq!(service.snapshots().len(), 1);
            let ended = client
                .post(service.endpoint().unwrap())
                .bearer_auth(&token)
                .json(&json!({ "hook_event_name": "SessionEnd", "session_id": session_id }))
                .send()
                .await
                .unwrap();
            assert_eq!(ended.status(), StatusCode::OK);
            assert!(service.snapshots().is_empty());
        }
        let events = service.events();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.hook_event_name == "SessionStart")
                .count(),
            2
        );
        assert_ne!(events[0].context.agent_id, events[2].context.agent_id);
    }

    #[tokio::test]
    async fn process_bootstrap_is_visible_before_codex_session_start_and_cleans_up_on_exit() {
        let service = AgentHookService::new();
        service.start().unwrap();
        let token = service
            .issue_bootstrap_token(TerminalRuntimeContext {
                mux_session_id: "session-1".into(),
                pane_id: "pane-1".into(),
                runtime_id: "runtime-1".into(),
            })
            .unwrap();
        let client = reqwest::Client::new();
        let endpoint = service.endpoint().unwrap();
        let start = client
            .post(&endpoint)
            .bearer_auth(&token)
            .json(&json!({
                "hook_event_name": "AgentProcessStart",
                "luna_mux_process_id": "process-1"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(start.status(), StatusCode::OK);
        let snapshot = service.snapshots();
        assert_eq!(snapshot.len(), 1);
        let agent_id = snapshot[0].context.agent_id.clone();

        let session = client
            .post(&endpoint)
            .bearer_auth(&token)
            .json(&json!({
                "hook_event_name": "SessionStart",
                "session_id": "codex-session-1",
                "luna_mux_process_id": "process-1"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(session.status(), StatusCode::OK);
        assert_eq!(service.snapshots()[0].context.agent_id, agent_id);

        let exit = client
            .post(&endpoint)
            .bearer_auth(&token)
            .json(&json!({
                "hook_event_name": "AgentProcessExit",
                "luna_mux_process_id": "process-1"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(exit.status(), StatusCode::OK);
        assert!(service.snapshots().is_empty());
    }

    #[tokio::test]
    async fn adapter_headers_bind_claude_hooks_to_one_automatic_agent() {
        let service = AgentHookService::new();
        service.start().unwrap();
        let token = service
            .issue_bootstrap_token(TerminalRuntimeContext {
                mux_session_id: "session-1".into(),
                pane_id: "pane-1".into(),
                runtime_id: "runtime-1".into(),
            })
            .unwrap();
        let client = reqwest::Client::new();
        let endpoint = service.endpoint().unwrap();
        let send = |payload: Value| {
            client
                .post(&endpoint)
                .bearer_auth(&token)
                .header(AGENT_ADAPTER_HEADER, "claude-code")
                .header(AGENT_PROCESS_ID_HEADER, "claude-process-1")
                .json(&payload)
                .send()
        };

        let started = send(json!({ "hook_event_name": "AgentProcessStart" }))
            .await
            .unwrap();
        assert_eq!(started.status(), StatusCode::OK);
        let agent_id = service.snapshots()[0].context.agent_id.clone();
        assert_eq!(
            service.snapshots()[0].context.launch_profile_id,
            "claude-code.auto"
        );

        let session = send(json!({
            "hook_event_name": "SessionStart",
            "session_id": "claude-session-1"
        }))
        .await
        .unwrap();
        assert_eq!(session.status(), StatusCode::OK);
        assert_eq!(service.snapshots()[0].context.agent_id, agent_id);
        let latest = service.events().pop().unwrap();
        assert_eq!(latest.adapter_id, "claude-code");
        assert_eq!(latest.agent_session_id.as_deref(), Some("claude-session-1"));

        let exited = send(json!({ "hook_event_name": "AgentProcessExit" }))
            .await
            .unwrap();
        assert_eq!(exited.status(), StatusCode::OK);
        assert!(service.snapshots().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires an installed and authenticated Codex CLI"]
    #[cfg(windows)]
    async fn installed_codex_emits_structured_events_for_a_real_turn() {
        if !Command::new("where.exe")
            .arg("codex.cmd")
            .status()
            .is_ok_and(|status| status.success())
        {
            return;
        }
        let service = AgentHookService::new();
        service.start().unwrap();
        let token = service
            .issue_token(TerminalManagedAgentContext {
                mux_session_id: "real-session".into(),
                pane_id: "real-pane".into(),
                runtime_id: "real-runtime".into(),
                agent_id: "real-agent".into(),
                launch_profile_id: "codex.default".into(),
            })
            .unwrap();
        let executable = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("debug")
            .join("luna-mux.exe");
        assert!(
            executable.exists(),
            "build luna-mux.exe before running this test"
        );
        let endpoint = service.endpoint().unwrap();
        let hook_command = format!("\"{}\" hook", executable.display());
        let hook_command = serde_json::to_string(&hook_command).unwrap();
        let hook_command_windows =
            serde_json::to_string(&format!("& \"{}\" hook", executable.display())).unwrap();
        let handler = format!(
            "[{{hooks=[{{type=\"command\",command={hook_command},commandWindows={hook_command_windows}}}]}}]"
        );
        let mut direct = Command::new(&executable)
            .arg("hook")
            .env("LUNA_MUX_HOOK_ENDPOINT", &endpoint)
            .env("LUNA_MUX_HOOK_AUTHORIZATION", &token)
            .stdin(Stdio::piped())
            .spawn()
            .expect("run the hook forwarder directly");
        direct
            .stdin
            .take()
            .unwrap()
            .write_all(br#"{"hook_event_name":"DirectProbe"}"#)
            .unwrap();
        assert!(direct.wait().unwrap().success());
        assert!(
            service
                .events()
                .iter()
                .any(|event| event.hook_event_name == "DirectProbe")
        );

        let mut sandbox_probe = Command::new("codex.cmd");
        sandbox_probe.args(["--config", "features.network_proxy=true"]);
        sandbox_probe.args(["--config", "network_proxy.domains.\"127.0.0.1\"=\"allow\""]);
        sandbox_probe.args(["sandbox", executable.to_str().unwrap(), "hook"]);
        let mut sandbox_probe = sandbox_probe
            .env("LUNA_MUX_HOOK_ENDPOINT", &endpoint)
            .env("LUNA_MUX_HOOK_AUTHORIZATION", &token)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run the hook forwarder inside the Codex sandbox");
        sandbox_probe
            .stdin
            .take()
            .unwrap()
            .write_all(br#"{"hook_event_name":"SandboxProbe"}"#)
            .unwrap();
        let sandbox_output = sandbox_probe.wait_with_output().unwrap();
        assert!(
            sandbox_output.status.success(),
            "sandboxed probe failed: {}{}",
            String::from_utf8_lossy(&sandbox_output.stdout),
            String::from_utf8_lossy(&sandbox_output.stderr)
        );

        let mut command = Command::new("codex.cmd");
        command.arg("--dangerously-bypass-hook-trust");
        command.args(["--enable", "hooks"]);
        for event in [
            "SessionStart",
            "SessionEnd",
            "UserPromptSubmit",
            "PreToolUse",
            "PermissionRequest",
            "PostToolUse",
            "SubagentStart",
            "SubagentStop",
            "Stop",
        ] {
            command.args(["--config", &format!("hooks.{event}={handler}")]);
        }
        command.args(["--config", "features.network_proxy=true"]);
        command.args(["--config", "network_proxy.domains.\"127.0.0.1\"=\"allow\""]);
        let output = command
            .args([
                "exec",
                "--skip-git-repo-check",
                "Reply with exactly the word OK and do not call tools.",
            ])
            .env("LUNA_MUX_HOOK_ENDPOINT", endpoint)
            .env("LUNA_MUX_HOOK_AUTHORIZATION", token)
            .output()
            .expect("run installed Codex CLI");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(
            output.status.success(),
            "Codex turn failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let events = service.events();
        let diagnostics = format!(
            "stdout:\n{}\nstderr:\n{}\nreceived: {:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
            events
                .iter()
                .map(|event| &event.hook_event_name)
                .collect::<Vec<_>>()
        );
        assert!(
            events
                .iter()
                .all(|event| event.context.agent_id == "real-agent")
        );
        for expected in ["SandboxProbe", "SessionStart", "UserPromptSubmit", "Stop"] {
            assert!(
                events.iter().any(|event| event.hook_event_name == expected),
                "missing {expected}; {diagnostics}"
            );
        }
    }
}
