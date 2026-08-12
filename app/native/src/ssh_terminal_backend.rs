use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, RwLock},
};

use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    database::Database,
    models::{AppEvent, ConnectInput, SessionStatus, SessionSummary},
    sessions::SessionManager,
    terminal_backend::{TerminalBackend, TerminalBackendResult, TerminalRuntimeEventSink},
    terminal_runtime_contract::{
        TerminalCapabilities, TerminalRuntime, TerminalRuntimeAuthentication,
        TerminalRuntimeCreateRequest, TerminalRuntimeEvent, TerminalRuntimeExitEvent,
        TerminalRuntimeExitReason, TerminalRuntimeOutputEvent, TerminalRuntimeOutputReadResult,
        TerminalRuntimeStatus, TerminalRuntimeStatusEvent, TerminalTarget, TerminalTargetKind,
        TerminalTransport,
    },
};

const SSH_TARGET_PREFIX: &str = "ssh-bookmark:";
const OUTPUT_CAPACITY_BYTES: usize = 1024 * 1024;

struct RuntimeRecord {
    runtime: TerminalRuntime,
    output: OutputBuffer,
    initial_input: Option<String>,
    close_requested: bool,
}

pub struct InProcessSshTerminalBackend {
    database: Arc<Database>,
    sessions: Arc<SessionManager>,
    runtimes: RwLock<HashMap<String, RuntimeRecord>>,
    event_sink: RwLock<Option<TerminalRuntimeEventSink>>,
}

impl InProcessSshTerminalBackend {
    pub fn new(database: Arc<Database>, sessions: Arc<SessionManager>) -> Arc<Self> {
        let backend = Arc::new(Self {
            database,
            sessions: sessions.clone(),
            runtimes: RwLock::new(HashMap::new()),
            event_sink: RwLock::new(None),
        });
        let weak = Arc::downgrade(&backend);
        sessions.set_event_observer(Arc::new(move |event| {
            if let Some(backend) = weak.upgrade() {
                backend.observe_ssh_event(event);
            }
        }));
        backend
    }

    pub fn target_id(bookmark_id: &str) -> String {
        format!("{SSH_TARGET_PREFIX}{bookmark_id}")
    }

    pub fn bookmark_id(target_id: &str) -> TerminalBackendResult<&str> {
        target_id
            .strip_prefix(SSH_TARGET_PREFIX)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "目标不是 SSH 书签".into())
    }

    pub fn find_active_by_target(&self, target_id: &str) -> Option<TerminalRuntime> {
        self.runtimes
            .read()
            .expect("SSH runtime lock")
            .values()
            .map(|record| &record.runtime)
            .find(|runtime| {
                runtime.target_id == target_id
                    && matches!(
                        runtime.status,
                        TerminalRuntimeStatus::Starting
                            | TerminalRuntimeStatus::Connecting
                            | TerminalRuntimeStatus::Running
                    )
            })
            .cloned()
    }

    pub fn legacy_summary(runtime: &TerminalRuntime) -> TerminalBackendResult<SessionSummary> {
        Ok(SessionSummary {
            id: runtime.runtime_id.clone(),
            bookmark_id: Self::bookmark_id(&runtime.target_id)?.into(),
            title: runtime.title.clone(),
            status: match runtime.status {
                TerminalRuntimeStatus::Starting | TerminalRuntimeStatus::Connecting => {
                    SessionStatus::Connecting
                }
                TerminalRuntimeStatus::Running => SessionStatus::Connected,
                TerminalRuntimeStatus::Exited => SessionStatus::Disconnected,
                TerminalRuntimeStatus::Error => SessionStatus::Error,
            },
            error: runtime.error.clone(),
        })
    }

    fn capabilities() -> TerminalCapabilities {
        TerminalCapabilities {
            terminal: true,
            resize: true,
            flow_control: true,
            interrupt: true,
            output_cursor: true,
            remote_files: true,
            port_forwarding: true,
        }
    }

    fn emit(&self, event: TerminalRuntimeEvent) {
        if let Some(sink) = self
            .event_sink
            .read()
            .expect("runtime event sink lock")
            .as_ref()
        {
            sink(event);
        }
    }

    fn observe_ssh_event(self: &Arc<Self>, event: AppEvent) {
        match event {
            AppEvent::Session(summary) => self.observe_status(summary),
            AppEvent::TerminalData(output) => {
                let normalized = {
                    let mut runtimes = self.runtimes.write().expect("SSH runtime lock");
                    runtimes
                        .get_mut(&output.session_id)
                        .map(|record| record.output.push(&output.session_id, output.data))
                };
                if let Some(event) = normalized {
                    self.emit(TerminalRuntimeEvent::Output(event));
                }
            }
            _ => {}
        }
    }

    fn observe_status(self: &Arc<Self>, summary: SessionSummary) {
        let (status_event, exit_event, initial_input) = {
            let mut runtimes = self.runtimes.write().expect("SSH runtime lock");
            let Some(record) = runtimes.get_mut(&summary.id) else {
                return;
            };
            record.runtime.status = match summary.status {
                SessionStatus::Connecting => TerminalRuntimeStatus::Connecting,
                SessionStatus::Connected => TerminalRuntimeStatus::Running,
                SessionStatus::Disconnected => TerminalRuntimeStatus::Exited,
                SessionStatus::Error => TerminalRuntimeStatus::Error,
            };
            record.runtime.error = summary.error.clone();
            let status_event = TerminalRuntimeEvent::Status(TerminalRuntimeStatusEvent {
                runtime: record.runtime.clone(),
            });
            let exit_event = match summary.status {
                SessionStatus::Disconnected | SessionStatus::Error => {
                    Some(TerminalRuntimeEvent::Exit(TerminalRuntimeExitEvent {
                        runtime_id: summary.id.clone(),
                        reason: if summary.status == SessionStatus::Error {
                            TerminalRuntimeExitReason::Failed
                        } else if record.close_requested {
                            TerminalRuntimeExitReason::Closed
                        } else {
                            TerminalRuntimeExitReason::ConnectionLost
                        },
                        cursor: record.output.next_cursor,
                        exit_code: None,
                        signal: None,
                        message: summary.error,
                    }))
                }
                _ => None,
            };
            let initial_input = (summary.status == SessionStatus::Connected)
                .then(|| record.initial_input.take())
                .flatten();
            (status_event, exit_event, initial_input)
        };
        self.emit(status_event);
        if let Some(event) = exit_event {
            self.emit(event);
        }
        if let Some(input) = initial_input {
            let sessions = self.sessions.clone();
            tauri::async_runtime::spawn(async move {
                let _ = sessions.write(&summary.id, input).await;
            });
        }
    }
}

#[async_trait]
impl TerminalBackend for InProcessSshTerminalBackend {
    fn set_event_sink(&self, sink: TerminalRuntimeEventSink) {
        *self.event_sink.write().expect("runtime event sink lock") = Some(sink);
    }

    fn targets(&self) -> TerminalBackendResult<Vec<TerminalTarget>> {
        Ok(self
            .database
            .list_bookmarks()?
            .into_iter()
            .map(|bookmark| TerminalTarget {
                id: Self::target_id(&bookmark.id),
                label: bookmark.name,
                transport: TerminalTransport::Ssh,
                kind: TerminalTargetKind::Ssh,
                capabilities: Self::capabilities(),
            })
            .collect())
    }

    fn list(&self) -> TerminalBackendResult<Vec<TerminalRuntime>> {
        Ok(self
            .runtimes
            .read()
            .map_err(|_| "SSH Runtime 状态锁已损坏")?
            .values()
            .map(|record| record.runtime.clone())
            .collect())
    }

    async fn create(
        &self,
        request: TerminalRuntimeCreateRequest,
    ) -> TerminalBackendResult<TerminalRuntime> {
        let bookmark_id = Self::bookmark_id(&request.target_id)?.to_string();
        let bookmark = self
            .database
            .get_bookmark(&bookmark_id)?
            .ok_or_else(|| "SSH 目标不存在".to_string())?;
        let authentication = match request.authentication {
            Some(TerminalRuntimeAuthentication::Ssh {
                credential,
                remember_credential,
                jump_credential,
                remember_jump_credential,
            }) => (
                credential,
                remember_credential,
                jump_credential,
                remember_jump_credential,
            ),
            None => (None, false, None, false),
        };
        let runtime = TerminalRuntime {
            runtime_id: Uuid::new_v4().to_string(),
            target_id: request.target_id,
            title: request.title.unwrap_or(bookmark.name),
            status: TerminalRuntimeStatus::Starting,
            capabilities: Self::capabilities(),
            error: None,
        };
        let initial_input = initial_shell_input(request.cwd.as_deref(), request.command.as_deref());
        self.runtimes
            .write()
            .map_err(|_| "SSH Runtime 状态锁已损坏")?
            .insert(
                runtime.runtime_id.clone(),
                RuntimeRecord {
                    runtime: runtime.clone(),
                    output: OutputBuffer::new(OUTPUT_CAPACITY_BYTES),
                    initial_input,
                    close_requested: false,
                },
            );
        self.emit(TerminalRuntimeEvent::Status(TerminalRuntimeStatusEvent {
            runtime: runtime.clone(),
        }));
        let result = self.sessions.connect_with_id(
            ConnectInput {
                bookmark_id,
                new_session: true,
                credential: authentication.0,
                remember_credential: authentication.1,
                jump_credential: authentication.2,
                remember_jump_credential: authentication.3,
            },
            runtime.runtime_id.clone(),
        );
        if let Err(error) = result {
            self.runtimes
                .write()
                .expect("SSH runtime lock")
                .remove(&runtime.runtime_id);
            return Err(error);
        }
        self.sessions
            .resize(&runtime.runtime_id, request.cols, request.rows)
            .await?;
        Ok(self
            .runtimes
            .read()
            .map_err(|_| "SSH Runtime 状态锁已损坏")?
            .get(&runtime.runtime_id)
            .map(|record| record.runtime.clone())
            .unwrap_or(runtime))
    }

    async fn write(&self, runtime_id: &str, data: &str) -> TerminalBackendResult<()> {
        self.sessions.write(runtime_id, data.to_string()).await
    }

    async fn resize(&self, runtime_id: &str, cols: u32, rows: u32) -> TerminalBackendResult<()> {
        self.sessions.resize(runtime_id, cols, rows).await
    }

    fn set_output_paused(&self, runtime_id: &str, paused: bool) -> TerminalBackendResult<()> {
        self.sessions.flow(runtime_id, paused)
    }

    async fn interrupt(&self, runtime_id: &str) -> TerminalBackendResult<()> {
        self.sessions.write(runtime_id, "\u{3}".into()).await
    }

    async fn close(&self, runtime_id: &str) -> TerminalBackendResult<()> {
        let should_disconnect = {
            let mut runtimes = self
                .runtimes
                .write()
                .map_err(|_| "SSH Runtime 状态锁已损坏")?;
            let record = runtimes
                .get_mut(runtime_id)
                .ok_or_else(|| "终端 Runtime 不存在".to_string())?;
            if matches!(
                record.runtime.status,
                TerminalRuntimeStatus::Exited | TerminalRuntimeStatus::Error
            ) {
                false
            } else {
                record.close_requested = true;
                true
            }
        };
        if should_disconnect {
            self.sessions.disconnect(runtime_id).await?;
        }
        Ok(())
    }

    fn read_output(
        &self,
        runtime_id: &str,
        from_cursor: u64,
        max_bytes: usize,
    ) -> TerminalBackendResult<TerminalRuntimeOutputReadResult> {
        self.runtimes
            .read()
            .map_err(|_| "SSH Runtime 状态锁已损坏")?
            .get(runtime_id)
            .ok_or_else(|| "终端 Runtime 不存在".to_string())?
            .output
            .read(runtime_id, from_cursor, max_bytes)
    }
}

fn initial_shell_input(cwd: Option<&str>, command: Option<&str>) -> Option<String> {
    let cwd = cwd.map(str::trim).filter(|value| !value.is_empty());
    let command = command.map(str::trim).filter(|value| !value.is_empty());
    match (cwd, command) {
        (Some(cwd), Some(command)) => Some(format!("cd -- {} && {}\r", shell_quote(cwd), command)),
        (Some(cwd), None) => Some(format!("cd -- {}\r", shell_quote(cwd))),
        (None, Some(command)) => Some(format!("{command}\r")),
        (None, None) => None,
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

struct OutputChunk {
    start_cursor: u64,
    data: String,
}

struct OutputBuffer {
    capacity: usize,
    size: usize,
    next_cursor: u64,
    chunks: VecDeque<OutputChunk>,
}

impl OutputBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            size: 0,
            next_cursor: 0,
            chunks: VecDeque::new(),
        }
    }

    fn push(&mut self, runtime_id: &str, data: String) -> TerminalRuntimeOutputEvent {
        let event = TerminalRuntimeOutputEvent::new(runtime_id, self.next_cursor, data);
        self.next_cursor = event.end_cursor;
        let mut chunk = OutputChunk {
            start_cursor: event.start_cursor,
            data: event.data.clone(),
        };
        if chunk.data.len() > self.capacity {
            let mut offset = chunk.data.len() - self.capacity;
            while !chunk.data.is_char_boundary(offset) {
                offset += 1;
            }
            chunk.start_cursor += offset as u64;
            chunk.data.drain(..offset);
        }
        self.size += chunk.data.len();
        self.chunks.push_back(chunk);
        while self.size > self.capacity {
            if let Some(removed) = self.chunks.pop_front() {
                self.size -= removed.data.len();
            }
        }
        event
    }

    fn read(
        &self,
        runtime_id: &str,
        requested_cursor: u64,
        max_bytes: usize,
    ) -> TerminalBackendResult<TerminalRuntimeOutputReadResult> {
        let earliest_cursor = self
            .chunks
            .front()
            .map(|chunk| chunk.start_cursor)
            .unwrap_or(self.next_cursor);
        let cursor = requested_cursor.max(earliest_cursor).min(self.next_cursor);
        let mut data = String::new();
        let mut next_cursor = cursor;
        for chunk in &self.chunks {
            let chunk_end = chunk.start_cursor + chunk.data.len() as u64;
            if chunk_end <= next_cursor || chunk.start_cursor > next_cursor {
                continue;
            }
            let mut offset = (next_cursor - chunk.start_cursor) as usize;
            while offset < chunk.data.len() && !chunk.data.is_char_boundary(offset) {
                offset += 1;
            }
            next_cursor = chunk.start_cursor + offset as u64;
            let remaining = max_bytes.saturating_sub(data.len());
            if remaining == 0 {
                break;
            }
            let mut end = (offset + remaining).min(chunk.data.len());
            while end > offset && !chunk.data.is_char_boundary(end) {
                end -= 1;
            }
            if end == offset {
                break;
            }
            data.push_str(&chunk.data[offset..end]);
            next_cursor = chunk.start_cursor + end as u64;
            if data.len() == max_bytes {
                break;
            }
        }
        Ok(TerminalRuntimeOutputReadResult {
            runtime_id: runtime_id.into(),
            requested_cursor,
            earliest_cursor,
            next_cursor,
            truncated: requested_cursor < earliest_cursor,
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{OutputBuffer, initial_shell_input};

    #[test]
    fn output_buffer_reads_incrementally_and_reports_truncation() {
        let mut output = OutputBuffer::new(6);
        let first = output.push("runtime-1", "abc".into());
        let second = output.push("runtime-1", "中def".into());
        assert_eq!((first.start_cursor, first.end_cursor), (0, 3));
        assert_eq!((second.start_cursor, second.end_cursor), (3, 9));
        let truncated = output.read("runtime-1", 0, 32).unwrap();
        assert!(truncated.truncated);
        assert_eq!(truncated.earliest_cursor, 3);
        assert_eq!(truncated.data, "中def");
        let incremental = output.read("runtime-1", 6, 3).unwrap();
        assert!(!incremental.truncated);
        assert_eq!(incremental.data, "def");
        assert_eq!(incremental.next_cursor, 9);
    }

    #[test]
    fn initial_input_quotes_cwd_and_keeps_command_explicit() {
        assert_eq!(
            initial_shell_input(Some("/work/a'b"), Some("codex")),
            Some("cd -- '/work/a'\"'\"'b' && codex\r".into())
        );
        assert_eq!(initial_shell_input(Some("  "), None), None);
    }
}
