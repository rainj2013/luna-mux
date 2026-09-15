use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel},
    },
    thread,
    time::{Duration, Instant},
};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub(crate) const LOG_FILE_NAME: &str = "terminal-runtime-diagnostics.log";
const MAX_LOG_BYTES: u64 = 1024 * 1024;
const MAX_ROTATED_FILES: u8 = 3;
const DUPLICATE_WINDOW: Duration = Duration::from_millis(250);
const DIAGNOSTIC_QUEUE_CAPACITY: usize = 2048;

#[derive(Clone)]
pub struct TerminalInputDiagnostics {
    observations: Arc<Mutex<InputObservationState>>,
    writer: SyncSender<WriterMessage>,
    timeline_sequence: Arc<AtomicU64>,
    dropped_events: Arc<AtomicU64>,
    writer_error_reported: Arc<AtomicBool>,
}

struct InputObservationState {
    next_sequence: u64,
    fingerprint_salt: [u8; 16],
    recent_inputs: HashMap<(&'static str, String), RecentInput>,
}

struct DiagnosticFileState {
    log_path: PathBuf,
    file: Option<File>,
    bytes_written: u64,
    file_error_reported: bool,
}

enum WriterMessage {
    Line(serde_json::Value),
    #[cfg(test)]
    Flush(std::sync::mpsc::Sender<()>),
}

struct RecentInput {
    sequence: u64,
    client_input_id: Option<u64>,
    fingerprint: String,
    byte_len: usize,
    at: Instant,
}

pub struct DuplicateObservation {
    source: &'static str,
    runtime_id: String,
    previous_sequence: u64,
    sequence: u64,
    previous_client_input_id: Option<u64>,
    client_input_id: Option<u64>,
    interval_ms: u128,
    byte_len: usize,
    fingerprint: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalUiDiagnosticEvent {
    pub kind: TerminalUiDiagnosticEventKind,
    pub connected: bool,
    pub visible: bool,
    pub document_visible: bool,
    pub focused: bool,
    pub alternate_buffer: bool,
    pub pending_output: usize,
    pub output_cursor: u64,
    pub viewport_y: u64,
    pub base_y: u64,
    pub key_category: Option<TerminalUiKeyCategory>,
    pub client_input_id: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TerminalUiDiagnosticEventKind {
    Heartbeat,
    Focus,
    Blur,
    PointerDown,
    Keydown,
    Input,
    Wheel,
    Scroll,
    FlowPause,
    FlowResume,
    Dispose,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TerminalUiKeyCategory {
    Printable,
    Enter,
    Escape,
    Control,
    Navigation,
    Other,
}

impl TerminalInputDiagnostics {
    pub fn new(log_dir: PathBuf) -> Self {
        let log_path = log_dir.join(LOG_FILE_NAME);
        let _ = fs::create_dir_all(&log_dir);
        let bytes_written = fs::metadata(&log_path)
            .map(|metadata| metadata.len())
            .unwrap_or_default();
        let (writer, receiver) = sync_channel(DIAGNOSTIC_QUEUE_CAPACITY);
        let timeline_sequence = Arc::new(AtomicU64::new(1));
        let dropped_events = Arc::new(AtomicU64::new(0));
        let writer_dropped_events = dropped_events.clone();
        let writer_timeline_sequence = timeline_sequence.clone();
        let writer_error_reported = Arc::new(AtomicBool::new(false));
        let disconnected_error_reported = writer_error_reported.clone();
        let writer_log_path = log_path.clone();
        let spawn_result = thread::Builder::new()
            .name("terminal-diagnostics-writer".into())
            .spawn(move || {
                diagnostic_writer_loop(
                    DiagnosticFileState {
                        log_path: writer_log_path,
                        file: None,
                        bytes_written,
                        file_error_reported: false,
                    },
                    receiver,
                    writer_timeline_sequence,
                    writer_dropped_events,
                );
            });
        if spawn_result.is_err() {
            eprintln!(
                "Luna Mux terminal diagnostics writer unavailable: {}",
                log_path.display()
            );
            disconnected_error_reported.store(true, Ordering::Release);
        }

        Self {
            observations: Arc::new(Mutex::new(InputObservationState {
                next_sequence: 1,
                fingerprint_salt: *Uuid::new_v4().as_bytes(),
                recent_inputs: HashMap::new(),
            })),
            writer,
            timeline_sequence,
            dropped_events,
            writer_error_reported,
        }
    }

    /// Observe only non-ASCII committed input. A record is emitted by the caller only when
    /// the same payload arrives again for the same runtime within the short diagnostic window.
    pub fn observe(
        &self,
        source: &'static str,
        runtime_id: &str,
        data: &str,
        client_input_id: Option<u64>,
    ) -> Option<DuplicateObservation> {
        if data.is_empty()
            || !data
                .chars()
                .any(|character| !character.is_ascii() && !character.is_control())
        {
            return None;
        }

        let now = Instant::now();
        let byte_len = data.len();
        let mut state = self.observations.lock().ok()?;
        if state.recent_inputs.len() >= 256 {
            state
                .recent_inputs
                .retain(|_, recent| now.duration_since(recent.at) <= Duration::from_secs(10));
        }
        let fingerprint = fingerprint(&state.fingerprint_salt, data);
        let sequence = state.next_sequence;
        state.next_sequence = state.next_sequence.saturating_add(1);
        let previous = state.recent_inputs.insert(
            (source, runtime_id.to_owned()),
            RecentInput {
                sequence,
                client_input_id,
                fingerprint: fingerprint.clone(),
                byte_len,
                at: now,
            },
        );

        let previous = previous?;
        let interval = now.duration_since(previous.at);
        if previous.fingerprint != fingerprint || interval > DUPLICATE_WINDOW {
            return None;
        }

        Some(DuplicateObservation {
            source,
            runtime_id: runtime_id.to_owned(),
            previous_sequence: previous.sequence,
            sequence,
            previous_client_input_id: previous.client_input_id,
            client_input_id,
            interval_ms: interval.as_millis(),
            byte_len: previous.byte_len.max(byte_len),
            fingerprint,
        })
    }

    pub fn record_observation(&self, observation: DuplicateObservation, status: &'static str) {
        self.append_json(json!({
            "ts": Utc::now().to_rfc3339(),
            "event": "possible_duplicate_input",
            "source": observation.source,
            "status": status,
            "runtimeId": observation.runtime_id,
            "previousSequence": observation.previous_sequence,
            "sequence": observation.sequence,
            "previousClientInputId": observation.previous_client_input_id,
            "clientInputId": observation.client_input_id,
            "intervalMs": observation.interval_ms,
            "byteLen": observation.byte_len,
            "fingerprint": observation.fingerprint,
        }));
    }

    pub fn record_ui_event(&self, runtime_id: &str, event: TerminalUiDiagnosticEvent) {
        self.append_json(json!({
            "ts": Utc::now().to_rfc3339(),
            "event": "terminal_ui",
            "runtimeId": runtime_id,
            "details": event,
        }));
    }

    pub fn record_runtime_event(
        &self,
        runtime_id: &str,
        event: &'static str,
        details: serde_json::Value,
    ) {
        self.append_json(json!({
            "ts": Utc::now().to_rfc3339(),
            "event": event,
            "runtimeId": runtime_id,
            "details": details,
        }));
    }

    fn append_json(&self, mut value: serde_json::Value) {
        let timeline_sequence = self.timeline_sequence.fetch_add(1, Ordering::Relaxed);
        let Some(object) = value.as_object_mut() else {
            return;
        };
        object.insert("timelineSequence".into(), timeline_sequence.into());
        match self.writer.try_send(WriterMessage::Line(value)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.dropped_events.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => {
                if !self.writer_error_reported.swap(true, Ordering::AcqRel) {
                    eprintln!("Luna Mux terminal diagnostics writer disconnected");
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn flush(&self) {
        let (sender, receiver) = std::sync::mpsc::channel();
        if self.writer.send(WriterMessage::Flush(sender)).is_ok() {
            let _ = receiver.recv_timeout(Duration::from_secs(2));
        }
    }
}

fn diagnostic_writer_loop(
    mut state: DiagnosticFileState,
    receiver: Receiver<WriterMessage>,
    timeline_sequence: Arc<AtomicU64>,
    dropped_events: Arc<AtomicU64>,
) {
    loop {
        match receiver.recv_timeout(Duration::from_secs(1)) {
            Ok(WriterMessage::Line(value)) => append_json_line(&mut state, value),
            #[cfg(test)]
            Ok(WriterMessage::Flush(sender)) => {
                record_dropped_events(&mut state, &timeline_sequence, &dropped_events);
                let _ = sender.send(());
            }
            Err(RecvTimeoutError::Timeout) => {
                record_dropped_events(&mut state, &timeline_sequence, &dropped_events);
            }
            Err(RecvTimeoutError::Disconnected) => {
                record_dropped_events(&mut state, &timeline_sequence, &dropped_events);
                break;
            }
        }
    }
}

fn record_dropped_events(
    state: &mut DiagnosticFileState,
    timeline_sequence: &AtomicU64,
    dropped_events: &AtomicU64,
) {
    let dropped = dropped_events.swap(0, Ordering::AcqRel);
    if dropped == 0 {
        return;
    }
    append_json_line(
        state,
        json!({
            "ts": Utc::now().to_rfc3339(),
            "event": "diagnostic_events_dropped",
            "timelineSequence": timeline_sequence.fetch_add(1, Ordering::Relaxed),
            "details": { "count": dropped },
        }),
    );
}

fn append_json_line(state: &mut DiagnosticFileState, value: serde_json::Value) {
    let mut line = match serde_json::to_vec(&value) {
        Ok(line) => line,
        Err(_) => return,
    };
    line.push(b'\n');
    if line.len() as u64 > MAX_LOG_BYTES {
        return;
    }
    if state.file.is_none() && !open_log_file(state) {
        return;
    }
    if state.bytes_written.saturating_add(line.len() as u64) > MAX_LOG_BYTES && !rotate_log(state) {
        return;
    }
    let Some(file) = state.file.as_mut() else {
        report_file_error(state);
        return;
    };
    if file.write_all(&line).and_then(|_| file.flush()).is_ok() {
        state.bytes_written = state.bytes_written.saturating_add(line.len() as u64);
    } else {
        state.file.take();
        report_file_error(state);
    }
}

fn open_log_file(state: &mut DiagnosticFileState) -> bool {
    state.file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&state.log_path)
        .ok();
    state.bytes_written = state
        .file
        .as_ref()
        .and_then(|file| file.metadata().ok())
        .map(|metadata| metadata.len())
        .unwrap_or_default();
    if state.file.is_none() {
        report_file_error(state);
        return false;
    }
    true
}

fn fingerprint(salt: &[u8; 16], data: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(salt);
    hasher.update(data.as_bytes());
    let digest = hasher.finalize();
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn rotate_log(state: &mut DiagnosticFileState) -> bool {
    state.file.take();
    for index in (1..=MAX_ROTATED_FILES).rev() {
        let destination = rotated_path(&state.log_path, index);
        if index == MAX_ROTATED_FILES {
            match fs::remove_file(&destination) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => {
                    report_file_error(state);
                    return false;
                }
            }
        }
        let source = if index == 1 {
            state.log_path.clone()
        } else {
            rotated_path(&state.log_path, index - 1)
        };
        match fs::rename(source, destination) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                report_file_error(state);
                return false;
            }
        }
    }
    state.bytes_written = 0;
    open_log_file(state)
}

fn rotated_path(path: &Path, index: u8) -> PathBuf {
    PathBuf::from(format!("{}.{index}", path.display()))
}

fn report_file_error(state: &mut DiagnosticFileState) {
    if !state.file_error_reported {
        eprintln!(
            "Luna Mux terminal input diagnostics unavailable: {}",
            state.log_path.display()
        );
        state.file_error_reported = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_log_dir() -> PathBuf {
        std::env::temp_dir().join(format!("luna-mux-terminal-input-{}", Uuid::new_v4()))
    }

    #[test]
    fn records_only_repeated_non_ascii_input() {
        let directory = test_log_dir();
        let diagnostics = TerminalInputDiagnostics::new(directory.clone());
        assert!(
            diagnostics
                .observe("runtime_command", "runtime-1", "好", Some(7))
                .is_none()
        );
        let observation = diagnostics
            .observe("runtime_command", "runtime-1", "好", Some(8))
            .expect("duplicate observation");
        diagnostics.record_observation(observation, "ok");
        diagnostics.flush();

        let contents = fs::read_to_string(directory.join(LOG_FILE_NAME)).expect("log file");
        assert!(contents.contains("possible_duplicate_input"));
        assert!(contents.contains("\"previousClientInputId\":7"));
        assert!(contents.contains("\"clientInputId\":8"));
        assert!(!contents.contains("好"));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn ignores_ascii_input() {
        let directory = test_log_dir();
        let diagnostics = TerminalInputDiagnostics::new(directory.clone());
        assert!(
            diagnostics
                .observe("runtime_command", "runtime-1", "aa", Some(1))
                .is_none()
        );
        assert!(
            diagnostics
                .observe("runtime_command", "runtime-1", "aa", Some(2))
                .is_none()
        );
        assert!(!directory.join(LOG_FILE_NAME).exists());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn runtime_events_record_metadata_without_terminal_content() {
        let directory = test_log_dir();
        let diagnostics = TerminalInputDiagnostics::new(directory.clone());
        diagnostics.record_runtime_event(
            "runtime-1",
            "pty_write_completed",
            json!({ "byteLen": 12, "elapsedMs": 4, "status": "ok" }),
        );
        diagnostics.record_ui_event(
            "runtime-1",
            TerminalUiDiagnosticEvent {
                kind: TerminalUiDiagnosticEventKind::Keydown,
                connected: true,
                visible: true,
                document_visible: true,
                focused: true,
                alternate_buffer: true,
                pending_output: 0,
                output_cursor: 42,
                viewport_y: 3,
                base_y: 3,
                key_category: Some(TerminalUiKeyCategory::Printable),
                client_input_id: None,
            },
        );
        diagnostics.flush();

        let contents = fs::read_to_string(directory.join(LOG_FILE_NAME)).expect("log file");
        assert!(contents.contains("\"event\":\"pty_write_completed\""));
        assert!(contents.contains("\"kind\":\"keydown\""));
        assert!(contents.contains("\"keyCategory\":\"printable\""));
        assert!(!contents.contains("terminal text"));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn ui_events_reject_terminal_content_fields() {
        let event = serde_json::from_value::<TerminalUiDiagnosticEvent>(json!({
            "kind": "input",
            "connected": true,
            "visible": true,
            "documentVisible": true,
            "focused": true,
            "alternateBuffer": true,
            "pendingOutput": 0,
            "outputCursor": 42,
            "viewportY": 3,
            "baseY": 3,
            "data": "terminal text"
        }));

        assert!(event.is_err());
    }

    #[test]
    fn rotates_logs_without_exceeding_the_retention_limit() {
        let directory = test_log_dir();
        let diagnostics = TerminalInputDiagnostics::new(directory.clone());
        for batch in 0..20 {
            for index in 0..1_000 {
                diagnostics.record_runtime_event(
                    "runtime-1",
                    "rotation_test",
                    json!({ "index": batch * 1_000 + index }),
                );
            }
            diagnostics.flush();
        }

        let retained = std::iter::once(directory.join(LOG_FILE_NAME))
            .chain(
                (1..=MAX_ROTATED_FILES)
                    .map(|index| rotated_path(&directory.join(LOG_FILE_NAME), index)),
            )
            .filter(|path| path.is_file())
            .collect::<Vec<_>>();
        assert!(directory.join(format!("{LOG_FILE_NAME}.1")).is_file());
        assert!(retained.len() <= MAX_ROTATED_FILES as usize + 1);
        assert!(
            retained
                .iter()
                .all(|path| fs::metadata(path).unwrap().len() <= MAX_LOG_BYTES)
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn failed_rotation_does_not_grow_the_active_log() {
        let directory = test_log_dir();
        fs::create_dir_all(&directory).unwrap();
        let log_path = directory.join(LOG_FILE_NAME);
        fs::write(&log_path, vec![b'x'; MAX_LOG_BYTES as usize]).unwrap();
        fs::create_dir(rotated_path(&log_path, MAX_ROTATED_FILES)).unwrap();
        let diagnostics = TerminalInputDiagnostics::new(directory.clone());

        diagnostics.record_runtime_event("runtime-1", "rotation_test", json!({}));
        diagnostics.flush();

        assert_eq!(fs::metadata(&log_path).unwrap().len(), MAX_LOG_BYTES);
        let _ = fs::remove_dir_all(directory);
    }
}
