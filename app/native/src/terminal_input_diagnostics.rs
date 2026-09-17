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
const MAX_PANIC_MESSAGE_CHARS: usize = 512;
const DETAILED_DIAGNOSTICS_ENV: &str = "LUNA_MUX_TERMINAL_DIAGNOSTICS";

/// Records every panic and then defers to the previously installed hook.
///
/// The panics that matter here are the ones taken while a runtime's output mutex
/// is held: that poisons the mutex, so every later lock fails and the PTY reader
/// thread dies on its next chunk, leaving one pane permanently silent. The
/// default hook writes the payload and source location to stderr, which a
/// Windows GUI build discards, so without this hook such a freeze leaves no
/// trace beyond "the pane stopped updating" — and the location is the one thing
/// that cannot be recovered afterwards.
pub fn install_panic_hook(diagnostics: Arc<TerminalInputDiagnostics>) {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        diagnostics.record_panic(panic_info);
        previous_hook(panic_info);
    }));
}

#[derive(Clone)]
pub struct TerminalInputDiagnostics {
    observations: Arc<Mutex<InputObservationState>>,
    writer: SyncSender<WriterMessage>,
    timeline_sequence: Arc<AtomicU64>,
    dropped_events: Arc<AtomicU64>,
    writer_error_reported: Arc<AtomicBool>,
    detailed: bool,
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
    /// A pane can be neither connected nor connecting, which silently discards
    /// input; without this field that state is indistinguishable from a slow
    /// connect in the recorded timeline.
    pub connecting: bool,
    pub visible: bool,
    pub document_visible: bool,
    pub focused: bool,
    /// What actually holds DOM focus, and how many xterm helper textareas exist
    /// in the document. A blinking cursor with unusable input means some
    /// textarea is focused; if it is not this pane's, the pane is fine and the
    /// focus is on a leftover instance.
    pub focus_owner: Option<String>,
    pub alternate_buffer: bool,
    pub pending_output: usize,
    pub output_cursor: u64,
    pub viewport_y: u64,
    pub base_y: u64,
    pub key_category: Option<TerminalUiKeyCategory>,
    pub client_input_id: Option<u64>,
    /// Correlates events across the runtime rebinding a pane can undergo, and
    /// identifies which pane emitted an event recorded under the unbound id.
    pub pane_id: Option<String>,
    pub input_path: Option<TerminalUiInputPath>,
    /// Free-form provenance for flow, mount and skipped-output events.
    pub reason: Option<String>,
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
    Mount,
    OutputSkipped,
}

/// Which branch the frontend took for a keystroke. Only `write` reaches the
/// PTY; the other two are the silent-discard paths.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TerminalUiInputPath {
    Write,
    Buffered,
    Dropped,
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
        Self::new_with_detail(log_dir, detailed_diagnostics_enabled())
    }

    #[cfg(test)]
    pub(crate) fn new_detailed(log_dir: PathBuf) -> Self {
        Self::new_with_detail(log_dir, true)
    }

    fn new_with_detail(log_dir: PathBuf, detailed: bool) -> Self {
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
            detailed,
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
        if !self.detailed
            || data.is_empty()
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
        if !self.detailed {
            return;
        }
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
        if !self.detailed {
            return;
        }
        self.append_json(json!({
            "ts": Utc::now().to_rfc3339(),
            "event": "terminal_ui",
            "runtimeId": runtime_id,
            "details": event,
        }));
    }

    /// Deliberately minimal: a panic hook must not acquire a lock, block, or
    /// panic on its own account. The record goes through the same queue as every
    /// other event, so a full queue drops it rather than delaying the panic.
    fn record_panic(&self, panic_info: &std::panic::PanicHookInfo<'_>) {
        let location = panic_info.location();
        let thread = std::thread::current();
        self.append_json(json!({
            "ts": Utc::now().to_rfc3339(),
            "event": "rust_panic",
            "threadName": thread.name().unwrap_or("unnamed"),
            "message": panic_message(panic_info),
            // The file and line identify the panicking statement; threadName
            // ties it back to a runtime when it is a named PTY reader.
            "file": location.map(|value| value.file()),
            "line": location.map(|value| value.line()),
            "column": location.map(|value| value.column()),
        }));
    }

    pub fn record_runtime_event(
        &self,
        runtime_id: &str,
        event: &'static str,
        details: serde_json::Value,
    ) {
        if !self.detailed && !is_critical_runtime_event(event, &details) {
            return;
        }
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
                flush_log_file(&mut state);
                let _ = sender.send(());
            }
            Err(RecvTimeoutError::Timeout) => {
                record_dropped_events(&mut state, &timeline_sequence, &dropped_events);
                flush_log_file(&mut state);
            }
            Err(RecvTimeoutError::Disconnected) => {
                record_dropped_events(&mut state, &timeline_sequence, &dropped_events);
                flush_log_file(&mut state);
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
    if file.write_all(&line).is_ok() {
        state.bytes_written = state.bytes_written.saturating_add(line.len() as u64);
    } else {
        state.file.take();
        report_file_error(state);
    }
}

fn flush_log_file(state: &mut DiagnosticFileState) {
    let Some(file) = state.file.as_mut() else {
        return;
    };
    if file.flush().is_err() {
        state.file.take();
        report_file_error(state);
    }
}

fn detailed_diagnostics_enabled() -> bool {
    std::env::var(DETAILED_DIAGNOSTICS_ENV).is_ok_and(|value| {
        !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "off"
        )
    })
}

fn is_critical_runtime_event(event: &str, details: &serde_json::Value) -> bool {
    matches!(
        event,
        "output_lock_poisoned"
            | "screen_recovered"
            | "pty_reader_abandoned"
            | "pty_reader_error"
            | "pty_reader_spawn_failed"
            | "pty_reader_drain_timeout"
    ) || matches!(
        event,
        "terminal_input_completed" | "pty_write_completed" | "taskkill_completed"
    ) && details.get("status").and_then(serde_json::Value::as_str) == Some("error")
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

/// A payload is a `&'static str` for `panic!` and a `String` for a formatted
/// `panic!`; anything else is reported as such instead of being dropped. The
/// message is truncated because a payload can carry an unbounded `Debug` dump.
fn panic_message(panic_info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = panic_info.payload();
    let message = if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "<non-string panic payload>".to_owned()
    };
    message.chars().take(MAX_PANIC_MESSAGE_CHARS).collect()
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
        let diagnostics = TerminalInputDiagnostics::new_with_detail(directory.clone(), true);
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
        let diagnostics = TerminalInputDiagnostics::new_with_detail(directory.clone(), true);
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
                connecting: false,
                visible: true,
                document_visible: true,
                focused: true,
                focus_owner: Some("own-helper-textarea[1/1]".into()),
                alternate_buffer: true,
                pending_output: 0,
                output_cursor: 42,
                viewport_y: 3,
                base_y: 3,
                key_category: Some(TerminalUiKeyCategory::Printable),
                client_input_id: None,
                pane_id: Some("pane-1".into()),
                input_path: None,
                reason: None,
            },
        );
        diagnostics.flush();

        let contents = fs::read_to_string(directory.join(LOG_FILE_NAME)).expect("log file");
        assert!(contents.contains("\"event\":\"pty_write_completed\""));
        assert!(contents.contains("\"kind\":\"keydown\""));
        assert!(contents.contains("\"keyCategory\":\"printable\""));
        assert!(contents.contains("\"paneId\":\"pane-1\""));
        assert!(contents.contains("\"focusOwner\":\"own-helper-textarea[1/1]\""));
        assert!(!contents.contains("terminal text"));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn optional_diagnostic_fields_may_be_omitted() {
        // The frontend drops undefined fields when serializing, so every
        // optional field must deserialize from an absent key.
        let event = serde_json::from_value::<TerminalUiDiagnosticEvent>(json!({
            "kind": "input",
            "connected": false,
            "connecting": false,
            "visible": true,
            "documentVisible": true,
            "focused": false,
            "alternateBuffer": false,
            "pendingOutput": 0,
            "outputCursor": 0,
            "viewportY": 0,
            "baseY": 0
        }))
        .expect("event without optional fields");

        assert!(event.pane_id.is_none());
        assert!(event.focus_owner.is_none());
        assert!(event.input_path.is_none());
        assert!(event.reason.is_none());
        assert!(event.key_category.is_none());
    }

    #[test]
    fn records_the_silent_discard_input_paths() {
        let directory = test_log_dir();
        let diagnostics = TerminalInputDiagnostics::new_with_detail(directory.clone(), true);
        for path in [TerminalUiInputPath::Buffered, TerminalUiInputPath::Dropped] {
            diagnostics.record_ui_event(
                "unbound",
                TerminalUiDiagnosticEvent {
                    kind: TerminalUiDiagnosticEventKind::Input,
                    connected: false,
                    connecting: false,
                    visible: true,
                    document_visible: true,
                    focused: true,
                    focus_owner: Some("other-helper-textarea[1/2]".into()),
                    alternate_buffer: true,
                    pending_output: 0,
                    output_cursor: 0,
                    viewport_y: 0,
                    base_y: 0,
                    key_category: None,
                    client_input_id: None,
                    pane_id: Some("pane-1".into()),
                    input_path: Some(path),
                    reason: Some("no-runtime-id".into()),
                },
            );
        }
        diagnostics.flush();

        let contents = fs::read_to_string(directory.join(LOG_FILE_NAME)).expect("log file");
        assert!(contents.contains("\"inputPath\":\"buffered\""));
        assert!(contents.contains("\"inputPath\":\"dropped\""));
        assert!(contents.contains("\"runtimeId\":\"unbound\""));
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
    fn records_a_panic_with_its_thread_and_location() {
        let directory = test_log_dir();
        let diagnostics = std::sync::Arc::new(TerminalInputDiagnostics::new(directory.clone()));
        install_panic_hook(diagnostics.clone());

        // The panic is expected, so its payload is discarded; what matters is
        // that the hook wrote the location the panic would otherwise lose.
        let panicked = std::thread::Builder::new()
            .name("pty-reader-test".into())
            .spawn(|| panic!("local output lock"))
            .expect("panicking thread")
            .join();
        diagnostics.flush();
        // Restore the harness hook so an unrelated failure elsewhere in this
        // binary is not reported through this test's temporary log.
        let _ = std::panic::take_hook();

        assert!(panicked.is_err());
        let contents = fs::read_to_string(directory.join(LOG_FILE_NAME)).expect("log file");
        assert!(contents.contains("\"event\":\"rust_panic\""));
        assert!(contents.contains("\"threadName\":\"pty-reader-test\""));
        assert!(contents.contains("\"message\":\"local output lock\""));
        assert!(contents.contains("\"line\":"));
        assert!(contents.contains("terminal_input_diagnostics.rs"));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn rotates_logs_without_exceeding_the_retention_limit() {
        let directory = test_log_dir();
        let diagnostics = TerminalInputDiagnostics::new_with_detail(directory.clone(), true);
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
        let diagnostics = TerminalInputDiagnostics::new_with_detail(directory.clone(), true);

        diagnostics.record_runtime_event("runtime-1", "rotation_test", json!({}));
        diagnostics.flush();

        assert_eq!(fs::metadata(&log_path).unwrap().len(), MAX_LOG_BYTES);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn default_mode_suppresses_verbose_events_but_keeps_runtime_failures() {
        let directory = test_log_dir();
        let diagnostics = TerminalInputDiagnostics::new_with_detail(directory.clone(), false);
        diagnostics.record_runtime_event("runtime-1", "pty_write_completed", json!({}));
        diagnostics.record_runtime_event(
            "runtime-1",
            "terminal_input_completed",
            json!({ "status": "error" }),
        );
        diagnostics.record_ui_event(
            "runtime-1",
            TerminalUiDiagnosticEvent {
                kind: TerminalUiDiagnosticEventKind::Heartbeat,
                connected: true,
                connecting: false,
                visible: true,
                document_visible: true,
                focused: true,
                focus_owner: None,
                alternate_buffer: false,
                pending_output: 0,
                output_cursor: 0,
                viewport_y: 0,
                base_y: 0,
                key_category: None,
                client_input_id: None,
                pane_id: None,
                input_path: None,
                reason: None,
            },
        );
        diagnostics.record_runtime_event("runtime-1", "pty_reader_error", json!({}));
        diagnostics.flush();

        let contents = fs::read_to_string(directory.join(LOG_FILE_NAME)).expect("log file");
        assert!(contents.contains("\"event\":\"pty_reader_error\""));
        assert!(contents.contains("\"event\":\"terminal_input_completed\""));
        assert!(!contents.contains("pty_write_completed"));
        assert!(!contents.contains("terminal_ui"));
        let _ = fs::remove_dir_all(directory);
    }
}
