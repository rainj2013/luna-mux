use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    local_pty_backend::InProcessLocalPtyTerminalBackend,
    ssh_terminal_backend::InProcessSshTerminalBackend,
    terminal_backend::{TerminalBackend, TerminalBackendResult, TerminalRuntimeEventSink},
    terminal_runtime_contract::{
        TerminalRuntime, TerminalRuntimeCreateRequest, TerminalRuntimeOutputReadResult,
        TerminalTarget,
    },
};

pub struct CompositeTerminalBackend {
    ssh: Arc<InProcessSshTerminalBackend>,
    local: Arc<InProcessLocalPtyTerminalBackend>,
}

impl CompositeTerminalBackend {
    pub fn new(
        ssh: Arc<InProcessSshTerminalBackend>,
        local: Arc<InProcessLocalPtyTerminalBackend>,
    ) -> Arc<Self> {
        Arc::new(Self { ssh, local })
    }

    fn local_runtime(&self, runtime_id: &str) -> TerminalBackendResult<bool> {
        Ok(self
            .local
            .list()?
            .into_iter()
            .any(|runtime| runtime.runtime_id == runtime_id))
    }
}

#[async_trait]
impl TerminalBackend for CompositeTerminalBackend {
    fn set_event_sink(&self, sink: TerminalRuntimeEventSink) {
        self.ssh.set_event_sink(sink.clone());
        self.local.set_event_sink(sink);
    }

    fn targets(&self) -> TerminalBackendResult<Vec<TerminalTarget>> {
        let mut targets = self.ssh.targets()?;
        targets.extend(self.local.targets()?);
        Ok(targets)
    }

    fn list(&self) -> TerminalBackendResult<Vec<TerminalRuntime>> {
        let mut runtimes = self.ssh.list()?;
        runtimes.extend(self.local.list()?);
        Ok(runtimes)
    }

    async fn create(
        &self,
        request: TerminalRuntimeCreateRequest,
    ) -> TerminalBackendResult<TerminalRuntime> {
        if InProcessLocalPtyTerminalBackend::is_local_target(&request.target_id) {
            self.local.create(request).await
        } else {
            self.ssh.create(request).await
        }
    }

    async fn write(&self, runtime_id: &str, data: &str) -> TerminalBackendResult<()> {
        if self.local_runtime(runtime_id)? {
            self.local.write(runtime_id, data).await
        } else {
            self.ssh.write(runtime_id, data).await
        }
    }

    async fn resize(&self, runtime_id: &str, cols: u32, rows: u32) -> TerminalBackendResult<()> {
        if self.local_runtime(runtime_id)? {
            self.local.resize(runtime_id, cols, rows).await
        } else {
            self.ssh.resize(runtime_id, cols, rows).await
        }
    }

    fn set_output_paused(&self, runtime_id: &str, paused: bool) -> TerminalBackendResult<()> {
        if self.local_runtime(runtime_id)? {
            self.local.set_output_paused(runtime_id, paused)
        } else {
            self.ssh.set_output_paused(runtime_id, paused)
        }
    }

    async fn interrupt(&self, runtime_id: &str) -> TerminalBackendResult<()> {
        if self.local_runtime(runtime_id)? {
            self.local.interrupt(runtime_id).await
        } else {
            self.ssh.interrupt(runtime_id).await
        }
    }

    async fn close(&self, runtime_id: &str) -> TerminalBackendResult<()> {
        if self.local_runtime(runtime_id)? {
            self.local.close(runtime_id).await
        } else {
            self.ssh.close(runtime_id).await
        }
    }

    fn read_output(
        &self,
        runtime_id: &str,
        from_cursor: u64,
        max_bytes: usize,
    ) -> TerminalBackendResult<TerminalRuntimeOutputReadResult> {
        if self.local_runtime(runtime_id)? {
            self.local.read_output(runtime_id, from_cursor, max_bytes)
        } else {
            self.ssh.read_output(runtime_id, from_cursor, max_bytes)
        }
    }
}
