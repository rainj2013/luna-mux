use async_trait::async_trait;
use std::sync::Arc;

use crate::session_contract::{
    ManagedSession, SessionCreateRequest, SessionEvent, SessionOutputReadResult, SessionTarget,
};

pub type SessionBackendResult<T> = Result<T, String>;
pub type SessionEventSink = Arc<dyn Fn(SessionEvent) + Send + Sync>;

#[async_trait]
pub trait SessionBackend: Send + Sync {
    fn set_event_sink(&self, sink: SessionEventSink);
    fn targets(&self) -> Vec<SessionTarget>;
    fn list(&self) -> Vec<ManagedSession>;

    async fn create(&self, request: SessionCreateRequest) -> SessionBackendResult<ManagedSession>;
    async fn write(&self, session_id: &str, data: &str) -> SessionBackendResult<()>;
    async fn resize(&self, session_id: &str, cols: u32, rows: u32) -> SessionBackendResult<()>;
    fn set_output_paused(&self, session_id: &str, paused: bool) -> SessionBackendResult<()>;
    async fn interrupt(&self, session_id: &str) -> SessionBackendResult<()>;
    async fn close(&self, session_id: &str) -> SessionBackendResult<()>;
    fn read_output(
        &self,
        session_id: &str,
        from_cursor: u64,
        max_bytes: usize,
    ) -> SessionBackendResult<SessionOutputReadResult>;
}
