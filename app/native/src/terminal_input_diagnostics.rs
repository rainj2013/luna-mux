use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use chrono::Utc;
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const LOG_FILE_NAME: &str = "terminal-input-diagnostics.log";
const MAX_LOG_BYTES: u64 = 1024 * 1024;
const MAX_ROTATED_FILES: u8 = 3;
const DUPLICATE_WINDOW: Duration = Duration::from_millis(250);

#[derive(Clone)]
pub struct TerminalInputDiagnostics {
    state: Arc<Mutex<DiagnosticState>>,
}

struct DiagnosticState {
    log_path: PathBuf,
    file: Option<File>,
    bytes_written: u64,
    next_sequence: u64,
    fingerprint_salt: [u8; 16],
    recent_inputs: HashMap<(&'static str, String), RecentInput>,
    file_error_reported: bool,
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

impl TerminalInputDiagnostics {
    pub fn new(log_dir: PathBuf) -> Self {
        let log_path = log_dir.join(LOG_FILE_NAME);
        let _ = fs::create_dir_all(&log_dir);
        let bytes_written = fs::metadata(&log_path)
            .map(|metadata| metadata.len())
            .unwrap_or_default();
        let file = (bytes_written > 0)
            .then(|| OpenOptions::new().append(true).open(&log_path).ok())
            .flatten();
        let bytes_written = file
            .as_ref()
            .and_then(|file| file.metadata().ok())
            .map(|metadata| metadata.len())
            .unwrap_or_default();

        Self {
            state: Arc::new(Mutex::new(DiagnosticState {
                log_path,
                file,
                bytes_written,
                next_sequence: 1,
                fingerprint_salt: *Uuid::new_v4().as_bytes(),
                recent_inputs: HashMap::new(),
                file_error_reported: false,
            })),
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
        let mut state = self.state.lock().ok()?;
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

    fn append_json(&self, value: serde_json::Value) {
        let mut line = match serde_json::to_vec(&value) {
            Ok(line) => line,
            Err(_) => return,
        };
        line.push(b'\n');

        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.file.is_none()
            || state.bytes_written.saturating_add(line.len() as u64) > MAX_LOG_BYTES
        {
            rotate_log(&mut state);
        }
        let Some(file) = state.file.as_mut() else {
            report_file_error(&mut state);
            return;
        };
        if file.write_all(&line).and_then(|_| file.flush()).is_ok() {
            state.bytes_written = state.bytes_written.saturating_add(line.len() as u64);
        } else {
            report_file_error(&mut state);
        }
    }
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

fn rotate_log(state: &mut DiagnosticState) {
    state.file.take();
    for index in (1..=MAX_ROTATED_FILES).rev() {
        let destination = rotated_path(&state.log_path, index);
        if index == MAX_ROTATED_FILES {
            let _ = fs::remove_file(&destination);
        }
        let source = if index == 1 {
            state.log_path.clone()
        } else {
            rotated_path(&state.log_path, index - 1)
        };
        let _ = fs::rename(source, destination);
    }
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
}

fn rotated_path(path: &Path, index: u8) -> PathBuf {
    PathBuf::from(format!("{}.{index}", path.display()))
}

fn report_file_error(state: &mut DiagnosticState) {
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
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_log_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "luna-mux-terminal-input-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
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
}
