use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use serde_json::json;

use crate::control_contract::{
    CONTROL_CONTRACT_VERSION, ControlAccess, ControlCaller, ControlCallerKind, ControlCatalog,
    ControlError, ControlErrorCode, ControlEvent, ControlEventReadResult,
    ControlOperationDescriptor, ControlRequest, ControlResourceKind, ControlResponse,
    ControlResult,
};
use crate::terminal_backend::TerminalBackend;
use crate::terminal_runtime_contract::TerminalRuntimeEvent;

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
        let mut inner = self.inner.lock().expect("control event buffer lock");
        let sequence = inner.next_sequence;
        inner.next_sequence = inner.next_sequence.saturating_add(1);
        inner.events.push_back(ControlEvent {
            sequence,
            event_type: event_type.into(),
            resource: resource
                .map(|(kind, id)| crate::control_contract::ControlResourceRef { kind, id }),
            payload,
        });
        while inner.events.len() > EVENT_CAPACITY {
            inner.events.pop_front();
        }
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
    backend: Arc<dyn TerminalBackend>,
    events: Arc<ControlEventBuffer>,
    idempotent_results: Mutex<HashMap<(String, String, String), ControlResponse>>,
}

impl InProcessControlService {
    pub fn new(backend: Arc<dyn TerminalBackend>) -> Arc<Self> {
        Arc::new(Self {
            backend,
            events: Arc::new(ControlEventBuffer::default()),
            idempotent_results: Mutex::new(HashMap::new()),
        })
    }

    pub fn event_buffer(&self) -> Arc<ControlEventBuffer> {
        self.events.clone()
    }

    fn descriptors() -> Vec<ControlOperationDescriptor> {
        vec![
            ControlOperationDescriptor {
                name: "terminal.targets.list".into(),
                version: 1,
                access: ControlAccess::Read,
                resource_kind: ControlResourceKind::TerminalTarget,
                mutating: false,
                supports_idempotency: true,
            },
            ControlOperationDescriptor {
                name: "terminal.runtimes.list".into(),
                version: 1,
                access: ControlAccess::Read,
                resource_kind: ControlResourceKind::TerminalRuntime,
                mutating: false,
                supports_idempotency: true,
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
            })
        }
    }

    fn validate_request(request: &ControlRequest) -> ControlResult<()> {
        if request.contract_version != CONTROL_CONTRACT_VERSION {
            return Err(ControlError {
                code: ControlErrorCode::UnsupportedVersion,
                message: format!("不支持控制契约版本 {}", request.contract_version),
                retryable: false,
            });
        }
        Ok(())
    }
}

fn internal_ui_caller() -> ControlCaller {
    use ControlResourceKind::*;
    ControlCaller {
        caller_id: "ui".into(),
        kind: ControlCallerKind::Ui,
        grants: [
            Application,
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
        Self::validate_request(&request)?;
        let descriptor = Self::descriptor(&request.operation).ok_or_else(|| ControlError {
            code: ControlErrorCode::UnknownOperation,
            message: format!("未知控制操作：{}", request.operation),
            retryable: false,
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
                });
            }
        }
        Self::authorize(caller, &descriptor, requested_id)?;
        if !request.arguments.is_object() {
            return Err(ControlError {
                code: ControlErrorCode::InvalidArguments,
                message: "arguments 必须是 JSON 对象".into(),
                retryable: false,
            });
        }
        let idempotency_key = request
            .idempotency_key
            .as_deref()
            .filter(|key| !key.is_empty());
        if let Some(key) = idempotency_key {
            if !descriptor.supports_idempotency {
                return Err(ControlError {
                    code: ControlErrorCode::InvalidArguments,
                    message: "该操作不支持幂等键".into(),
                    retryable: false,
                });
            }
            if let Some(response) = self
                .idempotent_results
                .lock()
                .expect("control idempotency lock")
                .get(&(
                    caller.caller_id.clone(),
                    request.operation.clone(),
                    key.into(),
                ))
                .cloned()
            {
                return Ok(response);
            }
        }
        let result = match request.operation.as_str() {
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
                serde_json::to_value(filtered).map_err(|error| internal_error(error.to_string()))?
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
                serde_json::to_value(filtered).map_err(|error| internal_error(error.to_string()))?
            }
            _ => unreachable!(),
        };
        let response = ControlResponse {
            request_id: request.request_id,
            result,
        };
        if let Some(key) = idempotency_key {
            self.idempotent_results
                .lock()
                .expect("control idempotency lock")
                .insert(
                    (caller.caller_id.clone(), request.operation, key.into()),
                    response.clone(),
                );
        }
        Ok(response)
    }

    fn read_events(
        &self,
        caller: &ControlCaller,
        from_sequence: u64,
        limit: usize,
    ) -> ControlResult<ControlEventReadResult> {
        if !caller.can_access_any(&ControlResourceKind::Application, &ControlAccess::Read)
            && !caller.can_access_any(&ControlResourceKind::TerminalRuntime, &ControlAccess::Read)
        {
            return Err(ControlError {
                code: ControlErrorCode::Unauthorized,
                message: "调用者没有事件读取权限".into(),
                retryable: false,
            });
        }
        let mut result = self.events.read(from_sequence, limit);
        if !caller.can_access_any(&ControlResourceKind::Application, &ControlAccess::Read) {
            result.events.retain(|event| {
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

fn internal_error(message: impl Into<String>) -> ControlError {
    ControlError {
        code: ControlErrorCode::Internal,
        message: message.into(),
        retryable: false,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;

    use super::*;
    use crate::control_contract::{ControlGrant, ControlResourceKind};
    use crate::terminal_backend::TerminalBackendResult;
    use crate::terminal_runtime_contract::{
        TerminalCapabilities, TerminalRuntime, TerminalRuntimeCreateRequest, TerminalRuntimeEvent,
        TerminalRuntimeOutputReadResult, TerminalRuntimeStatus, TerminalRuntimeStatusEvent,
        TerminalTarget,
    };

    struct EmptyBackend;

    #[async_trait]
    impl TerminalBackend for EmptyBackend {
        fn set_event_sink(&self, _sink: crate::terminal_backend::TerminalRuntimeEventSink) {}
        fn targets(&self) -> TerminalBackendResult<Vec<TerminalTarget>> {
            Ok(vec![])
        }
        fn list(&self) -> TerminalBackendResult<Vec<TerminalRuntime>> {
            Ok(vec![])
        }
        async fn create(
            &self,
            _request: TerminalRuntimeCreateRequest,
        ) -> TerminalBackendResult<TerminalRuntime> {
            Err("not implemented".into())
        }
        async fn write(&self, _runtime_id: &str, _data: &str) -> TerminalBackendResult<()> {
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
        async fn interrupt(&self, _runtime_id: &str) -> TerminalBackendResult<()> {
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

    #[test]
    fn version_mismatch_is_structured() {
        let error = InProcessControlService::validate_request(&ControlRequest {
            contract_version: CONTROL_CONTRACT_VERSION + 1,
            request_id: "request-1".into(),
            operation: "terminal.targets.list".into(),
            resource: None,
            arguments: serde_json::json!({}),
            idempotency_key: None,
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
        let service = InProcessControlService::new(Arc::new(EmptyBackend));
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
        };
        let first = service.invoke(&agent, request("request-1")).await.unwrap();
        let second = service.invoke(&agent, request("request-2")).await.unwrap();
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn invoke_rejects_unauthorized_and_mismatched_resource() {
        let service = InProcessControlService::new(Arc::new(EmptyBackend));
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
                },
            )
            .await
            .unwrap_err();
        assert_eq!(mismatched.code, ControlErrorCode::InvalidArguments);
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
