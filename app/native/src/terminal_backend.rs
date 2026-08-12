use async_trait::async_trait;
use std::sync::Arc;

use crate::terminal_runtime_contract::{
    TerminalRuntime, TerminalRuntimeCreateRequest, TerminalRuntimeEvent,
    TerminalRuntimeOutputReadResult, TerminalTarget,
};

pub type TerminalBackendResult<T> = Result<T, String>;
pub type TerminalRuntimeEventSink = Arc<dyn Fn(TerminalRuntimeEvent) + Send + Sync>;

#[async_trait]
pub trait TerminalBackend: Send + Sync {
    fn set_event_sink(&self, sink: TerminalRuntimeEventSink);
    fn targets(&self) -> Vec<TerminalTarget>;
    fn list(&self) -> Vec<TerminalRuntime>;

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
