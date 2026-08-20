use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex, RwLock},
};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use tauri::Emitter;
use uuid::Uuid;

use crate::agent_hooks::AgentHookService;
use crate::browser_runtime::{BrowserRuntime, BrowserRuntimeManager};
use crate::control_approval::ControlApprovalPolicy;
use crate::control_contract::{
    CONTROL_CONTRACT_VERSION, ControlAccess, ControlApproval, ControlApprovalRequirement,
    ControlCaller, ControlCallerKind, ControlCatalog, ControlError, ControlErrorCode, ControlEvent,
    ControlEventReadResult, ControlOperationDescriptor, ControlRequest, ControlResourceKind,
    ControlResponse, ControlResult,
};
use crate::database::Database;
use crate::luna_mcp::LunaMcpService;
use crate::models::{
    ControlAuditRecord, MuxPaneInput, MuxPaneKind, MuxSessionInput, MuxSplitDirection,
    MuxSplitNode, PortForwardProfile, TerminalSettings, TransferDirection, TransferRequest,
    TransferStatus, TransferTask, TunnelSummary, UiTheme,
};
use crate::terminal_backend::TerminalBackend;
use crate::terminal_runtime_contract::TerminalRuntimeEvent;
use crate::transfers::TransferManager;
use crate::tunnels::TunnelManager;

const EVENT_CAPACITY: usize = 2048;

#[derive(Default)]
pub struct ControlEventBuffer {
    inner: Mutex<EventBufferInner>,
}

#[derive(Default)]
struct EventBufferInner {
    next_sequence: u64,
    events: VecDeque<ControlEvent>,
}

impl ControlEventBuffer {
    pub fn record_control_event(
        &self,
        event_type: impl Into<String>,
        resource: Option<crate::control_contract::ControlResourceRef>,
        payload: serde_json::Value,
    ) {
        let mut inner = self.inner.lock().expect("control event buffer lock");
        let sequence = inner.next_sequence;
        inner.next_sequence = inner.next_sequence.saturating_add(1);
        inner.events.push_back(ControlEvent {
            sequence,
            timestamp: chrono::Utc::now().to_rfc3339(),
            event_type: event_type.into(),
            resource,
            payload,
        });
        while inner.events.len() > EVENT_CAPACITY {
            inner.events.pop_front();
        }
    }

    pub fn record_runtime_event(&self, event: TerminalRuntimeEvent) {
        let (event_type, resource, payload) = match event {
            TerminalRuntimeEvent::Status(value) => (
                "terminal.runtime.status",
                Some((
                    ControlResourceKind::TerminalRuntime,
                    value.runtime.runtime_id.clone(),
                )),
                json!(value),
            ),
            TerminalRuntimeEvent::Output(value) => (
                "terminal.runtime.output",
                Some((
                    ControlResourceKind::TerminalRuntime,
                    value.runtime_id.clone(),
                )),
                json!(value),
            ),
            TerminalRuntimeEvent::Exit(value) => (
                "terminal.runtime.exit",
                Some((
                    ControlResourceKind::TerminalRuntime,
                    value.runtime_id.clone(),
                )),
                json!(value),
            ),
        };
        self.record_control_event(
            event_type,
            resource.map(|(kind, id)| crate::control_contract::ControlResourceRef { kind, id }),
            payload,
        );
    }

    fn read(&self, from_sequence: u64, limit: usize) -> ControlEventReadResult {
        let inner = self.inner.lock().expect("control event buffer lock");
        let earliest_sequence = inner
            .events
            .front()
            .map(|e| e.sequence)
            .unwrap_or(inner.next_sequence);
        let requested_sequence = from_sequence;
        let start = from_sequence.max(earliest_sequence);
        let events = inner
            .events
            .iter()
            .filter(|event| event.sequence >= start)
            .take(limit.min(1000))
            .cloned()
            .collect::<Vec<_>>();
        let next_sequence = events
            .last()
            .map(|event| event.sequence.saturating_add(1))
            .unwrap_or(start);
        ControlEventReadResult {
            requested_sequence,
            earliest_sequence,
            next_sequence,
            truncated: from_sequence < earliest_sequence,
            events,
        }
    }
}

pub struct InProcessControlService {
    database: Arc<Database>,
    backend: Arc<dyn TerminalBackend>,
    side_effects: Arc<dyn ControlSideEffects>,
    events: Arc<ControlEventBuffer>,
    approvals: ControlApprovalPolicy,
    agent_hooks: Arc<AgentHookService>,
    luna_mcp: RwLock<Option<Arc<LunaMcpService>>>,
    mux_mutations: tokio::sync::Mutex<()>,
    idempotent_results: Mutex<HashMap<(String, String, String), IdempotencyEntry>>,
}

#[derive(Clone, PartialEq)]
struct IdempotencySignature {
    resource: Option<crate::control_contract::ControlResourceRef>,
    arguments: serde_json::Value,
}

enum IdempotencyEntry {
    InFlight(IdempotencySignature),
    Complete {
        signature: IdempotencySignature,
        response: ControlResponse,
    },
}

type IdempotencyCacheKey = (String, String, String);

#[async_trait]
pub trait ControlSideEffects: Send + Sync {
    async fn settings_set_ui_theme(&self, theme: UiTheme) -> Result<UiTheme, String>;
    async fn settings_set_terminal(
        &self,
        settings: TerminalSettings,
    ) -> Result<TerminalSettings, String>;
    async fn transfer_list(&self, runtime_id: &str) -> Result<Vec<TransferTask>, String>;
    async fn tunnel_list(&self, runtime_id: &str) -> Result<Vec<TunnelSummary>, String>;
    async fn notify_state_changed(
        &self,
        change_type: &str,
        payload: serde_json::Value,
    ) -> Result<(), String>;
    async fn transfer_enqueue(&self, request: TransferRequest)
    -> Result<Vec<TransferTask>, String>;
    async fn transfer_cancel(&self, transfer_id: &str) -> Result<bool, String>;
    async fn tunnel_start(
        &self,
        runtime_id: &str,
        profile_id: &str,
    ) -> Result<TunnelSummary, String>;
    async fn tunnel_stop(&self, tunnel_id: &str) -> Result<bool, String>;
    async fn browser_navigate(&self, runtime_id: &str, url: &str) -> Result<(), String>;
    async fn browser_snapshot(&self, runtime_id: &str) -> Result<serde_json::Value, String>;
    async fn browser_screenshot(&self, runtime_id: &str) -> Result<String, String>;
    async fn browser_click(&self, runtime_id: &str, selector: &str) -> Result<bool, String>;
    async fn browser_type(
        &self,
        runtime_id: &str,
        selector: &str,
        text: &str,
        clear: bool,
    ) -> Result<bool, String>;
    async fn browser_press(&self, runtime_id: &str, key: &str) -> Result<(), String>;
    async fn browser_scroll(
        &self,
        runtime_id: &str,
        delta_x: f64,
        delta_y: f64,
        x: f64,
        y: f64,
    ) -> Result<(), String>;
    async fn browser_evaluate(
        &self,
        runtime_id: &str,
        expression: &str,
    ) -> Result<serde_json::Value, String>;
    async fn browser_wait(
        &self,
        runtime_id: &str,
        selector: &str,
        timeout_ms: u64,
    ) -> Result<bool, String>;
    async fn browser_resize(&self, runtime_id: &str, width: u32, height: u32)
    -> Result<(), String>;
    async fn browser_close(&self, runtime_id: &str) -> Result<(), String>;
    async fn browser_runtimes_list(&self) -> Result<Vec<BrowserRuntime>, String>;
    async fn diagnostics_repair(
        &self,
        _runtime_id: &str,
        _action: &str,
    ) -> Result<serde_json::Value, String> {
        Err("当前运行环境不支持诊断修复".into())
    }
    async fn diagnostics_remote_helper(
        &self,
        _runtime_id: &str,
        _context_runtime_id: &str,
    ) -> Result<(bool, Option<String>), String> {
        Ok((false, None))
    }
    async fn diagnostics_browser_runtime(
        &self,
        _mux_session_id: &str,
    ) -> Result<Option<String>, String> {
        Ok(None)
    }
}

pub struct InProcessControlSideEffects {
    window: tauri::WebviewWindow,
    database: Arc<Database>,
    transfers: Arc<TransferManager>,
    tunnels: Arc<TunnelManager>,
    browser_runtimes: Arc<BrowserRuntimeManager>,
    sessions: Arc<crate::sessions::SessionManager>,
}

impl InProcessControlSideEffects {
    pub fn new(
        window: tauri::WebviewWindow,
        database: Arc<Database>,
        transfers: Arc<TransferManager>,
        tunnels: Arc<TunnelManager>,
        browser_runtimes: Arc<BrowserRuntimeManager>,
        sessions: Arc<crate::sessions::SessionManager>,
    ) -> Arc<Self> {
        Arc::new(Self {
            window,
            database,
            transfers,
            tunnels,
            browser_runtimes,
            sessions,
        })
    }
}

#[async_trait]
impl ControlSideEffects for InProcessControlSideEffects {
    async fn settings_set_ui_theme(&self, theme: UiTheme) -> Result<UiTheme, String> {
        crate::commands::save_ui_theme(&self.window, &self.database, theme)
    }

    async fn settings_set_terminal(
        &self,
        settings: TerminalSettings,
    ) -> Result<TerminalSettings, String> {
        crate::commands::save_terminal_settings(&self.database, Some(&self.window), settings)
    }

    async fn transfer_list(&self, runtime_id: &str) -> Result<Vec<TransferTask>, String> {
        Ok(self
            .transfers
            .list()?
            .into_iter()
            .filter(|task| task.session_id == runtime_id)
            .collect())
    }

    async fn tunnel_list(&self, runtime_id: &str) -> Result<Vec<TunnelSummary>, String> {
        Ok(self.tunnels.list(runtime_id))
    }

    async fn notify_state_changed(
        &self,
        change_type: &str,
        payload: serde_json::Value,
    ) -> Result<(), String> {
        self.window
            .emit(
                "control-state:changed",
                json!({ "type": change_type, "payload": payload }),
            )
            .map_err(|error| error.to_string())
    }

    async fn transfer_enqueue(
        &self,
        request: TransferRequest,
    ) -> Result<Vec<TransferTask>, String> {
        self.transfers.enqueue(request)
    }

    async fn transfer_cancel(&self, transfer_id: &str) -> Result<bool, String> {
        let task = self
            .transfers
            .list()?
            .into_iter()
            .find(|task| task.id == transfer_id)
            .ok_or_else(|| "传输任务不存在".to_string())?;
        let active = matches!(
            task.status,
            TransferStatus::Queued
                | TransferStatus::Scanning
                | TransferStatus::Running
                | TransferStatus::Conflict
        );
        if active {
            self.transfers.cancel(transfer_id);
        }
        Ok(active)
    }

    async fn tunnel_start(
        &self,
        runtime_id: &str,
        profile_id: &str,
    ) -> Result<TunnelSummary, String> {
        let profile = self
            .database
            .get_setting::<Vec<PortForwardProfile>>("portForwardProfiles", vec![])
            .into_iter()
            .find(|profile| profile.id == profile_id)
            .ok_or_else(|| "端口转发配置不存在".to_string())?;
        self.tunnels.start(runtime_id.to_string(), profile).await
    }

    async fn tunnel_stop(&self, tunnel_id: &str) -> Result<bool, String> {
        let Some(tunnel) = self.tunnels.get(tunnel_id) else {
            return Ok(false);
        };
        self.tunnels.stop(&tunnel.session_id, tunnel_id).await?;
        Ok(true)
    }

    async fn browser_navigate(&self, runtime_id: &str, url: &str) -> Result<(), String> {
        self.browser_runtimes.navigate(runtime_id, url)
    }

    async fn browser_snapshot(&self, runtime_id: &str) -> Result<serde_json::Value, String> {
        self.browser_runtimes.snapshot(runtime_id).await
    }

    async fn browser_screenshot(&self, runtime_id: &str) -> Result<String, String> {
        self.browser_runtimes.screenshot(runtime_id).await
    }

    async fn browser_click(&self, runtime_id: &str, selector: &str) -> Result<bool, String> {
        self.browser_runtimes.click(runtime_id, selector).await
    }

    async fn browser_type(
        &self,
        runtime_id: &str,
        selector: &str,
        text: &str,
        clear: bool,
    ) -> Result<bool, String> {
        self.browser_runtimes
            .type_text(runtime_id, selector, text, clear)
            .await
    }

    async fn browser_press(&self, runtime_id: &str, key: &str) -> Result<(), String> {
        self.browser_runtimes.press(runtime_id, key)
    }

    async fn browser_scroll(
        &self,
        runtime_id: &str,
        delta_x: f64,
        delta_y: f64,
        x: f64,
        y: f64,
    ) -> Result<(), String> {
        self.browser_runtimes
            .scroll(runtime_id, delta_x, delta_y, x, y)
    }

    async fn browser_evaluate(
        &self,
        runtime_id: &str,
        expression: &str,
    ) -> Result<serde_json::Value, String> {
        self.browser_runtimes.evaluate(runtime_id, expression).await
    }

    async fn browser_wait(
        &self,
        runtime_id: &str,
        selector: &str,
        timeout_ms: u64,
    ) -> Result<bool, String> {
        self.browser_runtimes
            .wait_for_selector(runtime_id, selector, timeout_ms)
            .await
    }

    async fn browser_resize(
        &self,
        runtime_id: &str,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        self.browser_runtimes.resize(runtime_id, width, height)
    }

    async fn browser_close(&self, runtime_id: &str) -> Result<(), String> {
        self.browser_runtimes.close(runtime_id).await
    }

    async fn browser_runtimes_list(&self) -> Result<Vec<BrowserRuntime>, String> {
        self.browser_runtimes.list()
    }

    async fn diagnostics_repair(
        &self,
        runtime_id: &str,
        action: &str,
    ) -> Result<serde_json::Value, String> {
        if action != "remoteHelper" {
            return Err("不支持的诊断修复操作".into());
        }
        let path = self
            .sessions
            .install_remote_agent_helper(runtime_id)
            .await?;
        Ok(
            json!({ "action": action, "runtimeId": runtime_id, "path": path, "requiresAgentRestart": true }),
        )
    }

    async fn diagnostics_remote_helper(
        &self,
        runtime_id: &str,
        context_runtime_id: &str,
    ) -> Result<(bool, Option<String>), String> {
        self.sessions
            .remote_agent_diagnostic(runtime_id, context_runtime_id)
            .await
    }

    async fn diagnostics_browser_runtime(
        &self,
        mux_session_id: &str,
    ) -> Result<Option<String>, String> {
        Ok(self
            .browser_runtimes
            .list()?
            .into_iter()
            .find(|runtime| runtime.mux_session_id == mux_session_id)
            .map(|runtime| format!("{:?} cdp={}", runtime.status, runtime.cdp_port)))
    }
}

impl InProcessControlService {
    pub fn new(
        database: Arc<Database>,
        backend: Arc<dyn TerminalBackend>,
        side_effects: Arc<dyn ControlSideEffects>,
        agent_hooks: Arc<AgentHookService>,
    ) -> Arc<Self> {
        Arc::new(Self {
            database,
            backend,
            side_effects,
            events: Arc::new(ControlEventBuffer::default()),
            approvals: ControlApprovalPolicy::default(),
            agent_hooks,
            luna_mcp: RwLock::new(None),
            mux_mutations: tokio::sync::Mutex::new(()),
            idempotent_results: Mutex::new(HashMap::new()),
        })
    }

    pub fn set_luna_mcp(&self, luna_mcp: Arc<LunaMcpService>) {
        *self.luna_mcp.write().expect("luna mcp diagnostics lock") = Some(luna_mcp);
    }

    pub fn event_buffer(&self) -> Arc<ControlEventBuffer> {
        self.events.clone()
    }

    fn idempotency_lookup(
        &self,
        key: &IdempotencyCacheKey,
        signature: &IdempotencySignature,
    ) -> ControlResult<Option<ControlResponse>> {
        let entries = self
            .idempotent_results
            .lock()
            .map_err(|_| internal_error("控制幂等状态锁已损坏"))?;
        match entries.get(key) {
            None => Ok(None),
            Some(IdempotencyEntry::Complete {
                signature: existing,
                response,
            }) if existing == signature => Ok(Some(response.clone())),
            Some(IdempotencyEntry::InFlight(existing)) if existing == signature => {
                Err(ControlError {
                    code: ControlErrorCode::Conflict,
                    message: "相同幂等请求正在执行".into(),
                    retryable: true,
                    details: Some(json!({ "status": "inFlight" })),
                })
            }
            Some(_) => Err(ControlError {
                code: ControlErrorCode::Conflict,
                message: "幂等键已绑定到不同的资源或参数".into(),
                retryable: false,
                details: None,
            }),
        }
    }

    fn idempotency_reserve(
        &self,
        key: &IdempotencyCacheKey,
        signature: &IdempotencySignature,
    ) -> ControlResult<Option<ControlResponse>> {
        let mut entries = self
            .idempotent_results
            .lock()
            .map_err(|_| internal_error("控制幂等状态锁已损坏"))?;
        match entries.get(key) {
            None => {
                entries.insert(key.clone(), IdempotencyEntry::InFlight(signature.clone()));
                Ok(None)
            }
            Some(IdempotencyEntry::Complete {
                signature: existing,
                response,
            }) if existing == signature => Ok(Some(response.clone())),
            Some(IdempotencyEntry::InFlight(existing)) if existing == signature => {
                Err(ControlError {
                    code: ControlErrorCode::Conflict,
                    message: "相同幂等请求正在执行".into(),
                    retryable: true,
                    details: Some(json!({ "status": "inFlight" })),
                })
            }
            Some(_) => Err(ControlError {
                code: ControlErrorCode::Conflict,
                message: "幂等键已绑定到不同的资源或参数".into(),
                retryable: false,
                details: None,
            }),
        }
    }

    fn idempotency_complete(
        &self,
        key: IdempotencyCacheKey,
        signature: IdempotencySignature,
        response: ControlResponse,
    ) {
        self.idempotent_results
            .lock()
            .expect("control idempotency lock")
            .insert(
                key,
                IdempotencyEntry::Complete {
                    signature,
                    response,
                },
            );
    }

    fn idempotency_forget(&self, key: &IdempotencyCacheKey) {
        self.idempotent_results
            .lock()
            .expect("control idempotency lock")
            .remove(key);
    }

    fn descriptors() -> Vec<ControlOperationDescriptor> {
        vec![
            ControlOperationDescriptor {
                name: "settings.appearance.get".into(),
                version: 1,
                access: ControlAccess::Read,
                resource_kind: ControlResourceKind::Settings,
                mutating: false,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "settings.theme.set".into(),
                version: 1,
                access: ControlAccess::Write,
                resource_kind: ControlResourceKind::Settings,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "settings.terminal.set".into(),
                version: 1,
                access: ControlAccess::Write,
                resource_kind: ControlResourceKind::Settings,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "diagnostics.run".into(),
                version: 1,
                access: ControlAccess::Read,
                resource_kind: ControlResourceKind::Settings,
                mutating: false,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "diagnostics.repair".into(),
                version: 1,
                access: ControlAccess::Write,
                resource_kind: ControlResourceKind::TerminalRuntime,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "connections.list".into(),
                version: 1,
                access: ControlAccess::Read,
                resource_kind: ControlResourceKind::ConnectionProfile,
                mutating: false,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "agents.list".into(),
                version: 1,
                access: ControlAccess::Read,
                resource_kind: ControlResourceKind::Agent,
                mutating: false,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "agents.get_status".into(),
                version: 1,
                access: ControlAccess::Read,
                resource_kind: ControlResourceKind::Agent,
                mutating: false,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "agents.send_task".into(),
                version: 1,
                access: ControlAccess::Write,
                resource_kind: ControlResourceKind::Agent,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "agents.interrupt".into(),
                version: 1,
                access: ControlAccess::Control,
                resource_kind: ControlResourceKind::Agent,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "mux.sessions.list".into(),
                version: 1,
                access: ControlAccess::Read,
                resource_kind: ControlResourceKind::MuxSession,
                mutating: false,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "mux.session.update".into(),
                version: 1,
                access: ControlAccess::Write,
                resource_kind: ControlResourceKind::MuxSession,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "mux.layout.set".into(),
                version: 1,
                access: ControlAccess::Write,
                resource_kind: ControlResourceKind::MuxSession,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "mux.panes.list".into(),
                version: 1,
                access: ControlAccess::Read,
                resource_kind: ControlResourceKind::Pane,
                mutating: false,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "mux.pane.create".into(),
                version: 1,
                access: ControlAccess::Write,
                resource_kind: ControlResourceKind::MuxSession,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "mux.pane.update".into(),
                version: 1,
                access: ControlAccess::Write,
                resource_kind: ControlResourceKind::Pane,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "terminal.targets.list".into(),
                version: 1,
                access: ControlAccess::Read,
                resource_kind: ControlResourceKind::TerminalTarget,
                mutating: false,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "terminal.runtimes.list".into(),
                version: 1,
                access: ControlAccess::Read,
                resource_kind: ControlResourceKind::TerminalRuntime,
                mutating: false,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "terminal.runtime.output.read".into(),
                version: 1,
                access: ControlAccess::Read,
                resource_kind: ControlResourceKind::TerminalRuntime,
                mutating: false,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "terminal.runtime.write".into(),
                version: 1,
                access: ControlAccess::Write,
                resource_kind: ControlResourceKind::TerminalRuntime,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "terminal.runtime.resize".into(),
                version: 1,
                access: ControlAccess::Write,
                resource_kind: ControlResourceKind::TerminalRuntime,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "terminal.runtime.flow.set".into(),
                version: 1,
                access: ControlAccess::Write,
                resource_kind: ControlResourceKind::TerminalRuntime,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "terminal.runtime.interrupt".into(),
                version: 1,
                access: ControlAccess::Control,
                resource_kind: ControlResourceKind::TerminalRuntime,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "terminal.runtime.close".into(),
                version: 1,
                access: ControlAccess::Control,
                resource_kind: ControlResourceKind::TerminalRuntime,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::User,
            },
            ControlOperationDescriptor {
                name: "transfer.enqueue".into(),
                version: 1,
                access: ControlAccess::Control,
                resource_kind: ControlResourceKind::TerminalRuntime,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::User,
            },
            ControlOperationDescriptor {
                name: "transfers.list".into(),
                version: 1,
                access: ControlAccess::Read,
                resource_kind: ControlResourceKind::TerminalRuntime,
                mutating: false,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "transfer.cancel".into(),
                version: 1,
                access: ControlAccess::Control,
                resource_kind: ControlResourceKind::Transfer,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::User,
            },
            ControlOperationDescriptor {
                name: "tunnel.start".into(),
                version: 1,
                access: ControlAccess::Control,
                resource_kind: ControlResourceKind::TerminalRuntime,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::User,
            },
            ControlOperationDescriptor {
                name: "tunnels.list".into(),
                version: 1,
                access: ControlAccess::Read,
                resource_kind: ControlResourceKind::TerminalRuntime,
                mutating: false,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "tunnel.profiles.list".into(),
                version: 1,
                access: ControlAccess::Read,
                resource_kind: ControlResourceKind::TerminalRuntime,
                mutating: false,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "tunnel.stop".into(),
                version: 1,
                access: ControlAccess::Control,
                resource_kind: ControlResourceKind::Tunnel,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::User,
            },
            ControlOperationDescriptor {
                name: "browser.runtimes.list".into(),
                version: 1,
                access: ControlAccess::Read,
                resource_kind: ControlResourceKind::BrowserRuntime,
                mutating: false,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "browser.navigate".into(),
                version: 1,
                access: ControlAccess::Write,
                resource_kind: ControlResourceKind::BrowserRuntime,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::User,
            },
            ControlOperationDescriptor {
                name: "browser.snapshot".into(),
                version: 1,
                access: ControlAccess::Read,
                resource_kind: ControlResourceKind::BrowserRuntime,
                mutating: false,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "browser.screenshot".into(),
                version: 1,
                access: ControlAccess::Read,
                resource_kind: ControlResourceKind::BrowserRuntime,
                mutating: false,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "browser.click".into(),
                version: 1,
                access: ControlAccess::Write,
                resource_kind: ControlResourceKind::BrowserRuntime,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::User,
            },
            ControlOperationDescriptor {
                name: "browser.type".into(),
                version: 1,
                access: ControlAccess::Write,
                resource_kind: ControlResourceKind::BrowserRuntime,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::User,
            },
            ControlOperationDescriptor {
                name: "browser.press".into(),
                version: 1,
                access: ControlAccess::Write,
                resource_kind: ControlResourceKind::BrowserRuntime,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::User,
            },
            ControlOperationDescriptor {
                name: "browser.scroll".into(),
                version: 1,
                access: ControlAccess::Write,
                resource_kind: ControlResourceKind::BrowserRuntime,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::User,
            },
            ControlOperationDescriptor {
                name: "browser.evaluate".into(),
                version: 1,
                access: ControlAccess::Write,
                resource_kind: ControlResourceKind::BrowserRuntime,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::User,
            },
            ControlOperationDescriptor {
                name: "browser.wait".into(),
                version: 1,
                access: ControlAccess::Read,
                resource_kind: ControlResourceKind::BrowserRuntime,
                mutating: false,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "browser.resize".into(),
                version: 1,
                access: ControlAccess::Write,
                resource_kind: ControlResourceKind::BrowserRuntime,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            },
            ControlOperationDescriptor {
                name: "browser.close".into(),
                version: 1,
                access: ControlAccess::Control,
                resource_kind: ControlResourceKind::BrowserRuntime,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::User,
            },
        ]
    }

    fn descriptor(operation: &str) -> Option<ControlOperationDescriptor> {
        Self::descriptors()
            .into_iter()
            .find(|item| item.name == operation)
    }

    fn authorize(
        caller: &ControlCaller,
        descriptor: &ControlOperationDescriptor,
        resource_id: Option<&str>,
    ) -> ControlResult<()> {
        if caller.has_access(&descriptor.resource_kind, resource_id, &descriptor.access)
            || (resource_id.is_none()
                && caller.can_access_any(&descriptor.resource_kind, &descriptor.access))
        {
            Ok(())
        } else {
            Err(ControlError {
                code: ControlErrorCode::Unauthorized,
                message: "调用者没有该资源的访问权限".into(),
                retryable: false,
                details: None,
            })
        }
    }

    fn validate_request(request: &ControlRequest) -> ControlResult<()> {
        if request.contract_version != CONTROL_CONTRACT_VERSION {
            return Err(ControlError {
                code: ControlErrorCode::UnsupportedVersion,
                message: format!("不支持控制契约版本 {}", request.contract_version),
                retryable: false,
                details: None,
            });
        }
        Ok(())
    }

    pub fn resolve_approval(
        &self,
        approval_id: &str,
        approved: bool,
    ) -> ControlResult<ControlApproval> {
        let approval = self.approvals.resolve(approval_id, approved)?;
        self.events.record_control_event(
            if approved {
                "control.approval.approved"
            } else {
                "control.approval.denied"
            },
            approval.resource.clone(),
            json!({
                "approvalId": approval.approval_id,
                "callerId": approval.caller_id,
                "callerKind": approval.caller_kind,
                "operation": approval.operation,
            }),
        );
        Ok(approval)
    }
}

fn internal_ui_caller() -> ControlCaller {
    use ControlResourceKind::*;
    ControlCaller {
        caller_id: "ui".into(),
        kind: ControlCallerKind::Ui,
        grants: [
            Application,
            Settings,
            ConnectionProfile,
            MuxSession,
            Pane,
            TerminalTarget,
            TerminalRuntime,
            Agent,
            BrowserRuntime,
            Transfer,
            Tunnel,
        ]
        .into_iter()
        .map(|resource_kind| crate::control_contract::ControlGrant {
            resource_kind,
            resource_id: None,
            access: ControlAccess::Control,
        })
        .collect(),
    }
}

pub fn ui_caller() -> ControlCaller {
    internal_ui_caller()
}

/// Transport-neutral application control boundary. Tauri, MCP, and future CLI
/// adapters must authenticate callers before invoking this service.
#[async_trait]
pub trait LunaControlService: Send + Sync {
    fn catalog(&self, caller: &ControlCaller) -> ControlCatalog;

    async fn invoke(
        &self,
        caller: &ControlCaller,
        request: ControlRequest,
    ) -> ControlResult<ControlResponse>;

    fn read_events(
        &self,
        caller: &ControlCaller,
        from_sequence: u64,
        limit: usize,
    ) -> ControlResult<ControlEventReadResult>;
}

#[async_trait]
impl LunaControlService for InProcessControlService {
    fn catalog(&self, caller: &ControlCaller) -> ControlCatalog {
        let operations = Self::descriptors()
            .into_iter()
            .filter(|descriptor| {
                caller.can_access_any(&descriptor.resource_kind, &descriptor.access)
                    || (caller.kind == ControlCallerKind::Agent
                        && descriptor.resource_kind == ControlResourceKind::BrowserRuntime)
            })
            .collect();
        ControlCatalog {
            contract_version: CONTROL_CONTRACT_VERSION,
            operations,
        }
    }

    async fn invoke(
        &self,
        caller: &ControlCaller,
        request: ControlRequest,
    ) -> ControlResult<ControlResponse> {
        let audit_operation = request.operation.clone();
        let audit_request_id = request.request_id.clone();
        let audit_resource = request.resource.clone();
        let audit_arguments = audit_arguments(&request);
        let outcome = async {
            Self::validate_request(&request)?;
            let descriptor = Self::descriptor(&request.operation).ok_or_else(|| ControlError {
                code: ControlErrorCode::UnknownOperation,
                message: format!("未知控制操作：{}", request.operation),
                retryable: false,
                details: None,
            })?;
            let requested_id = request
                .resource
                .as_ref()
                .map(|resource| resource.id.as_str());
            if let Some(resource) = &request.resource {
                if resource.kind != descriptor.resource_kind {
                    return Err(ControlError {
                        code: ControlErrorCode::InvalidArguments,
                        message: "操作资源类型不匹配".into(),
                        retryable: false,
                        details: None,
                    });
                }
            }
            if !(caller.kind == ControlCallerKind::Agent
                && request.operation == "browser.runtimes.list"
                && requested_id.is_none())
            {
                Self::authorize(caller, &descriptor, requested_id)?;
            }
            if !request.arguments.is_object() {
                return Err(ControlError {
                    code: ControlErrorCode::InvalidArguments,
                    message: "arguments 必须是 JSON 对象".into(),
                    retryable: false,
                    details: None,
                });
            }
            let idempotency_key = request
                .idempotency_key
                .as_deref()
                .filter(|key| !key.is_empty());
            let idempotency_cache_key = idempotency_key.map(|key| {
                (
                    caller.caller_id.clone(),
                    request.operation.clone(),
                    key.to_string(),
                )
            });
            let idempotency_signature = IdempotencySignature {
                resource: request.resource.clone(),
                arguments: request.arguments.clone(),
            };
            if idempotency_key.is_some() {
                if !descriptor.supports_idempotency {
                    return Err(ControlError {
                        code: ControlErrorCode::InvalidArguments,
                        message: "该操作不支持幂等键".into(),
                        retryable: false,
                        details: None,
                    });
                }
                let cache_key = idempotency_cache_key
                    .as_ref()
                    .expect("idempotency cache key");
                if let Some(response) =
                    self.idempotency_lookup(cache_key, &idempotency_signature)?
                {
                    return Ok(response);
                }
            }
            if descriptor.approval == ControlApprovalRequirement::User
                && !matches!(
                    caller.kind,
                    ControlCallerKind::Ui | ControlCallerKind::Internal
                )
            {
                if let Some(approval_id) = request.approval_id.as_deref() {
                    self.approvals.consume(caller, &request, approval_id)?;
                } else {
                    let approval = self.approvals.request(caller, &request)?;
                    self.events.record_control_event(
                        "control.approval.requested",
                        request.resource.clone(),
                        json!({
                            "approvalId": approval.approval_id,
                            "callerId": approval.caller_id,
                            "callerKind": approval.caller_kind,
                            "operation": approval.operation,
                            "expiresAt": approval.expires_at,
                        }),
                    );
                    return Err(ControlError {
                        code: ControlErrorCode::ApprovalRequired,
                        message: "该控制操作需要用户批准".into(),
                        retryable: true,
                        details: Some(json!({ "approval": approval })),
                    });
                }
            }
            if let Some(cache_key) = idempotency_cache_key.as_ref() {
                if let Some(response) =
                    self.idempotency_reserve(cache_key, &idempotency_signature)?
                {
                    return Ok(response);
                }
            }
            let execution: ControlResult<serde_json::Value> = async {
                Ok(match request.operation.as_str() {
                    "settings.appearance.get" => {
                        require_empty_arguments(&request)?;
                        let theme = self.database.get_setting("uiTheme", UiTheme::default());
                        let terminal = self
                            .database
                            .get_setting("terminalSettings", TerminalSettings::default());
                        let language = self.database.get_setting("language", "zh-CN".to_string());
                        let remote_agent_integration_enabled = self
                            .database
                            .get_setting("remoteAgentIntegrationEnabled", false);
                        json!({
                            "theme": theme,
                            "terminal": terminal,
                            "language": language,
                            "remoteAgentIntegrationEnabled": remote_agent_integration_enabled
                        })
                    }
                    "settings.theme.set" => {
                        let arguments: ThemeSetArguments = parse_arguments(&request)?;
                        let theme = self
                            .side_effects
                            .settings_set_ui_theme(arguments.theme)
                            .await
                            .map_err(internal_error)?;
                        json!({ "theme": theme })
                    }
                    "settings.terminal.set" => {
                        let settings: TerminalSettings = parse_arguments(&request)?;
                        let settings = self
                            .side_effects
                            .settings_set_terminal(settings)
                            .await
                            .map_err(internal_error)?;
                        serde_json::to_value(settings)
                            .map_err(|error| internal_error(error.to_string()))?
                    }
                    "diagnostics.run" => {
                        let arguments: DiagnosticsRunArguments = parse_arguments(&request)?;
                        let filter = arguments.filter.filter(|value| !value.trim().is_empty());
                        let pane_titles = self
                            .database
                            .list_mux_panes(None)
                            .map(|panes| {
                                panes
                                    .into_iter()
                                    .map(|pane| (pane.id, pane.title))
                                    .collect::<HashMap<_, _>>()
                            })
                            .unwrap_or_default();
                        let session_names = self
                            .database
                            .list_mux_sessions()
                            .map(|sessions| {
                                sessions
                                    .into_iter()
                                    .map(|session| (session.id, session.name))
                                    .collect::<HashMap<_, _>>()
                            })
                            .unwrap_or_default();
                        let managed_agents = self
                            .agent_hooks
                            .snapshots()
                            .into_iter()
                            .map(|snapshot| {
                                let adapter = crate::agent_adapters::adapter_id_for_profile(
                                    &snapshot.context.launch_profile_id,
                                )
                                .unwrap_or("unknown")
                                .to_string();
                                let pane_id = snapshot.context.pane_id;
                                let pane_title =
                                    pane_titles.get(&pane_id).cloned().unwrap_or_default();
                                let mux_session_id = snapshot.context.mux_session_id;
                                let session_name = session_names
                                    .get(&mux_session_id)
                                    .cloned()
                                    .unwrap_or_default();
                                crate::doctor::DoctorManagedAgent {
                                    agent_id: snapshot.context.agent_id,
                                    adapter,
                                    runtime_id: snapshot.context.runtime_id,
                                    pane_id,
                                    pane_title,
                                    mux_session_id,
                                    session_name,
                                    status: format!("{:?}", snapshot.status),
                                    last_activity: snapshot.last_activity,
                                }
                            })
                            .collect::<Vec<_>>();
                        let mut runtime_inputs = self
                            .backend
                            .list()
                            .unwrap_or_default()
                            .into_iter()
                            .filter_map(|runtime| {
                                let context = runtime.context.clone().or_else(|| {
                                    runtime.managed_agent.as_ref().map(|agent| {
                                        crate::terminal_runtime_contract::TerminalRuntimeContext {
                                            mux_session_id: agent.mux_session_id.clone(),
                                            pane_id: agent.pane_id.clone(),
                                            runtime_id: agent.runtime_id.clone(),
                                        }
                                    })
                                })?;
                                Some(crate::doctor::DoctorRuntimeInput {
                                    runtime_id: runtime.runtime_id.clone(),
                                    target_id: runtime.target_id.clone(),
                                    title: runtime.title.clone(),
                                    status: format!("{:?}", runtime.status),
                                    pane_id: Some(context.pane_id),
                                    pane_title: None,
                                    mux_session_id: Some(context.mux_session_id),
                                    hook_endpoint: self.agent_hooks.endpoint().ok(),
                                    hook_token: self
                                        .agent_hooks
                                        .diagnostic_token_for_runtime(&runtime.runtime_id),
                                    mcp_endpoint: self.luna_mcp.read().ok().and_then(|service| {
                                        service.as_ref().and_then(|service| service.endpoint().ok())
                                    }),
                                    mcp_token: self.luna_mcp.read().ok().and_then(|service| {
                                        service.as_ref().and_then(|service| {
                                            service
                                                .diagnostic_token_for_runtime(&runtime.runtime_id)
                                        })
                                    }),
                                    remote_helper_exists: None,
                                    remote_helper_log: None,
                                    remote_bridge_log: None,
                                    integration_enabled: !runtime
                                        .target_id
                                        .starts_with("ssh-bookmark:")
                                        || self
                                            .database
                                            .get_setting("remoteAgentIntegrationEnabled", false),
                                    browser_runtime: None,
                                })
                            })
                            .collect::<Vec<_>>();
                        for input in &mut runtime_inputs {
                            if input.target_id.starts_with("ssh-bookmark:") {
                                let (exists, log) = self
                                    .side_effects
                                    .diagnostics_remote_helper(&input.runtime_id, &input.runtime_id)
                                    .await
                                    .unwrap_or((false, None));
                                input.remote_helper_exists = Some(exists);
                                input.remote_helper_log = log;
                            }
                            if let Some(session_id) = input.mux_session_id.as_deref() {
                                if let Ok(Some(browser)) = self
                                    .side_effects
                                    .diagnostics_browser_runtime(session_id)
                                    .await
                                {
                                    input.browser_runtime = Some(browser);
                                }
                            }
                        }
                        let report = tauri::async_runtime::spawn_blocking(move || {
                            crate::doctor::run_report_with_runtime_inputs(
                                filter.as_deref(),
                                &managed_agents,
                                &runtime_inputs,
                            )
                        })
                        .await
                        .map_err(|error| internal_error(error.to_string()))?;
                        serde_json::to_value(report)
                            .map_err(|error| internal_error(error.to_string()))?
                    }
                    "diagnostics.repair" => {
                        let arguments: DiagnosticsRepairArguments = parse_arguments(&request)?;
                        let runtime_id = requested_id.ok_or_else(|| {
                            invalid_arguments(
                                "diagnostics.repair requires a terminal runtime resource",
                            )
                        })?;
                        let result = self
                            .side_effects
                            .diagnostics_repair(runtime_id, &arguments.action)
                            .await
                            .map_err(internal_error)?;
                        result
                    }
                    "connections.list" => {
                        require_empty_arguments(&request)?;
                        let connections = self
                            .database
                            .list_bookmarks()
                            .map_err(internal_error)?
                            .into_iter()
                            .map(|bookmark| {
                                json!({
                                    "id": bookmark.id,
                                    "name": bookmark.name,
                                    "host": bookmark.host,
                                    "port": bookmark.port,
                                    "username": bookmark.username,
                                    "authType": bookmark.auth_type,
                                    "groupName": bookmark.group_name,
                                    "favorite": bookmark.favorite,
                                    "jumpConnectionId": bookmark.jump_bookmark_id,
                                    "keepaliveEnabled": bookmark.keepalive_enabled,
                                    "lastConnectedAt": bookmark.last_connected_at,
                                    "hasSavedCredential": bookmark.has_saved_credential
                                })
                            })
                            .collect::<Vec<_>>();
                        json!(connections)
                    }
                    "agents.list" => {
                        require_empty_arguments(&request)?;
                        let snapshots = self
                            .agent_hooks
                            .snapshots()
                            .into_iter()
                            .filter(|agent| {
                                requested_id
                                    .map(|id| id == agent.context.agent_id)
                                    .unwrap_or_else(|| {
                                        caller.has_access(
                                            &ControlResourceKind::Agent,
                                            Some(&agent.context.agent_id),
                                            &ControlAccess::Read,
                                        )
                                    })
                            })
                            .collect::<Vec<_>>();
                        serde_json::to_value(snapshots)
                            .map_err(|error| internal_error(error.to_string()))?
                    }
                    "agents.get_status" => {
                        require_empty_arguments(&request)?;
                        let agent_id = required_resource_id(&request)?;
                        let snapshot = self
                            .agent_hooks
                            .snapshots()
                            .into_iter()
                            .find(|agent| agent.context.agent_id == agent_id)
                            .ok_or_else(|| ControlError {
                                code: ControlErrorCode::NotFound,
                                message: "Managed Agent 不存在".into(),
                                retryable: false,
                                details: None,
                            })?;
                        serde_json::to_value(snapshot)
                            .map_err(|error| internal_error(error.to_string()))?
                    }
                    "agents.send_task" => {
                        let agent_id = required_resource_id(&request)?;
                        let arguments: AgentTaskArguments = parse_arguments(&request)?;
                        if arguments.task.trim().is_empty() {
                            return Err(invalid_arguments("task 不能为空"));
                        }
                        let runtime_id = self
                            .agent_hooks
                            .snapshots()
                            .into_iter()
                            .find(|agent| agent.context.agent_id == agent_id)
                            .map(|agent| agent.context.runtime_id)
                            .ok_or_else(|| ControlError {
                                code: ControlErrorCode::NotFound,
                                message: "Managed Agent 不存在".into(),
                                retryable: false,
                                details: None,
                            })?;
                        let payload = format!("{}\r", arguments.task);
                        self.backend
                            .write(&runtime_id, &payload)
                            .await
                            .map_err(backend_error)?;
                        json!({ "acceptedBytes": arguments.task.len(), "runtimeId": runtime_id })
                    }
                    "agents.interrupt" => {
                        require_empty_arguments(&request)?;
                        let agent_id = required_resource_id(&request)?;
                        let runtime_id = self
                            .agent_hooks
                            .snapshots()
                            .into_iter()
                            .find(|agent| agent.context.agent_id == agent_id)
                            .map(|agent| agent.context.runtime_id)
                            .ok_or_else(|| ControlError {
                                code: ControlErrorCode::NotFound,
                                message: "Managed Agent 不存在".into(),
                                retryable: false,
                                details: None,
                            })?;
                        self.backend
                            .interrupt(&runtime_id)
                            .await
                            .map_err(backend_error)?;
                        json!({ "interrupted": true, "runtimeId": runtime_id })
                    }
                    "mux.sessions.list" => {
                        let sessions = self.database.list_mux_sessions().map_err(internal_error)?;
                        let filtered = sessions
                            .into_iter()
                            .filter(|session| {
                                requested_id.map(|id| id == session.id).unwrap_or_else(|| {
                                    caller.has_access(
                                        &ControlResourceKind::MuxSession,
                                        Some(&session.id),
                                        &ControlAccess::Read,
                                    )
                                })
                            })
                            .collect::<Vec<_>>();
                        serde_json::to_value(filtered)
                            .map_err(|error| internal_error(error.to_string()))?
                    }
                    "mux.session.update" => {
                        let _mux_guard = self.mux_mutations.lock().await;
                        let session_id = required_resource_id(&request)?;
                        let arguments: MuxSessionUpdateArguments = parse_arguments(&request)?;
                        let existing = self
                            .database
                            .list_mux_sessions()
                            .map_err(internal_error)?
                            .into_iter()
                            .find(|session| session.id == session_id)
                            .ok_or_else(|| ControlError {
                                code: ControlErrorCode::NotFound,
                                message: "Mux Session 不存在".into(),
                                retryable: false,
                                details: None,
                            })?;
                        let saved = self
                            .database
                            .save_mux_session(MuxSessionInput {
                                id: Some(existing.id),
                                name: arguments.name.unwrap_or(existing.name),
                                root_path: arguments.root_path.unwrap_or(existing.root_path),
                                layout: existing.layout,
                            })
                            .map_err(internal_error)?;
                        self.side_effects
                            .notify_state_changed("muxSessionSaved", json!(saved))
                            .await
                            .map_err(internal_error)?;
                        serde_json::to_value(saved)
                            .map_err(|error| internal_error(error.to_string()))?
                    }
                    "mux.layout.set" => {
                        let _mux_guard = self.mux_mutations.lock().await;
                        let session_id = required_resource_id(&request)?;
                        let arguments: MuxLayoutSetArguments = parse_arguments(&request)?;
                        let existing = self
                            .database
                            .list_mux_sessions()
                            .map_err(internal_error)?
                            .into_iter()
                            .find(|session| session.id == session_id)
                            .ok_or_else(|| ControlError {
                                code: ControlErrorCode::NotFound,
                                message: "Mux Session 不存在".into(),
                                retryable: false,
                                details: None,
                            })?;
                        let pane_ids = self
                            .database
                            .list_mux_panes(Some(session_id))
                            .map_err(internal_error)?
                            .into_iter()
                            .map(|pane| pane.id)
                            .collect::<HashSet<_>>();
                        validate_mux_layout(arguments.layout.as_ref(), &pane_ids)?;
                        let saved = self
                            .database
                            .save_mux_session(MuxSessionInput {
                                id: Some(existing.id),
                                name: existing.name,
                                root_path: existing.root_path,
                                layout: arguments.layout,
                            })
                            .map_err(internal_error)?;
                        self.side_effects
                            .notify_state_changed("muxSessionSaved", json!(saved))
                            .await
                            .map_err(internal_error)?;
                        serde_json::to_value(saved)
                            .map_err(|error| internal_error(error.to_string()))?
                    }
                    "mux.panes.list" => {
                        let mux_session_id = request
                            .arguments
                            .get("muxSessionId")
                            .and_then(|value| value.as_str());
                        let panes = self
                            .database
                            .list_mux_panes(mux_session_id)
                            .map_err(internal_error)?;
                        let filtered = panes
                            .into_iter()
                            .filter(|pane| {
                                requested_id.map(|id| id == pane.id).unwrap_or_else(|| {
                                    caller.has_access(
                                        &ControlResourceKind::Pane,
                                        Some(&pane.id),
                                        &ControlAccess::Read,
                                    )
                                })
                            })
                            .collect::<Vec<_>>();
                        serde_json::to_value(filtered)
                            .map_err(|error| internal_error(error.to_string()))?
                    }
                    "mux.pane.create" => {
                        let _mux_guard = self.mux_mutations.lock().await;
                        let session_id = required_resource_id(&request)?;
                        let arguments: MuxPaneCreateArguments = parse_arguments(&request)?;
                        let session = self
                            .database
                            .list_mux_sessions()
                            .map_err(internal_error)?
                            .into_iter()
                            .find(|session| session.id == session_id)
                            .ok_or_else(|| ControlError {
                                code: ControlErrorCode::NotFound,
                                message: "Mux Session 不存在".into(),
                                retryable: false,
                                details: None,
                            })?;
                        let target = self
                            .backend
                            .targets()
                            .map_err(internal_error)?
                            .into_iter()
                            .find(|target| target.id == arguments.target_id)
                            .ok_or_else(|| ControlError {
                                code: ControlErrorCode::NotFound,
                                message: "Terminal Target 不存在".into(),
                                retryable: false,
                                details: None,
                            })?;
                        if !caller.has_access(
                            &ControlResourceKind::TerminalTarget,
                            Some(&target.id),
                            &ControlAccess::Read,
                        ) {
                            return Err(ControlError {
                                code: ControlErrorCode::Unauthorized,
                                message: "调用者没有该 Terminal Target 的访问权限".into(),
                                retryable: false,
                                details: None,
                            });
                        }
                        let existing_panes = self
                            .database
                            .list_mux_panes(Some(session_id))
                            .map_err(internal_error)?;
                        if let Some(anchor) = arguments.anchor_pane_id.as_deref()
                            && !existing_panes.iter().any(|pane| pane.id == anchor)
                        {
                            return Err(invalid_arguments("anchorPaneId 必须属于目标 Mux Session"));
                        }
                        let ratio = arguments.split_ratio.unwrap_or(0.5);
                        if !ratio.is_finite() || !(0.1..=0.9).contains(&ratio) {
                            return Err(invalid_arguments("splitRatio 必须在 0.1 到 0.9 之间"));
                        }
                        let target_id = target.id;
                        let title = arguments
                            .title
                            .filter(|title| !title.trim().is_empty())
                            .unwrap_or(target.label);
                        let bookmark_id = target_id
                            .strip_prefix("ssh-bookmark:")
                            .unwrap_or_default()
                            .to_string();
                        let pane = self
                            .database
                            .save_mux_pane(MuxPaneInput {
                                id: None,
                                mux_session_id: session_id.into(),
                                kind: MuxPaneKind::Terminal,
                                title,
                                target_id,
                                bookmark_id,
                                cwd: arguments.cwd.unwrap_or_else(|| session.root_path.clone()),
                                command: arguments.command,
                                launch_profile_id: arguments.launch_profile_id,
                            })
                            .map_err(internal_error)?;
                        let pane_ids = existing_panes
                            .iter()
                            .map(|pane| pane.id.clone())
                            .collect::<Vec<_>>();
                        let normalized = normalize_mux_layout(session.layout.clone(), &pane_ids);
                        let layout = insert_mux_pane(
                            normalized,
                            arguments.anchor_pane_id.as_deref(),
                            &pane.id,
                            arguments
                                .split_direction
                                .unwrap_or(MuxSplitDirection::Horizontal),
                            ratio,
                        );
                        let saved_session = match self.database.save_mux_session(MuxSessionInput {
                            id: Some(session.id),
                            name: session.name,
                            root_path: session.root_path,
                            layout: Some(layout),
                        }) {
                            Ok(saved) => saved,
                            Err(error) => {
                                let _ = self.database.delete_mux_pane(&pane.id);
                                return Err(internal_error(error));
                            }
                        };
                        self.side_effects
                            .notify_state_changed(
                                "muxPaneCreated",
                                json!({
                                    "pane": pane.clone(),
                                    "session": saved_session.clone(),
                                    "start": arguments.start
                                }),
                            )
                            .await
                            .map_err(internal_error)?;
                        json!({
                            "pane": pane,
                            "session": saved_session,
                            "startRequested": arguments.start
                        })
                    }
                    "mux.pane.update" => {
                        let _mux_guard = self.mux_mutations.lock().await;
                        let pane_id = required_resource_id(&request)?;
                        let arguments: MuxPaneUpdateArguments = parse_arguments(&request)?;
                        let existing = self
                            .database
                            .list_mux_panes(None)
                            .map_err(internal_error)?
                            .into_iter()
                            .find(|pane| pane.id == pane_id)
                            .ok_or_else(|| ControlError {
                                code: ControlErrorCode::NotFound,
                                message: "Mux Pane 不存在".into(),
                                retryable: false,
                                details: None,
                            })?;
                        let saved = self
                            .database
                            .save_mux_pane(MuxPaneInput {
                                id: Some(existing.id),
                                mux_session_id: existing.mux_session_id,
                                kind: existing.kind,
                                title: arguments.title.unwrap_or(existing.title),
                                target_id: existing.target_id,
                                bookmark_id: existing.bookmark_id,
                                cwd: arguments.cwd.unwrap_or(existing.cwd),
                                command: arguments.command.unwrap_or(existing.command),
                                launch_profile_id: arguments
                                    .launch_profile_id
                                    .unwrap_or(existing.launch_profile_id),
                            })
                            .map_err(internal_error)?;
                        self.side_effects
                            .notify_state_changed("muxPaneSaved", json!(saved))
                            .await
                            .map_err(internal_error)?;
                        serde_json::to_value(saved)
                            .map_err(|error| internal_error(error.to_string()))?
                    }
                    "terminal.targets.list" => {
                        let targets = self.backend.targets().map_err(internal_error)?;
                        let filtered = targets
                            .into_iter()
                            .filter(|target| {
                                requested_id.map(|id| id == target.id).unwrap_or_else(|| {
                                    caller.has_access(
                                        &ControlResourceKind::TerminalTarget,
                                        Some(&target.id),
                                        &ControlAccess::Read,
                                    )
                                })
                            })
                            .collect::<Vec<_>>();
                        serde_json::to_value(filtered)
                            .map_err(|error| internal_error(error.to_string()))?
                    }
                    "terminal.runtimes.list" => {
                        let runtimes = self.backend.list().map_err(internal_error)?;
                        let filtered = runtimes
                            .into_iter()
                            .filter(|runtime| {
                                requested_id
                                    .map(|id| id == runtime.runtime_id)
                                    .unwrap_or_else(|| {
                                        caller.has_access(
                                            &ControlResourceKind::TerminalRuntime,
                                            Some(&runtime.runtime_id),
                                            &ControlAccess::Read,
                                        )
                                    })
                            })
                            .collect::<Vec<_>>();
                        serde_json::to_value(filtered)
                            .map_err(|error| internal_error(error.to_string()))?
                    }
                    "terminal.runtime.output.read" => {
                        let runtime_id = required_resource_id(&request)?;
                        let from_cursor = optional_u64(&request, "fromCursor", 0)?;
                        let max_bytes = optional_u64(&request, "maxBytes", 64 * 1024)?
                            .clamp(4, 1024 * 1024) as usize;
                        serde_json::to_value(
                            self.backend
                                .read_output(runtime_id, from_cursor, max_bytes)
                                .map_err(backend_error)?,
                        )
                        .map_err(|error| internal_error(error.to_string()))?
                    }
                    "terminal.runtime.write" => {
                        let runtime_id = required_resource_id(&request)?;
                        let data = required_string(&request, "data", true)?;
                        self.backend
                            .write(runtime_id, data)
                            .await
                            .map_err(backend_error)?;
                        json!({ "acceptedBytes": data.len() })
                    }
                    "terminal.runtime.resize" => {
                        let runtime_id = required_resource_id(&request)?;
                        let cols = required_dimension(&request, "cols")?;
                        let rows = required_dimension(&request, "rows")?;
                        self.backend
                            .resize(runtime_id, cols, rows)
                            .await
                            .map_err(backend_error)?;
                        json!({ "cols": cols, "rows": rows })
                    }
                    "terminal.runtime.flow.set" => {
                        let runtime_id = required_resource_id(&request)?;
                        let paused = request
                            .arguments
                            .get("paused")
                            .and_then(|value| value.as_bool())
                            .ok_or_else(|| invalid_arguments("paused 必须是布尔值"))?;
                        self.backend
                            .set_output_paused(runtime_id, paused)
                            .map_err(backend_error)?;
                        json!({ "paused": paused })
                    }
                    "terminal.runtime.interrupt" => {
                        let runtime_id = required_resource_id(&request)?;
                        self.backend
                            .interrupt(runtime_id)
                            .await
                            .map_err(backend_error)?;
                        json!({ "interrupted": true })
                    }
                    "terminal.runtime.close" => {
                        let runtime_id = required_resource_id(&request)?;
                        self.backend
                            .close(runtime_id)
                            .await
                            .map_err(backend_error)?;
                        json!({ "closed": true })
                    }
                    "transfer.enqueue" => {
                        let runtime_id = required_resource_id(&request)?;
                        require_runtime_capability(&*self.backend, runtime_id, "remoteFiles")?;
                        let arguments: TransferEnqueueArguments = parse_arguments(&request)?;
                        let tasks = self
                            .side_effects
                            .transfer_enqueue(TransferRequest {
                                session_id: runtime_id.into(),
                                direction: arguments.direction,
                                source_paths: arguments.source_paths,
                                destination_directory: arguments.destination_directory,
                            })
                            .await
                            .map_err(backend_error)?;
                        serde_json::to_value(tasks)
                            .map_err(|error| internal_error(error.to_string()))?
                    }
                    "transfers.list" => {
                        let runtime_id = required_resource_id(&request)?;
                        require_empty_arguments(&request)?;
                        serde_json::to_value(
                            self.side_effects
                                .transfer_list(runtime_id)
                                .await
                                .map_err(backend_error)?,
                        )
                        .map_err(|error| internal_error(error.to_string()))?
                    }
                    "transfer.cancel" => {
                        let transfer_id = required_resource_id(&request)?;
                        require_empty_arguments(&request)?;
                        let cancelled = self
                            .side_effects
                            .transfer_cancel(transfer_id)
                            .await
                            .map_err(backend_error)?;
                        json!({ "cancelled": cancelled })
                    }
                    "tunnel.start" => {
                        let runtime_id = required_resource_id(&request)?;
                        require_runtime_capability(&*self.backend, runtime_id, "portForwarding")?;
                        let arguments: TunnelStartArguments = parse_arguments(&request)?;
                        serde_json::to_value(
                            self.side_effects
                                .tunnel_start(runtime_id, &arguments.profile_id)
                                .await
                                .map_err(backend_error)?,
                        )
                        .map_err(|error| internal_error(error.to_string()))?
                    }
                    "tunnels.list" => {
                        let runtime_id = required_resource_id(&request)?;
                        require_empty_arguments(&request)?;
                        serde_json::to_value(
                            self.side_effects
                                .tunnel_list(runtime_id)
                                .await
                                .map_err(backend_error)?,
                        )
                        .map_err(|error| internal_error(error.to_string()))?
                    }
                    "tunnel.profiles.list" => {
                        let runtime_id = required_resource_id(&request)?;
                        require_runtime_capability(&*self.backend, runtime_id, "portForwarding")?;
                        let arguments: TunnelProfilesListArguments = parse_arguments(&request)?;
                        let profiles = self
                            .database
                            .get_setting::<Vec<PortForwardProfile>>("portForwardProfiles", vec![])
                            .into_iter()
                            .filter(|profile| {
                                arguments
                                    .bookmark_id
                                    .as_deref()
                                    .map(|id| id == profile.bookmark_id)
                                    .unwrap_or(true)
                            })
                            .collect::<Vec<_>>();
                        serde_json::to_value(profiles)
                            .map_err(|error| internal_error(error.to_string()))?
                    }
                    "tunnel.stop" => {
                        let tunnel_id = required_resource_id(&request)?;
                        require_empty_arguments(&request)?;
                        let stopped = self
                            .side_effects
                            .tunnel_stop(tunnel_id)
                            .await
                            .map_err(backend_error)?;
                        json!({ "stopped": stopped })
                    }
                    "browser.runtimes.list" => {
                        require_empty_arguments(&request)?;
                        let runtimes = self
                            .side_effects
                            .browser_runtimes_list()
                            .await
                            .map_err(backend_error)?;
                        let filtered = runtimes
                            .into_iter()
                            .filter(|runtime| {
                                requested_id.map(|id| id == runtime.id).unwrap_or_else(|| {
                                    caller.has_access(
                                        &ControlResourceKind::BrowserRuntime,
                                        Some(&runtime.id),
                                        &ControlAccess::Read,
                                    )
                                })
                            })
                            .collect::<Vec<_>>();
                        serde_json::to_value(filtered)
                            .map_err(|error| internal_error(error.to_string()))?
                    }
                    "browser.navigate" => {
                        let runtime_id = required_resource_id(&request)?;
                        let url = required_string(&request, "url", false)?;
                        self.side_effects
                            .browser_navigate(runtime_id, url)
                            .await
                            .map_err(backend_error)?;
                        json!({ "navigated": true, "url": url })
                    }
                    "browser.snapshot" => {
                        require_empty_arguments(&request)?;
                        let runtime_id = required_resource_id(&request)?;
                        self.side_effects
                            .browser_snapshot(runtime_id)
                            .await
                            .map_err(backend_error)?
                    }
                    "browser.screenshot" => {
                        require_empty_arguments(&request)?;
                        let runtime_id = required_resource_id(&request)?;
                        let data_url = self
                            .side_effects
                            .browser_screenshot(runtime_id)
                            .await
                            .map_err(backend_error)?;
                        json!({ "dataUrl": data_url })
                    }
                    "browser.click" => {
                        let runtime_id = required_resource_id(&request)?;
                        let arguments: BrowserSelectorArguments = parse_arguments(&request)?;
                        let clicked = self
                            .side_effects
                            .browser_click(runtime_id, &arguments.selector)
                            .await
                            .map_err(backend_error)?;
                        json!({ "clicked": clicked })
                    }
                    "browser.type" => {
                        let runtime_id = required_resource_id(&request)?;
                        let arguments: BrowserTypeArguments = parse_arguments(&request)?;
                        let typed = self
                            .side_effects
                            .browser_type(
                                runtime_id,
                                &arguments.selector,
                                &arguments.text,
                                arguments.clear,
                            )
                            .await
                            .map_err(backend_error)?;
                        json!({ "typed": typed, "acceptedBytes": arguments.text.len() })
                    }
                    "browser.press" => {
                        let runtime_id = required_resource_id(&request)?;
                        let arguments: BrowserPressArguments = parse_arguments(&request)?;
                        self.side_effects
                            .browser_press(runtime_id, &arguments.key)
                            .await
                            .map_err(backend_error)?;
                        json!({ "pressed": true })
                    }
                    "browser.scroll" => {
                        let runtime_id = required_resource_id(&request)?;
                        let arguments: BrowserScrollArguments = parse_arguments(&request)?;
                        self.side_effects
                            .browser_scroll(
                                runtime_id,
                                arguments.delta_x,
                                arguments.delta_y,
                                arguments.x,
                                arguments.y,
                            )
                            .await
                            .map_err(backend_error)?;
                        json!({ "scrolled": true })
                    }
                    "browser.evaluate" => {
                        let runtime_id = required_resource_id(&request)?;
                        let arguments: BrowserEvaluateArguments = parse_arguments(&request)?;
                        self.side_effects
                            .browser_evaluate(runtime_id, &arguments.expression)
                            .await
                            .map_err(backend_error)?
                    }
                    "browser.wait" => {
                        let runtime_id = required_resource_id(&request)?;
                        let arguments: BrowserWaitArguments = parse_arguments(&request)?;
                        let found = self
                            .side_effects
                            .browser_wait(runtime_id, &arguments.selector, arguments.timeout_ms)
                            .await
                            .map_err(backend_error)?;
                        json!({ "found": found })
                    }
                    "browser.resize" => {
                        let runtime_id = required_resource_id(&request)?;
                        let width = required_dimension(&request, "width")?;
                        let height = required_dimension(&request, "height")?;
                        self.side_effects
                            .browser_resize(runtime_id, width, height)
                            .await
                            .map_err(backend_error)?;
                        json!({ "width": width, "height": height })
                    }
                    "browser.close" => {
                        require_empty_arguments(&request)?;
                        let runtime_id = required_resource_id(&request)?;
                        self.side_effects
                            .browser_close(runtime_id)
                            .await
                            .map_err(backend_error)?;
                        json!({ "closed": true })
                    }
                    _ => unreachable!(),
                })
            }
            .await;
            let result = match execution {
                Ok(result) => result,
                Err(error) => {
                    if let Some(cache_key) = idempotency_cache_key.as_ref() {
                        self.idempotency_forget(cache_key);
                    }
                    return Err(error);
                }
            };
            let response = ControlResponse {
                request_id: request.request_id.clone(),
                result,
            };
            if let Some(cache_key) = idempotency_cache_key {
                self.idempotency_complete(cache_key, idempotency_signature, response.clone());
            }
            Ok(response)
        }
        .await;
        let (event_type, result, error_code) = match &outcome {
            Ok(_) => ("control.operation.completed", "success", None),
            Err(error) => (
                "control.operation.failed",
                "failure",
                Some(error.code.clone()),
            ),
        };
        self.events.record_control_event(
            event_type,
            audit_resource.clone(),
            json!({
                "callerId": caller.caller_id,
                "callerKind": caller.kind,
                "operation": audit_operation.clone(),
                "requestId": audit_request_id,
                "resource": audit_resource,
                "arguments": audit_arguments.clone(),
                "result": result,
                "errorCode": error_code.clone(),
            }),
        );
        let resource_kind = audit_resource
            .as_ref()
            .map(|resource| enum_string(&resource.kind))
            .unwrap_or_default();
        let resource_id = audit_resource
            .as_ref()
            .map(|resource| resource.id.clone())
            .unwrap_or_default();
        let persistent_audit = ControlAuditRecord {
            id: Uuid::new_v4().to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            caller_id: caller.caller_id.clone(),
            caller_kind: enum_string(&caller.kind),
            operation: audit_operation,
            resource_kind,
            resource_id,
            arguments: audit_arguments,
            result: result.into(),
            error_code: error_code.as_ref().map(enum_string).unwrap_or_default(),
        };
        if let Err(error) = self.database.append_control_audit(&persistent_audit) {
            eprintln!("failed to persist control audit: {error}");
        }
        outcome
    }

    fn read_events(
        &self,
        caller: &ControlCaller,
        from_sequence: u64,
        limit: usize,
    ) -> ControlResult<ControlEventReadResult> {
        if !caller.can_access_any(&ControlResourceKind::Application, &ControlAccess::Read)
            && !caller.can_access_any(&ControlResourceKind::TerminalRuntime, &ControlAccess::Read)
            && !caller.can_access_any(&ControlResourceKind::MuxSession, &ControlAccess::Read)
            && !caller.can_access_any(&ControlResourceKind::Pane, &ControlAccess::Read)
            && !caller.can_access_any(&ControlResourceKind::Transfer, &ControlAccess::Read)
            && !caller.can_access_any(&ControlResourceKind::Tunnel, &ControlAccess::Read)
            && !caller.can_access_any(&ControlResourceKind::Agent, &ControlAccess::Read)
            && !caller.can_access_any(&ControlResourceKind::BrowserRuntime, &ControlAccess::Read)
        {
            return Err(ControlError {
                code: ControlErrorCode::Unauthorized,
                message: "调用者没有事件读取权限".into(),
                retryable: false,
                details: None,
            });
        }
        let mut result = self.events.read(from_sequence, limit);
        if !caller.can_access_any(&ControlResourceKind::Application, &ControlAccess::Read) {
            result.events.retain(|event| {
                if event.event_type.starts_with("control.operation.") {
                    return event
                        .payload
                        .get("callerId")
                        .and_then(|value| value.as_str())
                        == Some(caller.caller_id.as_str());
                }
                event
                    .resource
                    .as_ref()
                    .map(|resource| {
                        caller.has_access(&resource.kind, Some(&resource.id), &ControlAccess::Read)
                    })
                    .unwrap_or(false)
            });
        }
        Ok(result)
    }
}

fn enum_string(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn internal_error(message: impl Into<String>) -> ControlError {
    ControlError {
        code: ControlErrorCode::Internal,
        message: message.into(),
        retryable: false,
        details: None,
    }
}

fn invalid_arguments(message: impl Into<String>) -> ControlError {
    ControlError {
        code: ControlErrorCode::InvalidArguments,
        message: message.into(),
        retryable: false,
        details: None,
    }
}

fn backend_error(message: String) -> ControlError {
    let code = if message.to_ascii_lowercase().contains("not found") || message.contains("不存在")
    {
        ControlErrorCode::NotFound
    } else {
        ControlErrorCode::Unavailable
    };
    ControlError {
        code,
        message,
        retryable: false,
        details: None,
    }
}

fn required_resource_id(request: &ControlRequest) -> ControlResult<&str> {
    request
        .resource
        .as_ref()
        .map(|resource| resource.id.trim())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| invalid_arguments("该操作必须指定非空资源 ID"))
}

fn normalize_mux_layout(layout: Option<MuxSplitNode>, pane_ids: &[String]) -> Option<MuxSplitNode> {
    fn prune(
        node: MuxSplitNode,
        valid: &HashSet<&str>,
        placed: &mut HashSet<String>,
    ) -> Option<MuxSplitNode> {
        match node {
            MuxSplitNode::Pane { pane_id } => (valid.contains(pane_id.as_str())
                && placed.insert(pane_id.clone()))
            .then_some(MuxSplitNode::Pane { pane_id }),
            MuxSplitNode::Split {
                direction,
                ratio,
                first,
                second,
            } => match (prune(*first, valid, placed), prune(*second, valid, placed)) {
                (Some(first), Some(second)) => Some(MuxSplitNode::Split {
                    direction,
                    ratio: ratio.clamp(0.1, 0.9),
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (Some(node), None) | (None, Some(node)) => Some(node),
                (None, None) => None,
            },
        }
    }

    let valid = pane_ids.iter().map(String::as_str).collect::<HashSet<_>>();
    let mut placed = HashSet::new();
    let mut normalized = layout.and_then(|layout| prune(layout, &valid, &mut placed));
    for pane_id in pane_ids {
        if !placed.insert(pane_id.clone()) {
            continue;
        }
        let leaf = MuxSplitNode::Pane {
            pane_id: pane_id.clone(),
        };
        normalized = Some(match normalized {
            Some(existing) => MuxSplitNode::Split {
                direction: MuxSplitDirection::Horizontal,
                ratio: 0.5,
                first: Box::new(existing),
                second: Box::new(leaf),
            },
            None => leaf,
        });
    }
    normalized
}

fn insert_mux_pane(
    layout: Option<MuxSplitNode>,
    anchor_pane_id: Option<&str>,
    pane_id: &str,
    direction: MuxSplitDirection,
    ratio: f64,
) -> MuxSplitNode {
    fn insert(
        node: MuxSplitNode,
        anchor: &str,
        leaf: &MuxSplitNode,
        direction: &MuxSplitDirection,
        ratio: f64,
    ) -> (MuxSplitNode, bool) {
        match node {
            MuxSplitNode::Pane { pane_id } if pane_id == anchor => (
                MuxSplitNode::Split {
                    direction: direction.clone(),
                    ratio,
                    first: Box::new(MuxSplitNode::Pane { pane_id }),
                    second: Box::new(leaf.clone()),
                },
                true,
            ),
            pane @ MuxSplitNode::Pane { .. } => (pane, false),
            MuxSplitNode::Split {
                direction: current_direction,
                ratio: current_ratio,
                first,
                second,
            } => {
                let (first, inserted) = insert(*first, anchor, leaf, direction, ratio);
                if inserted {
                    return (
                        MuxSplitNode::Split {
                            direction: current_direction,
                            ratio: current_ratio,
                            first: Box::new(first),
                            second,
                        },
                        true,
                    );
                }
                let (second, inserted) = insert(*second, anchor, leaf, direction, ratio);
                (
                    MuxSplitNode::Split {
                        direction: current_direction,
                        ratio: current_ratio,
                        first: Box::new(first),
                        second: Box::new(second),
                    },
                    inserted,
                )
            }
        }
    }

    let leaf = MuxSplitNode::Pane {
        pane_id: pane_id.into(),
    };
    let Some(layout) = layout else {
        return leaf;
    };
    if let Some(anchor) = anchor_pane_id {
        let (layout, inserted) = insert(layout, anchor, &leaf, &direction, ratio);
        if inserted {
            return layout;
        }
        return MuxSplitNode::Split {
            direction,
            ratio,
            first: Box::new(layout),
            second: Box::new(leaf),
        };
    }
    MuxSplitNode::Split {
        direction,
        ratio,
        first: Box::new(layout),
        second: Box::new(leaf),
    }
}

fn validate_mux_layout(
    layout: Option<&MuxSplitNode>,
    pane_ids: &HashSet<String>,
) -> ControlResult<()> {
    fn visit(
        node: &MuxSplitNode,
        depth: usize,
        valid: &HashSet<String>,
        found: &mut HashSet<String>,
    ) -> ControlResult<()> {
        if depth > 32 {
            return Err(invalid_arguments("布局嵌套不能超过 32 层"));
        }
        match node {
            MuxSplitNode::Pane { pane_id } => {
                if !valid.contains(pane_id) {
                    return Err(invalid_arguments(format!(
                        "布局包含不属于当前 Session 的 Pane：{pane_id}"
                    )));
                }
                if !found.insert(pane_id.clone()) {
                    return Err(invalid_arguments(format!("布局重复引用了 Pane：{pane_id}")));
                }
            }
            MuxSplitNode::Split {
                ratio,
                first,
                second,
                ..
            } => {
                if !ratio.is_finite() || !(0.1..=0.9).contains(ratio) {
                    return Err(invalid_arguments("布局 split ratio 必须在 0.1 到 0.9 之间"));
                }
                visit(first, depth + 1, valid, found)?;
                visit(second, depth + 1, valid, found)?;
            }
        }
        Ok(())
    }

    let mut found = HashSet::new();
    if let Some(layout) = layout {
        visit(layout, 0, pane_ids, &mut found)?;
    }
    if &found != pane_ids {
        return Err(invalid_arguments(
            "布局必须且只能包含当前 Session 的全部 Pane",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ThemeSetArguments {
    theme: UiTheme,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MuxSessionUpdateArguments {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    root_path: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MuxPaneCreateArguments {
    target_id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    command: String,
    #[serde(default)]
    launch_profile_id: String,
    #[serde(default)]
    anchor_pane_id: Option<String>,
    #[serde(default)]
    split_direction: Option<MuxSplitDirection>,
    #[serde(default)]
    split_ratio: Option<f64>,
    #[serde(default = "default_true")]
    start: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MuxLayoutSetArguments {
    layout: Option<MuxSplitNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MuxPaneUpdateArguments {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    launch_profile_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TunnelProfilesListArguments {
    #[serde(default)]
    bookmark_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TransferEnqueueArguments {
    direction: TransferDirection,
    source_paths: Vec<String>,
    destination_directory: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TunnelStartArguments {
    profile_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentTaskArguments {
    task: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrowserSelectorArguments {
    selector: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrowserTypeArguments {
    selector: String,
    text: String,
    #[serde(default)]
    clear: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrowserPressArguments {
    key: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrowserScrollArguments {
    #[serde(default)]
    delta_x: f64,
    delta_y: f64,
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrowserEvaluateArguments {
    expression: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DiagnosticsRunArguments {
    #[serde(default)]
    filter: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DiagnosticsRepairArguments {
    action: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BrowserWaitArguments {
    selector: String,
    #[serde(default = "default_browser_wait_timeout_ms")]
    timeout_ms: u64,
}

fn default_browser_wait_timeout_ms() -> u64 {
    5_000
}

fn parse_arguments<T: for<'de> Deserialize<'de>>(request: &ControlRequest) -> ControlResult<T> {
    serde_json::from_value(request.arguments.clone())
        .map_err(|error| invalid_arguments(format!("arguments 无效：{error}")))
}

fn require_empty_arguments(request: &ControlRequest) -> ControlResult<()> {
    if request
        .arguments
        .as_object()
        .is_some_and(|arguments| arguments.is_empty())
    {
        Ok(())
    } else {
        Err(invalid_arguments("该操作不接受 arguments"))
    }
}

fn require_runtime_capability(
    backend: &dyn TerminalBackend,
    runtime_id: &str,
    capability: &str,
) -> ControlResult<()> {
    let runtime = backend
        .list()
        .map_err(backend_error)?
        .into_iter()
        .find(|runtime| runtime.runtime_id == runtime_id)
        .ok_or_else(|| ControlError {
            code: ControlErrorCode::NotFound,
            message: "Terminal Runtime 不存在".into(),
            retryable: false,
            details: None,
        })?;
    let available = match capability {
        "remoteFiles" => runtime.capabilities.remote_files,
        "portForwarding" => runtime.capabilities.port_forwarding,
        _ => false,
    };
    if available {
        Ok(())
    } else {
        Err(ControlError {
            code: ControlErrorCode::Unavailable,
            message: format!("该 Terminal Runtime 不支持 {capability}"),
            retryable: false,
            details: None,
        })
    }
}

fn required_string<'a>(
    request: &'a ControlRequest,
    name: &str,
    allow_empty: bool,
) -> ControlResult<&'a str> {
    let value = request
        .arguments
        .get(name)
        .and_then(|value| value.as_str())
        .ok_or_else(|| invalid_arguments(format!("{name} 必须是字符串")))?;
    if !allow_empty && value.is_empty() {
        return Err(invalid_arguments(format!("{name} 不能为空")));
    }
    Ok(value)
}

fn optional_u64(request: &ControlRequest, name: &str, default: u64) -> ControlResult<u64> {
    request
        .arguments
        .get(name)
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| invalid_arguments(format!("{name} 必须是非负整数")))
        })
        .unwrap_or(Ok(default))
}

fn required_dimension(request: &ControlRequest, name: &str) -> ControlResult<u32> {
    let value = optional_u64(request, name, 0)?;
    if !(1..=10_000).contains(&value) {
        return Err(invalid_arguments(format!("{name} 必须在 1 到 10000 之间")));
    }
    Ok(value as u32)
}

fn audit_arguments(request: &ControlRequest) -> serde_json::Value {
    if matches!(
        request.operation.as_str(),
        "terminal.runtime.write" | "agents.send_task" | "browser.type" | "browser.evaluate"
    ) {
        let field = match request.operation.as_str() {
            "agents.send_task" => "task",
            "browser.type" => "text",
            "browser.evaluate" => "expression",
            _ => "data",
        };
        json!({
            "dataBytes": request.arguments
                .get(field)
                .and_then(|value| value.as_str())
                .map(str::len),
            "selector": request.arguments.get("selector"),
            "clear": request.arguments.get("clear")
        })
    } else {
        request.arguments.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::*;
    use crate::control_contract::{ControlGrant, ControlResourceKind};
    use crate::models::{MuxPaneInput, MuxPaneKind, MuxSessionInput};
    use crate::terminal_backend::TerminalBackendResult;
    use crate::terminal_runtime_contract::{
        TerminalCapabilities, TerminalRuntime, TerminalRuntimeCreateRequest, TerminalRuntimeEvent,
        TerminalRuntimeOutputReadResult, TerminalRuntimeStatus, TerminalRuntimeStatusEvent,
        TerminalTarget, TerminalTargetKind, TerminalTransport,
    };

    struct EmptyBackend {
        targets: Vec<TerminalTarget>,
        runtimes: Vec<TerminalRuntime>,
        writes: Mutex<Vec<(String, String)>>,
        interrupts: Mutex<Vec<String>>,
    }

    impl EmptyBackend {
        fn empty() -> Self {
            Self {
                targets: vec![],
                runtimes: vec![],
                writes: Mutex::new(vec![]),
                interrupts: Mutex::new(vec![]),
            }
        }

        fn ssh_runtime(runtime_id: &str) -> Self {
            Self {
                targets: vec![],
                runtimes: vec![TerminalRuntime {
                    runtime_id: runtime_id.into(),
                    target_id: "ssh-bookmark:test".into(),
                    title: "SSH".into(),
                    status: TerminalRuntimeStatus::Running,
                    capabilities: TerminalCapabilities {
                        terminal: true,
                        resize: true,
                        flow_control: true,
                        interrupt: true,
                        output_cursor: true,
                        remote_files: true,
                        port_forwarding: true,
                    },
                    context: None,
                    managed_agent: None,
                    error: None,
                }],
                writes: Mutex::new(vec![]),
                interrupts: Mutex::new(vec![]),
            }
        }

        fn local_target() -> Self {
            Self {
                targets: vec![TerminalTarget {
                    id: "local:shell".into(),
                    label: "Local Shell".into(),
                    transport: TerminalTransport::LocalPty,
                    kind: TerminalTargetKind::MacosShell,
                    capabilities: crate::terminal_backend::standard_terminal_capabilities(false),
                }],
                runtimes: vec![],
                writes: Mutex::new(vec![]),
                interrupts: Mutex::new(vec![]),
            }
        }
    }

    struct EmptySideEffects;

    #[async_trait]
    impl ControlSideEffects for EmptySideEffects {
        async fn settings_set_ui_theme(&self, theme: UiTheme) -> Result<UiTheme, String> {
            Ok(theme)
        }

        async fn settings_set_terminal(
            &self,
            settings: TerminalSettings,
        ) -> Result<TerminalSettings, String> {
            Ok(settings)
        }

        async fn transfer_list(&self, _runtime_id: &str) -> Result<Vec<TransferTask>, String> {
            Ok(vec![])
        }

        async fn tunnel_list(&self, _runtime_id: &str) -> Result<Vec<TunnelSummary>, String> {
            Ok(vec![])
        }

        async fn notify_state_changed(
            &self,
            _change_type: &str,
            _payload: serde_json::Value,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn transfer_enqueue(
            &self,
            _request: TransferRequest,
        ) -> Result<Vec<TransferTask>, String> {
            Ok(vec![])
        }

        async fn transfer_cancel(&self, _transfer_id: &str) -> Result<bool, String> {
            Ok(false)
        }

        async fn tunnel_start(
            &self,
            _runtime_id: &str,
            _profile_id: &str,
        ) -> Result<TunnelSummary, String> {
            Err("not implemented".into())
        }

        async fn tunnel_stop(&self, _tunnel_id: &str) -> Result<bool, String> {
            Ok(false)
        }

        async fn browser_navigate(&self, _runtime_id: &str, _url: &str) -> Result<(), String> {
            Ok(())
        }

        async fn browser_snapshot(&self, _runtime_id: &str) -> Result<serde_json::Value, String> {
            Ok(json!({ "nodes": [] }))
        }

        async fn browser_screenshot(&self, _runtime_id: &str) -> Result<String, String> {
            Ok("data:image/png;base64,dGVzdA==".into())
        }

        async fn browser_click(&self, _runtime_id: &str, _selector: &str) -> Result<bool, String> {
            Ok(true)
        }

        async fn browser_type(
            &self,
            _runtime_id: &str,
            _selector: &str,
            _text: &str,
            _clear: bool,
        ) -> Result<bool, String> {
            Ok(true)
        }

        async fn browser_press(&self, _runtime_id: &str, _key: &str) -> Result<(), String> {
            Ok(())
        }

        async fn browser_scroll(
            &self,
            _runtime_id: &str,
            _delta_x: f64,
            _delta_y: f64,
            _x: f64,
            _y: f64,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn browser_evaluate(
            &self,
            _runtime_id: &str,
            _expression: &str,
        ) -> Result<serde_json::Value, String> {
            Ok(json!({ "value": true }))
        }

        async fn browser_wait(
            &self,
            _runtime_id: &str,
            _selector: &str,
            _timeout_ms: u64,
        ) -> Result<bool, String> {
            Ok(true)
        }

        async fn browser_resize(
            &self,
            _runtime_id: &str,
            _width: u32,
            _height: u32,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn browser_close(&self, _runtime_id: &str) -> Result<(), String> {
            Ok(())
        }

        async fn browser_runtimes_list(&self) -> Result<Vec<BrowserRuntime>, String> {
            Ok(vec![])
        }
    }

    #[derive(Default)]
    struct RecordingSideEffects {
        themes: Mutex<Vec<UiTheme>>,
        enqueues: Mutex<Vec<TransferRequest>>,
        cancellations: Mutex<Vec<String>>,
        tunnel_starts: Mutex<Vec<(String, String)>>,
        tunnel_stops: Mutex<Vec<String>>,
        browser_navigations: Mutex<Vec<(String, String)>>,
        browser_clicks: Mutex<Vec<(String, String)>>,
        browser_types: Mutex<Vec<(String, String, String, bool)>>,
        browser_presses: Mutex<Vec<(String, String)>>,
        browser_scrolls: Mutex<Vec<(String, f64, f64, f64, f64)>>,
        browser_evaluations: Mutex<Vec<(String, String)>>,
        browser_waits: Mutex<Vec<(String, String, u64)>>,
        browser_resizes: Mutex<Vec<(String, u32, u32)>>,
        browser_closes: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ControlSideEffects for RecordingSideEffects {
        async fn settings_set_ui_theme(&self, theme: UiTheme) -> Result<UiTheme, String> {
            self.themes.lock().unwrap().push(theme.clone());
            Ok(theme)
        }

        async fn settings_set_terminal(
            &self,
            settings: TerminalSettings,
        ) -> Result<TerminalSettings, String> {
            Ok(settings)
        }

        async fn transfer_list(&self, _runtime_id: &str) -> Result<Vec<TransferTask>, String> {
            Ok(vec![])
        }

        async fn tunnel_list(&self, _runtime_id: &str) -> Result<Vec<TunnelSummary>, String> {
            Ok(vec![])
        }

        async fn notify_state_changed(
            &self,
            _change_type: &str,
            _payload: serde_json::Value,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn transfer_enqueue(
            &self,
            request: TransferRequest,
        ) -> Result<Vec<TransferTask>, String> {
            self.enqueues.lock().unwrap().push(request);
            Ok(vec![])
        }

        async fn transfer_cancel(&self, transfer_id: &str) -> Result<bool, String> {
            self.cancellations.lock().unwrap().push(transfer_id.into());
            Ok(true)
        }

        async fn tunnel_start(
            &self,
            runtime_id: &str,
            profile_id: &str,
        ) -> Result<TunnelSummary, String> {
            self.tunnel_starts
                .lock()
                .unwrap()
                .push((runtime_id.into(), profile_id.into()));
            Ok(TunnelSummary {
                id: "tunnel-created".into(),
                profile_id: profile_id.into(),
                session_id: runtime_id.into(),
                name: "Test".into(),
                forward_type: crate::models::PortForwardType::Local,
                bind_address: "127.0.0.1".into(),
                bind_port: 43210,
                target_host: "127.0.0.1".into(),
                target_port: 3000,
                status: crate::models::TunnelStatus::Running,
                error: None,
                removed: false,
            })
        }

        async fn tunnel_stop(&self, tunnel_id: &str) -> Result<bool, String> {
            self.tunnel_stops.lock().unwrap().push(tunnel_id.into());
            Ok(true)
        }

        async fn browser_navigate(&self, runtime_id: &str, url: &str) -> Result<(), String> {
            self.browser_navigations
                .lock()
                .unwrap()
                .push((runtime_id.into(), url.into()));
            Ok(())
        }

        async fn browser_snapshot(&self, _runtime_id: &str) -> Result<serde_json::Value, String> {
            Ok(json!({ "nodes": [{ "role": { "value": "button" } }] }))
        }

        async fn browser_screenshot(&self, _runtime_id: &str) -> Result<String, String> {
            Ok("data:image/png;base64,dGVzdA==".into())
        }

        async fn browser_click(&self, runtime_id: &str, selector: &str) -> Result<bool, String> {
            self.browser_clicks
                .lock()
                .unwrap()
                .push((runtime_id.into(), selector.into()));
            Ok(true)
        }

        async fn browser_type(
            &self,
            runtime_id: &str,
            selector: &str,
            text: &str,
            clear: bool,
        ) -> Result<bool, String> {
            self.browser_types.lock().unwrap().push((
                runtime_id.into(),
                selector.into(),
                text.into(),
                clear,
            ));
            Ok(true)
        }

        async fn browser_press(&self, runtime_id: &str, key: &str) -> Result<(), String> {
            self.browser_presses
                .lock()
                .unwrap()
                .push((runtime_id.into(), key.into()));
            Ok(())
        }

        async fn browser_scroll(
            &self,
            runtime_id: &str,
            delta_x: f64,
            delta_y: f64,
            x: f64,
            y: f64,
        ) -> Result<(), String> {
            self.browser_scrolls
                .lock()
                .unwrap()
                .push((runtime_id.into(), delta_x, delta_y, x, y));
            Ok(())
        }

        async fn browser_evaluate(
            &self,
            runtime_id: &str,
            expression: &str,
        ) -> Result<serde_json::Value, String> {
            self.browser_evaluations
                .lock()
                .unwrap()
                .push((runtime_id.into(), expression.into()));
            Ok(json!({ "value": 42 }))
        }

        async fn browser_wait(
            &self,
            runtime_id: &str,
            selector: &str,
            timeout_ms: u64,
        ) -> Result<bool, String> {
            self.browser_waits.lock().unwrap().push((
                runtime_id.into(),
                selector.into(),
                timeout_ms,
            ));
            Ok(true)
        }

        async fn browser_resize(
            &self,
            runtime_id: &str,
            width: u32,
            height: u32,
        ) -> Result<(), String> {
            self.browser_resizes
                .lock()
                .unwrap()
                .push((runtime_id.into(), width, height));
            Ok(())
        }

        async fn browser_close(&self, runtime_id: &str) -> Result<(), String> {
            self.browser_closes.lock().unwrap().push(runtime_id.into());
            Ok(())
        }

        async fn browser_runtimes_list(&self) -> Result<Vec<BrowserRuntime>, String> {
            Ok(vec![])
        }
    }

    fn service() -> (Arc<InProcessControlService>, std::path::PathBuf) {
        service_with(Arc::new(EmptyBackend::empty()), Arc::new(EmptySideEffects))
    }

    fn service_with(
        backend: Arc<dyn TerminalBackend>,
        side_effects: Arc<dyn ControlSideEffects>,
    ) -> (Arc<InProcessControlService>, std::path::PathBuf) {
        service_with_agent_hooks(backend, side_effects, AgentHookService::new())
    }

    fn service_with_agent_hooks(
        backend: Arc<dyn TerminalBackend>,
        side_effects: Arc<dyn ControlSideEffects>,
        agent_hooks: Arc<AgentHookService>,
    ) -> (Arc<InProcessControlService>, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "{}-control-{}.db",
            crate::product::PRODUCT_KEY,
            uuid::Uuid::new_v4()
        ));
        let database = Arc::new(
            Database::open(
                &path,
                &format!("{}.control-test", crate::product::CREDENTIAL_SERVICE),
            )
            .expect("open control test database"),
        );
        (
            InProcessControlService::new(database, backend, side_effects, agent_hooks),
            path,
        )
    }

    #[async_trait]
    impl TerminalBackend for EmptyBackend {
        fn set_event_sink(&self, _sink: crate::terminal_backend::TerminalRuntimeEventSink) {}
        fn targets(&self) -> TerminalBackendResult<Vec<TerminalTarget>> {
            Ok(self.targets.clone())
        }
        fn list(&self) -> TerminalBackendResult<Vec<TerminalRuntime>> {
            Ok(self.runtimes.clone())
        }
        async fn create(
            &self,
            _request: TerminalRuntimeCreateRequest,
        ) -> TerminalBackendResult<TerminalRuntime> {
            Err("not implemented".into())
        }
        async fn write(&self, runtime_id: &str, data: &str) -> TerminalBackendResult<()> {
            self.writes
                .lock()
                .unwrap()
                .push((runtime_id.into(), data.into()));
            Ok(())
        }
        async fn resize(
            &self,
            _runtime_id: &str,
            _cols: u32,
            _rows: u32,
        ) -> TerminalBackendResult<()> {
            Ok(())
        }
        fn set_output_paused(&self, _runtime_id: &str, _paused: bool) -> TerminalBackendResult<()> {
            Ok(())
        }
        async fn interrupt(&self, runtime_id: &str) -> TerminalBackendResult<()> {
            self.interrupts.lock().unwrap().push(runtime_id.into());
            Ok(())
        }
        async fn close(&self, _runtime_id: &str) -> TerminalBackendResult<()> {
            Ok(())
        }
        fn read_output(
            &self,
            _runtime_id: &str,
            _from_cursor: u64,
            _max_bytes: usize,
        ) -> TerminalBackendResult<TerminalRuntimeOutputReadResult> {
            Err("not implemented".into())
        }
    }

    fn caller(kind: ControlResourceKind, id: Option<&str>, access: ControlAccess) -> ControlCaller {
        ControlCaller {
            caller_id: "test".into(),
            kind: ControlCallerKind::Agent,
            grants: vec![ControlGrant {
                resource_kind: kind,
                resource_id: id.map(str::to_string),
                access,
            }],
        }
    }

    #[test]
    fn event_buffer_reports_truncation() {
        let buffer = ControlEventBuffer::default();
        for _ in 0..EVENT_CAPACITY + 2 {
            buffer.record_runtime_event(TerminalRuntimeEvent::Status(TerminalRuntimeStatusEvent {
                runtime: TerminalRuntime {
                    runtime_id: "r".into(),
                    target_id: "t".into(),
                    title: "t".into(),
                    status: TerminalRuntimeStatus::Running,
                    capabilities: TerminalCapabilities {
                        terminal: true,
                        resize: true,
                        flow_control: true,
                        interrupt: true,
                        output_cursor: true,
                        remote_files: false,
                        port_forwarding: false,
                    },
                    context: None,
                    managed_agent: None,
                    error: None,
                },
            }));
        }
        assert!(buffer.read(0, 10).truncated);
    }

    #[test]
    fn scoped_grant_does_not_authorize_other_resource() {
        let agent = caller(
            ControlResourceKind::TerminalRuntime,
            Some("r1"),
            ControlAccess::Read,
        );
        assert!(agent.has_access(
            &ControlResourceKind::TerminalRuntime,
            Some("r1"),
            &ControlAccess::Read
        ));
        assert!(!agent.has_access(
            &ControlResourceKind::TerminalRuntime,
            Some("r2"),
            &ControlAccess::Read
        ));
    }

    #[tokio::test]
    async fn agent_browser_tools_stay_discoverable_while_runtimes_remain_grant_filtered() {
        let (service, path) = service();
        let agent = caller(
            ControlResourceKind::TerminalRuntime,
            Some("runtime-1"),
            ControlAccess::Control,
        );
        let catalog = service.catalog(&agent);
        assert!(
            catalog
                .operations
                .iter()
                .any(|operation| operation.name == "browser.runtimes.list")
        );
        assert!(
            catalog
                .operations
                .iter()
                .any(|operation| operation.name == "browser.click")
        );

        let response = service
            .invoke(
                &agent,
                ControlRequest {
                    contract_version: CONTROL_CONTRACT_VERSION,
                    request_id: "browser-list-without-grant".into(),
                    operation: "browser.runtimes.list".into(),
                    resource: None,
                    arguments: json!({}),
                    idempotency_key: None,
                    approval_id: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(response.result, json!([]));

        let unauthorized = service
            .invoke(
                &agent,
                ControlRequest {
                    contract_version: CONTROL_CONTRACT_VERSION,
                    request_id: "browser-list-forged-runtime".into(),
                    operation: "browser.runtimes.list".into(),
                    resource: Some(crate::control_contract::ControlResourceRef {
                        kind: ControlResourceKind::BrowserRuntime,
                        id: "ungranted-runtime".into(),
                    }),
                    arguments: json!({}),
                    idempotency_key: None,
                    approval_id: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(unauthorized.code, ControlErrorCode::Unauthorized);
        drop(service);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn version_mismatch_is_structured() {
        let error = InProcessControlService::validate_request(&ControlRequest {
            contract_version: CONTROL_CONTRACT_VERSION + 1,
            request_id: "request-1".into(),
            operation: "terminal.targets.list".into(),
            resource: None,
            arguments: serde_json::json!({}),
            idempotency_key: None,
            approval_id: None,
        })
        .unwrap_err();
        assert_eq!(error.code, ControlErrorCode::UnsupportedVersion);
        assert!(!error.retryable);
        assert_eq!(
            serde_json::to_value(error).unwrap()["code"],
            "unsupportedVersion"
        );
    }

    #[test]
    fn duplicate_idempotency_key_is_reserved_for_supported_operations() {
        let descriptor = InProcessControlService::descriptor("terminal.targets.list").unwrap();
        assert!(descriptor.supports_idempotency);
    }

    #[tokio::test]
    async fn duplicate_idempotency_key_reuses_first_response() {
        let (service, path) = service();
        let agent = caller(
            ControlResourceKind::TerminalTarget,
            None,
            ControlAccess::Read,
        );
        let request = |request_id: &str| ControlRequest {
            contract_version: CONTROL_CONTRACT_VERSION,
            request_id: request_id.into(),
            operation: "terminal.targets.list".into(),
            resource: None,
            arguments: serde_json::json!({}),
            idempotency_key: Some("same".into()),
            approval_id: None,
        };
        let first = service.invoke(&agent, request("request-1")).await.unwrap();
        let second = service.invoke(&agent, request("request-2")).await.unwrap();
        assert_eq!(first, second);
        drop(service);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn invoke_rejects_unauthorized_and_mismatched_resource() {
        let (service, path) = service();
        let agent = caller(
            ControlResourceKind::TerminalRuntime,
            Some("r1"),
            ControlAccess::Read,
        );
        let unauthorized = service
            .invoke(
                &agent,
                ControlRequest {
                    contract_version: CONTROL_CONTRACT_VERSION,
                    request_id: "request-1".into(),
                    operation: "terminal.targets.list".into(),
                    resource: None,
                    arguments: serde_json::json!({}),
                    idempotency_key: None,
                    approval_id: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(unauthorized.code, ControlErrorCode::Unauthorized);
        let mismatched = service
            .invoke(
                &agent,
                ControlRequest {
                    contract_version: CONTROL_CONTRACT_VERSION,
                    request_id: "request-2".into(),
                    operation: "terminal.targets.list".into(),
                    resource: Some(crate::control_contract::ControlResourceRef {
                        kind: ControlResourceKind::TerminalRuntime,
                        id: "r1".into(),
                    }),
                    arguments: serde_json::json!({}),
                    idempotency_key: None,
                    approval_id: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(mismatched.code, ControlErrorCode::InvalidArguments);
        drop(service);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn discovers_only_authorized_mux_sessions_and_panes() {
        let (service, path) = service();
        let first = service
            .database
            .save_mux_session(MuxSessionInput {
                id: None,
                name: "First".into(),
                root_path: String::new(),
                layout: None,
            })
            .unwrap();
        let second = service
            .database
            .save_mux_session(MuxSessionInput {
                id: None,
                name: "Second".into(),
                root_path: String::new(),
                layout: None,
            })
            .unwrap();
        let pane = service
            .database
            .save_mux_pane(MuxPaneInput {
                id: None,
                mux_session_id: first.id.clone(),
                kind: MuxPaneKind::Terminal,
                title: "Shell".into(),
                target_id: "local:powershell".into(),
                bookmark_id: String::new(),
                cwd: String::new(),
                command: String::new(),
                launch_profile_id: String::new(),
            })
            .unwrap();
        let request = |operation: &str, arguments| ControlRequest {
            contract_version: CONTROL_CONTRACT_VERSION,
            request_id: operation.into(),
            operation: operation.into(),
            resource: None,
            arguments,
            idempotency_key: None,
            approval_id: None,
        };
        let sessions = service
            .invoke(
                &caller(
                    ControlResourceKind::MuxSession,
                    Some(&first.id),
                    ControlAccess::Read,
                ),
                request("mux.sessions.list", json!({})),
            )
            .await
            .unwrap();
        assert_eq!(sessions.result.as_array().unwrap().len(), 1);
        assert_eq!(sessions.result[0]["id"], first.id);
        let panes = service
            .invoke(
                &caller(
                    ControlResourceKind::Pane,
                    Some(&pane.id),
                    ControlAccess::Read,
                ),
                request("mux.panes.list", json!({ "muxSessionId": first.id })),
            )
            .await
            .unwrap();
        assert_eq!(panes.result.as_array().unwrap().len(), 1);
        assert_eq!(panes.result[0]["id"], pane.id);
        assert_ne!(sessions.result[0]["id"], second.id);
        drop(service);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn agent_discovery_and_actions_respect_caller_access() {
        let hooks = AgentHookService::new();
        let target = crate::terminal_runtime_contract::TerminalManagedAgentContext {
            mux_session_id: "session-1".into(),
            pane_id: "pane-target".into(),
            runtime_id: "runtime-target".into(),
            agent_id: "agent-target".into(),
            launch_profile_id: "codex.default".into(),
        };
        hooks.issue_token(target.clone()).unwrap();
        let backend = Arc::new(EmptyBackend::ssh_runtime("runtime-target"));
        let (service, path) =
            service_with_agent_hooks(backend.clone(), Arc::new(EmptySideEffects), hooks);
        let request = |operation: &str, arguments: serde_json::Value| ControlRequest {
            contract_version: CONTROL_CONTRACT_VERSION,
            request_id: operation.into(),
            operation: operation.into(),
            resource: Some(crate::control_contract::ControlResourceRef {
                kind: ControlResourceKind::Agent,
                id: target.agent_id.clone(),
            }),
            arguments,
            idempotency_key: None,
            approval_id: None,
        };
        let read_caller = caller(
            ControlResourceKind::Agent,
            Some("agent-target"),
            ControlAccess::Read,
        );
        let listed = service
            .invoke(&read_caller, request("agents.list", json!({})))
            .await
            .unwrap();
        assert_eq!(listed.result.as_array().unwrap().len(), 1);
        assert_eq!(listed.result[0]["context"]["agentId"], "agent-target");
        assert_eq!(
            service
                .invoke(
                    &read_caller,
                    request("agents.send_task", json!({ "task": "Review the change" }))
                )
                .await
                .unwrap_err()
                .code,
            ControlErrorCode::Unauthorized
        );

        let trusted_write = ControlCaller {
            caller_id: "internal-test".into(),
            kind: ControlCallerKind::Internal,
            grants: vec![ControlGrant {
                resource_kind: ControlResourceKind::Agent,
                resource_id: Some("agent-target".into()),
                access: ControlAccess::Control,
            }],
        };
        service
            .invoke(
                &trusted_write,
                request("agents.send_task", json!({ "task": "Review the change" })),
            )
            .await
            .unwrap();
        service
            .invoke(&trusted_write, request("agents.interrupt", json!({})))
            .await
            .unwrap();
        assert_eq!(
            backend.writes.lock().unwrap().as_slice(),
            &[("runtime-target".into(), "Review the change\r".into())]
        );
        assert_eq!(
            backend.interrupts.lock().unwrap().as_slice(),
            &["runtime-target".to_string()]
        );
        let audits = service.read_events(&trusted_write, 0, 100).unwrap();
        let task_audit = audits
            .events
            .iter()
            .find(|event| event.payload["operation"] == "agents.send_task")
            .unwrap();
        assert!(task_audit.payload["arguments"].get("task").is_none());
        assert_eq!(task_audit.payload["arguments"]["dataBytes"], 17);
        drop(service);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn records_timestamped_control_audit_events() {
        let (service, path) = service();
        let agent = caller(ControlResourceKind::MuxSession, None, ControlAccess::Read);
        service
            .invoke(
                &agent,
                ControlRequest {
                    contract_version: CONTROL_CONTRACT_VERSION,
                    request_id: "audit-request".into(),
                    operation: "mux.sessions.list".into(),
                    resource: None,
                    arguments: json!({}),
                    idempotency_key: None,
                    approval_id: None,
                },
            )
            .await
            .unwrap();
        let events = service.read_events(&agent, 0, 10).unwrap();
        let audit = events
            .events
            .iter()
            .find(|event| event.event_type == "control.operation.completed")
            .expect("audit event");
        assert!(!audit.timestamp.is_empty());
        assert_eq!(audit.payload["callerId"], "test");
        assert_eq!(audit.payload["operation"], "mux.sessions.list");
        assert_eq!(audit.payload["requestId"], "audit-request");
        let persisted = service.database.list_control_audit(10).unwrap();
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].caller_id, "test");
        assert_eq!(persisted[0].caller_kind, "agent");
        assert_eq!(persisted[0].operation, "mux.sessions.list");
        assert_eq!(persisted[0].result, "success");
        assert_eq!(service.database.clear_control_audit().unwrap(), 1);
        assert!(service.database.list_control_audit(10).unwrap().is_empty());
        drop(service);
        let _ = std::fs::remove_file(path);
    }

    fn runtime_request(
        operation: &str,
        arguments: serde_json::Value,
        approval_id: Option<String>,
    ) -> ControlRequest {
        ControlRequest {
            contract_version: CONTROL_CONTRACT_VERSION,
            request_id: format!("request-{operation}"),
            operation: operation.into(),
            resource: Some(crate::control_contract::ControlResourceRef {
                kind: ControlResourceKind::TerminalRuntime,
                id: "runtime-1".into(),
            }),
            arguments,
            idempotency_key: Some(format!("key-{operation}")),
            approval_id,
        }
    }

    fn resource_request(
        operation: &str,
        kind: ControlResourceKind,
        id: &str,
        arguments: serde_json::Value,
        idempotency_key: &str,
        approval_id: Option<String>,
    ) -> ControlRequest {
        ControlRequest {
            contract_version: CONTROL_CONTRACT_VERSION,
            request_id: format!("request-{operation}"),
            operation: operation.into(),
            resource: Some(crate::control_contract::ControlResourceRef {
                kind,
                id: id.into(),
            }),
            arguments,
            idempotency_key: Some(idempotency_key.into()),
            approval_id,
        }
    }

    async fn approve_and_invoke(
        service: &InProcessControlService,
        caller: &ControlCaller,
        mut request: ControlRequest,
    ) -> ControlResponse {
        let required = service.invoke(caller, request.clone()).await.unwrap_err();
        assert_eq!(required.code, ControlErrorCode::ApprovalRequired);
        let approval_id = required.details.unwrap()["approval"]["approvalId"]
            .as_str()
            .unwrap()
            .to_string();
        service.resolve_approval(&approval_id, true).unwrap();
        request.approval_id = Some(approval_id);
        service.invoke(caller, request).await.unwrap()
    }

    #[tokio::test]
    async fn transfer_and_tunnel_side_effects_are_scoped_approved_and_idempotent() {
        let effects = Arc::new(RecordingSideEffects::default());
        let (service, path) = service_with(
            Arc::new(EmptyBackend::ssh_runtime("runtime-1")),
            effects.clone(),
        );
        let runtime_agent = caller(
            ControlResourceKind::TerminalRuntime,
            Some("runtime-1"),
            ControlAccess::Control,
        );
        let enqueue = resource_request(
            "transfer.enqueue",
            ControlResourceKind::TerminalRuntime,
            "runtime-1",
            json!({
                "direction": "upload",
                "sourcePaths": ["C:\\work\\main.rs"],
                "destinationDirectory": "/tmp"
            }),
            "enqueue-1",
            None,
        );
        let first = approve_and_invoke(&service, &runtime_agent, enqueue.clone()).await;
        assert_eq!(first.result, json!([]));
        let duplicate = service.invoke(&runtime_agent, enqueue).await.unwrap();
        assert_eq!(duplicate, first);
        assert_eq!(effects.enqueues.lock().unwrap().len(), 1);
        assert_eq!(effects.enqueues.lock().unwrap()[0].session_id, "runtime-1");

        let tunnel_start = resource_request(
            "tunnel.start",
            ControlResourceKind::TerminalRuntime,
            "runtime-1",
            json!({ "profileId": "profile-1" }),
            "tunnel-start-1",
            None,
        );
        let tunnel = approve_and_invoke(&service, &runtime_agent, tunnel_start).await;
        assert_eq!(tunnel.result["id"], "tunnel-created");
        assert_eq!(
            effects.tunnel_starts.lock().unwrap().as_slice(),
            &[("runtime-1".into(), "profile-1".into())]
        );

        let transfer_agent = caller(
            ControlResourceKind::Transfer,
            Some("transfer-1"),
            ControlAccess::Control,
        );
        let cancelled = approve_and_invoke(
            &service,
            &transfer_agent,
            resource_request(
                "transfer.cancel",
                ControlResourceKind::Transfer,
                "transfer-1",
                json!({}),
                "cancel-1",
                None,
            ),
        )
        .await;
        assert_eq!(cancelled.result["cancelled"], true);

        let tunnel_agent = caller(
            ControlResourceKind::Tunnel,
            Some("tunnel-created"),
            ControlAccess::Control,
        );
        let stopped = approve_and_invoke(
            &service,
            &tunnel_agent,
            resource_request(
                "tunnel.stop",
                ControlResourceKind::Tunnel,
                "tunnel-created",
                json!({}),
                "stop-1",
                None,
            ),
        )
        .await;
        assert_eq!(stopped.result["stopped"], true);
        assert_eq!(
            effects.cancellations.lock().unwrap().as_slice(),
            &["transfer-1"]
        );
        assert_eq!(
            effects.tunnel_stops.lock().unwrap().as_slice(),
            &["tunnel-created"]
        );
        drop(service);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn idempotency_key_cannot_be_reused_for_different_resource_or_arguments() {
        let effects = Arc::new(RecordingSideEffects::default());
        let (service, path) =
            service_with(Arc::new(EmptyBackend::ssh_runtime("runtime-1")), effects);
        let agent = caller(
            ControlResourceKind::TerminalRuntime,
            None,
            ControlAccess::Control,
        );
        let first = resource_request(
            "transfer.enqueue",
            ControlResourceKind::TerminalRuntime,
            "runtime-1",
            json!({
                "direction": "upload",
                "sourcePaths": ["one"],
                "destinationDirectory": "/tmp"
            }),
            "same-key",
            None,
        );
        approve_and_invoke(&service, &agent, first).await;
        let changed_arguments = resource_request(
            "transfer.enqueue",
            ControlResourceKind::TerminalRuntime,
            "runtime-1",
            json!({
                "direction": "upload",
                "sourcePaths": ["two"],
                "destinationDirectory": "/tmp"
            }),
            "same-key",
            None,
        );
        assert_eq!(
            service
                .invoke(&agent, changed_arguments)
                .await
                .unwrap_err()
                .code,
            ControlErrorCode::Conflict
        );
        let other_resource = resource_request(
            "transfer.enqueue",
            ControlResourceKind::TerminalRuntime,
            "runtime-2",
            json!({
                "direction": "upload",
                "sourcePaths": ["one"],
                "destinationDirectory": "/tmp"
            }),
            "same-key",
            None,
        );
        assert_eq!(
            service
                .invoke(&agent, other_resource)
                .await
                .unwrap_err()
                .code,
            ControlErrorCode::Conflict
        );
        drop(service);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn destructive_runtime_control_requires_one_time_user_approval() {
        let (service, path) = service();
        let agent = caller(
            ControlResourceKind::TerminalRuntime,
            Some("runtime-1"),
            ControlAccess::Control,
        );
        let request = runtime_request("terminal.runtime.close", json!({}), None);
        let error = service.invoke(&agent, request.clone()).await.unwrap_err();
        assert_eq!(error.code, ControlErrorCode::ApprovalRequired);
        let approval_id = error.details.unwrap()["approval"]["approvalId"]
            .as_str()
            .unwrap()
            .to_string();
        service.resolve_approval(&approval_id, true).unwrap();
        let mut approved = request;
        approved.approval_id = Some(approval_id.clone());
        let response = service.invoke(&agent, approved.clone()).await.unwrap();
        assert_eq!(response.result["closed"], true);
        approved.idempotency_key = None;
        assert_eq!(
            service.invoke(&agent, approved).await.unwrap_err().code,
            ControlErrorCode::ApprovalDenied
        );
        drop(service);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn safe_runtime_controls_execute_with_write_grant_and_audit_failures() {
        let (service, path) = service();
        let agent = caller(
            ControlResourceKind::TerminalRuntime,
            Some("runtime-1"),
            ControlAccess::Write,
        );
        let resize = service
            .invoke(
                &agent,
                runtime_request(
                    "terminal.runtime.resize",
                    json!({ "cols": 120, "rows": 40 }),
                    None,
                ),
            )
            .await
            .unwrap();
        assert_eq!(resize.result, json!({ "cols": 120, "rows": 40 }));
        let mut invalid_resize = runtime_request(
            "terminal.runtime.resize",
            json!({ "cols": 0, "rows": 40 }),
            None,
        );
        invalid_resize.idempotency_key = Some("key-invalid-resize".into());
        let error = service.invoke(&agent, invalid_resize).await.unwrap_err();
        assert_eq!(error.code, ControlErrorCode::InvalidArguments);
        let audit = service
            .read_events(&agent, 0, 100)
            .unwrap()
            .events
            .into_iter()
            .find(|event| {
                event.event_type == "control.operation.failed"
                    && event.payload["operation"] == "terminal.runtime.resize"
            })
            .expect("failure audit");
        assert_eq!(audit.payload["result"], "failure");
        assert_eq!(audit.payload["errorCode"], "invalidArguments");
        drop(service);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn raw_terminal_write_is_delivered_exactly_once_for_idempotent_retry() {
        let backend = Arc::new(EmptyBackend::ssh_runtime("runtime-1"));
        let (service, path) = service_with(backend.clone(), Arc::new(EmptySideEffects));
        let trusted = ControlCaller {
            caller_id: "internal-write-test".into(),
            kind: ControlCallerKind::Internal,
            grants: vec![ControlGrant {
                resource_kind: ControlResourceKind::TerminalRuntime,
                resource_id: Some("runtime-1".into()),
                access: ControlAccess::Write,
            }],
        };
        let request = runtime_request(
            "terminal.runtime.write",
            json!({ "data": "echo once\r" }),
            None,
        );
        let first = service.invoke(&trusted, request.clone()).await.unwrap();
        let retry = service.invoke(&trusted, request).await.unwrap();
        assert_eq!(first, retry);
        assert_eq!(
            backend.writes.lock().unwrap().as_slice(),
            &[("runtime-1".into(), "echo once\r".into())]
        );
        let audit = service.read_events(&trusted, 0, 100).unwrap();
        assert!(
            audit
                .events
                .iter()
                .filter(|event| {
                    event.payload["operation"] == "terminal.runtime.write"
                        && event.payload["result"] == "success"
                })
                .count()
                >= 2
        );
        assert!(
            audit
                .events
                .iter()
                .all(|event| { event.payload["arguments"].get("data").is_none() })
        );
        drop(service);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn browser_tools_are_scoped_approved_idempotent_and_redacted() {
        let effects = Arc::new(RecordingSideEffects::default());
        let (service, path) = service_with(Arc::new(EmptyBackend::empty()), effects.clone());
        let agent = caller(
            ControlResourceKind::BrowserRuntime,
            Some("browser-1"),
            ControlAccess::Control,
        );
        let request = |operation: &str, arguments: serde_json::Value| {
            resource_request(
                operation,
                ControlResourceKind::BrowserRuntime,
                "browser-1",
                arguments,
                &format!("key-{operation}"),
                None,
            )
        };

        let snapshot = service
            .invoke(&agent, request("browser.snapshot", json!({})))
            .await
            .unwrap();
        assert_eq!(snapshot.result["nodes"][0]["role"]["value"], "button");
        let screenshot = service
            .invoke(&agent, request("browser.screenshot", json!({})))
            .await
            .unwrap();
        assert!(
            screenshot.result["dataUrl"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
        let waited = service
            .invoke(
                &agent,
                request(
                    "browser.wait",
                    json!({ "selector": "#ready", "timeoutMs": 1200 }),
                ),
            )
            .await
            .unwrap();
        assert_eq!(waited.result["found"], true);

        for (operation, arguments) in [
            (
                "browser.navigate",
                json!({ "url": "http://localhost:3000" }),
            ),
            ("browser.click", json!({ "selector": "button.save" })),
            (
                "browser.type",
                json!({ "selector": "#prompt", "text": "private prompt", "clear": true }),
            ),
            ("browser.press", json!({ "key": "Ctrl+Enter" })),
            (
                "browser.scroll",
                json!({ "deltaX": 0.0, "deltaY": 640.0, "x": 100.0, "y": 200.0 }),
            ),
            (
                "browser.evaluate",
                json!({ "expression": "document.body.dataset.secret" }),
            ),
        ] {
            let pending = request(operation, arguments);
            let response = approve_and_invoke(&service, &agent, pending.clone()).await;
            assert!(!response.result.is_null());
            if operation == "browser.type" {
                let duplicate = service.invoke(&agent, pending).await.unwrap();
                assert_eq!(duplicate, response);
            }
        }

        assert_eq!(effects.browser_navigations.lock().unwrap().len(), 1);
        assert_eq!(effects.browser_clicks.lock().unwrap().len(), 1);
        assert_eq!(effects.browser_types.lock().unwrap().len(), 1);
        assert_eq!(effects.browser_presses.lock().unwrap().len(), 1);
        assert_eq!(effects.browser_scrolls.lock().unwrap().len(), 1);
        assert_eq!(effects.browser_evaluations.lock().unwrap().len(), 1);
        assert_eq!(
            effects.browser_waits.lock().unwrap().as_slice(),
            &[("browser-1".into(), "#ready".into(), 1200)]
        );

        let audit = service.database.list_control_audit(100).unwrap();
        let typed = audit
            .iter()
            .find(|record| record.operation == "browser.type" && record.result == "success")
            .unwrap();
        assert_eq!(typed.arguments["dataBytes"], 14);
        assert_eq!(typed.arguments["selector"], "#prompt");
        assert!(typed.arguments.get("text").is_none());
        let evaluated = audit
            .iter()
            .find(|record| record.operation == "browser.evaluate" && record.result == "success")
            .unwrap();
        assert_eq!(evaluated.arguments["dataBytes"], 28);
        assert!(evaluated.arguments.get("expression").is_none());

        drop(service);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn operation_catalog_declares_approval_policy() {
        let close = InProcessControlService::descriptor("terminal.runtime.close").unwrap();
        assert_eq!(close.access, ControlAccess::Control);
        assert_eq!(close.approval, ControlApprovalRequirement::User);
        let resize = InProcessControlService::descriptor("terminal.runtime.resize").unwrap();
        assert_eq!(resize.access, ControlAccess::Write);
        assert_eq!(resize.approval, ControlApprovalRequirement::None);
    }

    #[tokio::test]
    async fn settings_tools_are_authorized_without_application_super_grant() {
        let effects = Arc::new(RecordingSideEffects::default());
        let (service, path) = service_with(Arc::new(EmptyBackend::empty()), effects.clone());
        let settings_caller = caller(ControlResourceKind::Settings, None, ControlAccess::Write);
        assert!(
            !settings_caller
                .can_access_any(&ControlResourceKind::Application, &ControlAccess::Read)
        );
        let catalog = service.catalog(&settings_caller);
        assert!(
            catalog
                .operations
                .iter()
                .any(|operation| operation.name == "settings.appearance.get")
        );
        assert!(
            catalog
                .operations
                .iter()
                .any(|operation| operation.name == "settings.theme.set")
        );
        assert!(
            catalog
                .operations
                .iter()
                .any(|operation| operation.name == "settings.terminal.set")
        );

        let response = service
            .invoke(
                &settings_caller,
                ControlRequest {
                    contract_version: CONTROL_CONTRACT_VERSION,
                    request_id: "set-theme".into(),
                    operation: "settings.theme.set".into(),
                    resource: None,
                    arguments: json!({ "theme": "light" }),
                    idempotency_key: Some("theme-light".into()),
                    approval_id: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(response.result, json!({ "theme": "light" }));
        assert_eq!(
            serde_json::to_value(effects.themes.lock().unwrap().as_slice()).unwrap(),
            json!(["light"])
        );

        drop(service);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn creates_idempotent_panes_and_rejects_invalid_layouts() {
        let (service, path) = service_with(
            Arc::new(EmptyBackend::local_target()),
            Arc::new(RecordingSideEffects::default()),
        );
        let session = service
            .database
            .save_mux_session(MuxSessionInput {
                id: Some("session-layout".into()),
                name: "Layout".into(),
                root_path: "/tmp".into(),
                layout: None,
            })
            .unwrap();
        let agent = ControlCaller {
            caller_id: "pane-builder".into(),
            kind: ControlCallerKind::Agent,
            grants: vec![
                ControlGrant {
                    resource_kind: ControlResourceKind::MuxSession,
                    resource_id: Some(session.id.clone()),
                    access: ControlAccess::Write,
                },
                ControlGrant {
                    resource_kind: ControlResourceKind::TerminalTarget,
                    resource_id: None,
                    access: ControlAccess::Read,
                },
            ],
        };
        let create = |request_id: &str, key: &str, anchor: Option<&str>| ControlRequest {
            contract_version: CONTROL_CONTRACT_VERSION,
            request_id: request_id.into(),
            operation: "mux.pane.create".into(),
            resource: Some(crate::control_contract::ControlResourceRef {
                kind: ControlResourceKind::MuxSession,
                id: session.id.clone(),
            }),
            arguments: json!({
                "targetId": "local:shell",
                "title": request_id,
                "anchorPaneId": anchor,
                "splitDirection": "vertical",
                "splitRatio": 0.4,
                "start": true
            }),
            idempotency_key: Some(key.into()),
            approval_id: None,
        };
        let first = service
            .invoke(&agent, create("First", "create-first", None))
            .await
            .unwrap();
        let first_id = first.result["pane"]["id"].as_str().unwrap().to_string();
        let duplicate = service
            .invoke(&agent, create("First", "create-first", None))
            .await
            .unwrap();
        assert_eq!(duplicate.result["pane"]["id"], first_id);
        let second = service
            .invoke(&agent, create("Second", "create-second", Some(&first_id)))
            .await
            .unwrap();
        let second_id = second.result["pane"]["id"].as_str().unwrap().to_string();
        assert_eq!(
            service
                .database
                .list_mux_panes(Some(&session.id))
                .unwrap()
                .len(),
            2
        );

        let invalid = service
            .invoke(
                &agent,
                ControlRequest {
                    contract_version: CONTROL_CONTRACT_VERSION,
                    request_id: "duplicate-layout-leaf".into(),
                    operation: "mux.layout.set".into(),
                    resource: Some(crate::control_contract::ControlResourceRef {
                        kind: ControlResourceKind::MuxSession,
                        id: session.id.clone(),
                    }),
                    arguments: json!({
                        "layout": {
                            "type": "split",
                            "direction": "horizontal",
                            "ratio": 0.5,
                            "first": { "type": "pane", "paneId": first_id },
                            "second": { "type": "pane", "paneId": first_id }
                        }
                    }),
                    idempotency_key: None,
                    approval_id: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(invalid.code, ControlErrorCode::InvalidArguments);

        let current = service
            .database
            .list_mux_sessions()
            .unwrap()
            .into_iter()
            .find(|item| item.id == session.id)
            .unwrap();
        let leaves = match current.layout.unwrap() {
            MuxSplitNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                assert_eq!(direction, MuxSplitDirection::Vertical);
                assert_eq!(ratio, 0.4);
                vec![*first, *second]
            }
            _ => panic!("two Panes must produce a split layout"),
        };
        assert!(
            leaves
                .iter()
                .any(|node| matches!(node, MuxSplitNode::Pane { pane_id } if pane_id == &first_id))
        );
        assert!(
            leaves.iter().any(
                |node| matches!(node, MuxSplitNode::Pane { pane_id } if pane_id == &second_id)
            )
        );

        drop(service);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rust_catalog_matches_checked_in_transport_contract() {
        let contract: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../contracts/control-contract.json"
        )))
        .unwrap();
        assert_eq!(
            contract["contractVersion"],
            serde_json::Value::from(CONTROL_CONTRACT_VERSION)
        );
        assert_eq!(
            contract["initialOperations"],
            serde_json::to_value(InProcessControlService::descriptors()).unwrap()
        );
    }

    #[test]
    fn access_levels_are_ordered() {
        assert!(ControlAccess::Control.allows(&ControlAccess::Read));
        assert!(ControlAccess::Control.allows(&ControlAccess::Write));
        assert!(!ControlAccess::Write.allows(&ControlAccess::Control));
        assert!(ControlAccess::Write.allows(&ControlAccess::Read));
        assert!(!ControlAccess::Read.allows(&ControlAccess::Write));
    }
}
