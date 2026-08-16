use async_trait::async_trait;
use std::sync::Arc;

use crate::terminal_runtime_contract::{
    TerminalRuntime, TerminalRuntimeCreateRequest, TerminalRuntimeEvent,
    TerminalRuntimeOutputReadResult, TerminalTarget,
};

pub type TerminalBackendResult<T> = Result<T, String>;
pub type TerminalRuntimeEventSink = Arc<dyn Fn(TerminalRuntimeEvent) + Send + Sync>;

/// Every terminal transport exposes the same interactive controls. Remote
/// file and forwarding tools are additive SSH capabilities, not a different
/// terminal UI contract.
pub fn standard_terminal_capabilities(
    remote_tools: bool,
) -> crate::terminal_runtime_contract::TerminalCapabilities {
    crate::terminal_runtime_contract::TerminalCapabilities {
        terminal: true,
        resize: true,
        flow_control: true,
        interrupt: true,
        output_cursor: true,
        remote_files: remote_tools,
        port_forwarding: remote_tools,
    }
}

#[async_trait]
pub trait TerminalBackend: Send + Sync {
    fn set_event_sink(&self, sink: TerminalRuntimeEventSink);
    fn targets(&self) -> TerminalBackendResult<Vec<TerminalTarget>>;
    fn list(&self) -> TerminalBackendResult<Vec<TerminalRuntime>>;

    async fn create(
        &self,
        request: TerminalRuntimeCreateRequest,
    ) -> TerminalBackendResult<TerminalRuntime>;
    async fn write(&self, runtime_id: &str, data: &str) -> TerminalBackendResult<()>;
    async fn resize(&self, runtime_id: &str, cols: u32, rows: u32) -> TerminalBackendResult<()>;
    fn set_output_paused(&self, runtime_id: &str, paused: bool) -> TerminalBackendResult<()>;
    async fn interrupt(&self, runtime_id: &str) -> TerminalBackendResult<()>;
    async fn close(&self, runtime_id: &str) -> TerminalBackendResult<()>;
    fn read_output(
        &self,
        runtime_id: &str,
        from_cursor: u64,
        max_bytes: usize,
    ) -> TerminalBackendResult<TerminalRuntimeOutputReadResult>;
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use uuid::Uuid;

    use super::*;
    use crate::terminal_runtime_contract::{
        TerminalCapabilities, TerminalRuntimeCreateRequest, TerminalRuntimeOutputEvent,
        TerminalRuntimeStatus, TerminalRuntimeStatusEvent, TerminalTargetKind, TerminalTransport,
    };

    struct MockBackend {
        runtimes: Mutex<Vec<TerminalRuntime>>,
        writes: Mutex<Vec<(String, String)>>,
        events: Mutex<Vec<TerminalRuntimeEvent>>,
    }

    impl MockBackend {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                runtimes: Mutex::new(Vec::new()),
                writes: Mutex::new(Vec::new()),
                events: Mutex::new(Vec::new()),
            })
        }

        fn capabilities() -> TerminalCapabilities {
            TerminalCapabilities {
                terminal: true,
                resize: true,
                flow_control: true,
                interrupt: true,
                output_cursor: true,
                remote_files: false,
                port_forwarding: false,
            }
        }

        fn record_output(&self, runtime_id: &str, data: &str) {
            self.events
                .lock()
                .expect("mock events lock")
                .push(TerminalRuntimeEvent::Output(
                    TerminalRuntimeOutputEvent::new(runtime_id, 0, data),
                ));
        }
    }

    #[async_trait]
    impl TerminalBackend for MockBackend {
        fn set_event_sink(&self, sink: TerminalRuntimeEventSink) {
            if let Some(runtime) = self
                .runtimes
                .lock()
                .expect("mock runtimes lock")
                .last()
                .cloned()
            {
                sink(TerminalRuntimeEvent::Status(TerminalRuntimeStatusEvent {
                    runtime,
                }));
            }
        }

        fn targets(&self) -> TerminalBackendResult<Vec<TerminalTarget>> {
            Ok(vec![TerminalTarget {
                id: "mock-target".into(),
                label: "Mock terminal".into(),
                transport: TerminalTransport::LocalPty,
                kind: TerminalTargetKind::Powershell,
                capabilities: Self::capabilities(),
            }])
        }

        fn list(&self) -> TerminalBackendResult<Vec<TerminalRuntime>> {
            Ok(self.runtimes.lock().expect("mock runtimes lock").clone())
        }

        async fn create(
            &self,
            request: TerminalRuntimeCreateRequest,
        ) -> TerminalBackendResult<TerminalRuntime> {
            let runtime = TerminalRuntime {
                runtime_id: request
                    .runtime_id
                    .unwrap_or_else(|| Uuid::new_v4().to_string()),
                target_id: request.target_id,
                title: request.title.unwrap_or_else(|| "Mock terminal".into()),
                status: TerminalRuntimeStatus::Running,
                capabilities: Self::capabilities(),
                context: request.context,
                managed_agent: request.managed_agent,
                error: None,
            };
            self.runtimes
                .lock()
                .expect("mock runtimes lock")
                .push(runtime.clone());
            Ok(runtime)
        }

        async fn write(&self, runtime_id: &str, data: &str) -> TerminalBackendResult<()> {
            if !self
                .runtimes
                .lock()
                .expect("mock runtimes lock")
                .iter()
                .any(|runtime| runtime.runtime_id == runtime_id)
            {
                return Err("mock runtime not found".into());
            }
            self.writes
                .lock()
                .expect("mock writes lock")
                .push((runtime_id.into(), data.into()));
            Ok(())
        }

        async fn resize(
            &self,
            runtime_id: &str,
            _cols: u32,
            _rows: u32,
        ) -> TerminalBackendResult<()> {
            if self
                .runtimes
                .lock()
                .expect("mock runtimes lock")
                .iter()
                .any(|runtime| runtime.runtime_id == runtime_id)
            {
                Ok(())
            } else {
                Err("mock runtime not found".into())
            }
        }

        fn set_output_paused(&self, runtime_id: &str, _paused: bool) -> TerminalBackendResult<()> {
            if self
                .runtimes
                .lock()
                .expect("mock runtimes lock")
                .iter()
                .any(|runtime| runtime.runtime_id == runtime_id)
            {
                Ok(())
            } else {
                Err("mock runtime not found".into())
            }
        }

        async fn interrupt(&self, runtime_id: &str) -> TerminalBackendResult<()> {
            self.write(runtime_id, "\u{3}").await
        }

        async fn close(&self, runtime_id: &str) -> TerminalBackendResult<()> {
            let mut runtimes = self.runtimes.lock().expect("mock runtimes lock");
            let runtime = runtimes
                .iter_mut()
                .find(|runtime| runtime.runtime_id == runtime_id)
                .ok_or_else(|| "mock runtime not found".to_string())?;
            runtime.status = TerminalRuntimeStatus::Exited;
            Ok(())
        }

        fn read_output(
            &self,
            runtime_id: &str,
            from_cursor: u64,
            _max_bytes: usize,
        ) -> TerminalBackendResult<TerminalRuntimeOutputReadResult> {
            let event = self
                .events
                .lock()
                .expect("mock events lock")
                .iter()
                .find_map(|event| match event {
                    TerminalRuntimeEvent::Output(output) if output.runtime_id == runtime_id => {
                        Some(output.clone())
                    }
                    _ => None,
                });
            let (earliest_cursor, next_cursor, data) = event
                .map(|output| (output.start_cursor, output.end_cursor, output.data))
                .unwrap_or((0, 0, String::new()));
            Ok(TerminalRuntimeOutputReadResult {
                runtime_id: runtime_id.into(),
                requested_cursor: from_cursor,
                earliest_cursor,
                next_cursor,
                truncated: from_cursor < earliest_cursor,
                data,
            })
        }
    }

    #[tokio::test]
    async fn mock_backend_supports_the_runtime_lifecycle_contract() {
        let backend = MockBackend::new();
        let runtime = backend
            .create(TerminalRuntimeCreateRequest {
                runtime_id: None,
                context: None,
                target_id: "mock-target".into(),
                title: None,
                cwd: None,
                command: None,
                authentication: None,
                managed_agent: None,
                launch_environment: Default::default(),
                cols: 80,
                rows: 24,
            })
            .await
            .unwrap();
        assert_eq!(backend.list().unwrap().len(), 1);
        backend
            .write(&runtime.runtime_id, "echo hi\r")
            .await
            .unwrap();
        backend.resize(&runtime.runtime_id, 100, 30).await.unwrap();
        backend
            .set_output_paused(&runtime.runtime_id, true)
            .unwrap();
        backend.interrupt(&runtime.runtime_id).await.unwrap();
        assert_eq!(backend.writes.lock().unwrap().len(), 2);
        backend.close(&runtime.runtime_id).await.unwrap();
        assert_eq!(
            backend.list().unwrap()[0].status,
            TerminalRuntimeStatus::Exited
        );
    }

    #[test]
    fn mock_backend_exposes_target_capabilities_without_ssh_tools() {
        let backend = MockBackend::new();
        let target = backend.targets().unwrap().remove(0);
        assert_eq!(target.transport, TerminalTransport::LocalPty);
        assert!(!target.capabilities.remote_files);
        assert!(!target.capabilities.port_forwarding);
    }

    #[test]
    fn local_and_ssh_terminal_capabilities_differ_only_by_remote_tools() {
        let local = standard_terminal_capabilities(false);
        let ssh = standard_terminal_capabilities(true);
        assert_eq!(local.terminal, ssh.terminal);
        assert_eq!(local.resize, ssh.resize);
        assert_eq!(local.flow_control, ssh.flow_control);
        assert_eq!(local.interrupt, ssh.interrupt);
        assert_eq!(local.output_cursor, ssh.output_cursor);
        assert!(!local.remote_files && ssh.remote_files);
        assert!(!local.port_forwarding && ssh.port_forwarding);
    }

    #[tokio::test]
    async fn mock_backend_preserves_utf8_output_cursor_reads() {
        let backend = MockBackend::new();
        let runtime = backend
            .create(TerminalRuntimeCreateRequest {
                runtime_id: None,
                context: None,
                target_id: "mock-target".into(),
                title: None,
                cwd: None,
                command: None,
                authentication: None,
                managed_agent: None,
                launch_environment: Default::default(),
                cols: 80,
                rows: 24,
            })
            .await
            .unwrap();
        backend.record_output(&runtime.runtime_id, "A中");
        let result = backend.read_output(&runtime.runtime_id, 0, 32).unwrap();
        assert_eq!(result.data, "A中");
        assert_eq!(result.next_cursor, 4);
        assert!(!result.truncated);
    }
}
