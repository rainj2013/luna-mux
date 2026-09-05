//! Transport-neutral interaction observations. A match is not a command exit status.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AsyncMutex;

use crate::{
    control_contract::{ControlError, ControlErrorCode, ControlResult},
    terminal_backend::TerminalBackend,
    terminal_cli::{CliObservation, PromptWait},
    terminal_runtime_contract::{TERMINAL_SCREEN_MAX_BYTES, TerminalScreenModes},
    terminal_runtime_contract::{TerminalRuntimeOutputReadResult, TerminalRuntimeStatus},
};

const MAX_RECORDS: usize = 1024;
const MAX_BYTES: usize = 1024 * 1024;
const POLL: Duration = Duration::from_millis(50);

fn error(code: ControlErrorCode, message: &str) -> ControlError {
    ControlError {
        code,
        message: message.into(),
        retryable: false,
        details: None,
    }
}
fn invalid(message: &str) -> ControlError {
    error(ControlErrorCode::InvalidArguments, message)
}
fn unavailable(_: String) -> ControlError {
    // Backend errors may include remote diagnostics; do not copy them into execution records.
    error(
        ControlErrorCode::Unavailable,
        "Terminal backend unavailable",
    )
}
fn default_timeout() -> u64 {
    1000
}
fn default_bytes() -> usize {
    65536
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum InteractionInput {
    Text {
        text: String,
        #[serde(default)]
        submit: bool,
    },
    Key {
        key: TerminalKey,
    },
    AutoPaste {
        text: String,
    },
    Paste {
        text: String,
        #[serde(default, rename = "bracketedPaste")]
        bracketed_paste: bool,
    },
}

#[derive(Clone, Deserialize, Serialize)]
pub enum TerminalKey {
    Enter,
    Tab,
    Escape,
    Backspace,
    CtrlD,
    CtrlL,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
}

impl InteractionInput {
    fn needs_modes(&self) -> bool {
        matches!(
            self,
            Self::AutoPaste { .. }
                | Self::Key {
                    key: TerminalKey::ArrowUp
                        | TerminalKey::ArrowDown
                        | TerminalKey::ArrowLeft
                        | TerminalKey::ArrowRight
                        | TerminalKey::Home
                        | TerminalKey::End
                }
        )
    }

    fn encode(&self, modes: Option<&TerminalScreenModes>) -> ControlResult<String> {
        if self.needs_modes() && modes.is_none() {
            return Err(invalid(
                "This input requires a reliable terminal screen mode snapshot",
            ));
        }
        let application_cursor = modes.is_some_and(|m| m.application_cursor);
        let value = match self {
            Self::AutoPaste { text } => {
                let bracketed = modes.is_some_and(|m| m.bracketed_paste);
                if !bracketed && text.contains(['\r', '\n']) {
                    return Err(invalid(
                        "Automatic multiline paste requires bracketed paste mode; use explicit paste only if line execution is intended",
                    ));
                }
                return Self::Paste {
                    text: text.clone(),
                    bracketed_paste: bracketed,
                }
                .encode(None);
            }
            Self::Text { text, submit } => {
                if text.chars().any(char::is_control) {
                    return Err(invalid(
                        "text must be a single line without control characters; use paste or the raw write operation",
                    ));
                }
                format!("{text}{}", if *submit { "\r" } else { "" })
            }
            Self::Key { key } => match key {
                TerminalKey::Enter => "\r",
                TerminalKey::Tab => "\t",
                TerminalKey::Escape => "\x1b",
                TerminalKey::Backspace => "\x7f",
                TerminalKey::CtrlD => "\x04",
                TerminalKey::CtrlL => "\x0c",
                TerminalKey::ArrowUp => {
                    if application_cursor {
                        "\x1bOA"
                    } else {
                        "\x1b[A"
                    }
                }
                TerminalKey::ArrowDown => {
                    if application_cursor {
                        "\x1bOB"
                    } else {
                        "\x1b[B"
                    }
                }
                TerminalKey::ArrowRight => {
                    if application_cursor {
                        "\x1bOC"
                    } else {
                        "\x1b[C"
                    }
                }
                TerminalKey::ArrowLeft => {
                    if application_cursor {
                        "\x1bOD"
                    } else {
                        "\x1b[D"
                    }
                }
                TerminalKey::Home => {
                    if application_cursor {
                        "\x1bOH"
                    } else {
                        "\x1b[H"
                    }
                }
                TerminalKey::End => {
                    if application_cursor {
                        "\x1bOF"
                    } else {
                        "\x1b[F"
                    }
                }
                TerminalKey::PageUp => "\x1b[5~",
                TerminalKey::PageDown => "\x1b[6~",
                TerminalKey::Insert => "\x1b[2~",
                TerminalKey::Delete => "\x1b[3~",
            }
            .into(),
            Self::Paste {
                text,
                bracketed_paste,
            } => {
                if text
                    .chars()
                    .any(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t'))
                {
                    return Err(invalid("paste contains unsupported control characters"));
                }
                let normalized = text.replace("\r\n", "\n").replace(['\r', '\n'], "\r");
                if *bracketed_paste {
                    format!("\x1b[200~{normalized}\x1b[201~")
                } else {
                    normalized
                }
            }
        };
        if value.is_empty() || value.len() > 65536 {
            return Err(invalid("encoded input must contain 1 to 65536 bytes"));
        }
        Ok(value)
    }
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WaitCondition {
    pub text: Option<String>,
    #[serde(default)]
    pub match_mode: MatchMode,
    pub idle_ms: Option<u64>,
    pub prompt: Option<PromptWait>,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MatchMode {
    #[default]
    Contains,
    Suffix,
}

impl WaitCondition {
    fn validate(&self) -> ControlResult<()> {
        if let Some(prompt) = &self.prompt {
            prompt.matcher().map_err(|e| invalid(&e))?;
        }
        if self
            .text
            .as_ref()
            .is_some_and(|s| s.is_empty() || s.len() > 4096)
        {
            return Err(invalid("wait.text must contain 1 to 4096 bytes"));
        }
        if self.idle_ms.is_some_and(|v| !(100..=30000).contains(&v)) {
            return Err(invalid("idleMs must be between 100 and 30000"));
        }
        Ok(())
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InteractArguments {
    pub input: InteractionInput,
    #[serde(default)]
    pub wait: WaitCondition,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default = "default_bytes")]
    pub max_bytes: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputWaitArguments {
    pub from_cursor: u64,
    #[serde(default)]
    pub wait: WaitCondition,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default = "default_bytes")]
    pub max_bytes: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionWaitArguments {
    pub execution_id: String,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default = "default_bytes")]
    pub max_bytes: usize,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionReadArguments {
    pub execution_id: String,
    pub from_cursor: Option<u64>,
    #[serde(default = "default_bytes")]
    pub max_bytes: usize,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionCancelArguments {
    pub execution_id: String,
}

fn validate_limits(timeout_ms: u64, max_bytes: usize) -> ControlResult<()> {
    if timeout_ms > 30000 || !(4..=MAX_BYTES).contains(&max_bytes) {
        return Err(invalid(
            "timeoutMs must be 0..30000 and maxBytes must be 4..1048576",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WaitReason {
    Matched,
    Prompt,
    Idle,
    Timeout,
    OutputLimit,
    OutputGap,
    RuntimeExited,
    Cancelled,
    InputChanged,
    WriteUncertain,
    Unavailable,
}
#[derive(Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
enum InputState {
    Sending,
    Accepted,
    Uncertain,
    NotSent,
}

struct Progress {
    cursor: u64,
    tail: String,
    last_output: Instant,
}
impl Progress {
    fn new(cursor: u64) -> Self {
        Self {
            cursor,
            tail: String::new(),
            last_output: Instant::now(),
        }
    }
    fn observe(
        &mut self,
        output: &TerminalRuntimeOutputReadResult,
        condition: &WaitCondition,
    ) -> bool {
        self.cursor = output.next_cursor;
        if output.truncated {
            self.tail.clear();
        }
        if output.data.is_empty() {
            return false;
        }
        self.last_output = Instant::now();
        let Some(text) = &condition.text else {
            return false;
        };
        self.tail.push_str(&output.data);
        let matched = match condition.match_mode {
            MatchMode::Contains => self.tail.contains(text),
            MatchMode::Suffix => self.tail.ends_with(text),
        };
        let mut offset = self.tail.len().saturating_sub(text.len());
        while !self.tail.is_char_boundary(offset) {
            offset += 1;
        }
        self.tail.drain(..offset);
        matched
    }
}

struct ExecutionState {
    input: InputState,
    finish: Option<WaitReason>,
    progress: Progress,
}
struct Execution {
    id: String,
    owner: String,
    runtime: String,
    key_hash: [u8; 32],
    signature: [u8; 32],
    start_cursor: u64,
    created: Instant,
    condition: WaitCondition,
    state: Mutex<ExecutionState>,
    waiter: AsyncMutex<()>,
}
#[derive(Default)]
struct Registry {
    executions: HashMap<String, Arc<Execution>>,
    gates: HashMap<String, Weak<AsyncMutex<()>>>,
}

pub struct TerminalInteractionManager {
    backend: Arc<dyn TerminalBackend>,
    registry: Mutex<Registry>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub reason: WaitReason,
    pub output: TerminalRuntimeOutputReadResult,
    pub elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<CliObservation>,
}

impl TerminalInteractionManager {
    pub fn new(backend: Arc<dyn TerminalBackend>) -> Arc<Self> {
        Arc::new(Self {
            backend,
            registry: Mutex::new(Registry::default()),
        })
    }

    fn gate(&self, runtime: &str) -> Arc<AsyncMutex<()>> {
        let mut registry = self.registry.lock().expect("interaction registry");
        registry.gates.retain(|_, gate| gate.strong_count() > 0);
        if let Some(gate) = registry.gates.get(runtime).and_then(Weak::upgrade) {
            return gate;
        }
        let gate = Arc::new(AsyncMutex::new(()));
        registry.gates.insert(runtime.into(), Arc::downgrade(&gate));
        gate
    }

    fn running(&self, runtime: &str) -> ControlResult<bool> {
        let runtimes = self.backend.list().map_err(unavailable)?;
        Ok(runtimes.iter().any(|r| {
            r.runtime_id == runtime
                && matches!(
                    r.status,
                    TerminalRuntimeStatus::Running
                        | TerminalRuntimeStatus::Starting
                        | TerminalRuntimeStatus::Connecting
                )
        }))
    }

    fn get(&self, owner: &str, runtime: &str, id: &str) -> ControlResult<Arc<Execution>> {
        self.registry
            .lock()
            .expect("interaction registry")
            .executions
            .get(id)
            .filter(|e| e.owner == owner && e.runtime == runtime)
            .cloned()
            .ok_or_else(|| {
                error(
                    ControlErrorCode::NotFound,
                    "Execution not found for this caller and Runtime",
                )
            })
    }

    fn active(&self, runtime: &str) -> Vec<Arc<Execution>> {
        self.registry
            .lock()
            .expect("interaction registry")
            .executions
            .values()
            .filter(|e| {
                e.runtime == runtime && e.state.lock().expect("execution state").finish.is_none()
            })
            .cloned()
            .collect()
    }

    /// All managed input uses this gate. Human input takes over; automation fails closed.
    pub async fn write(&self, runtime: &str, data: &str, human: bool) -> ControlResult<()> {
        let gate = self.gate(runtime);
        let guard = if human {
            gate.lock_owned().await
        } else {
            gate.try_lock_owned()
                .map_err(|_| error(ControlErrorCode::Conflict, "Runtime input is busy"))?
        };
        let active = self.active(runtime);
        if !human && !active.is_empty() {
            return Err(error(
                ControlErrorCode::Conflict,
                "Runtime has an active interaction; wait or cancel it before writing",
            ));
        }
        for execution in active {
            execution.state.lock().expect("execution state").finish =
                Some(WaitReason::InputChanged);
        }
        let backend = self.backend.clone();
        let runtime = runtime.to_owned();
        let data = data.to_owned();
        // A native blocking write can outlive its caller future. Keep the input
        // gate with the work, including legacy and desktop input, until it ends.
        tokio::spawn(async move {
            let _guard = guard;
            backend.write(&runtime, &data).await.map_err(unavailable)
        })
        .await
        .map_err(|_| unavailable(String::new()))?
    }

    pub async fn interrupt(&self, runtime: &str) -> ControlResult<()> {
        let guard = self.gate(runtime).lock_owned().await;
        for execution in self.active(runtime) {
            execution.state.lock().expect("execution state").finish =
                Some(WaitReason::InputChanged);
        }
        let backend = self.backend.clone();
        let runtime = runtime.to_owned();
        tokio::spawn(async move {
            let _guard = guard;
            backend.interrupt(&runtime).await.map_err(unavailable)
        })
        .await
        .map_err(|_| unavailable(String::new()))?
    }

    pub async fn start(
        self: &Arc<Self>,
        owner: &str,
        runtime: &str,
        key: &str,
        args: &InteractArguments,
    ) -> ControlResult<String> {
        validate_limits(args.timeout_ms, args.max_bytes)?;
        args.wait.validate()?;
        if key.is_empty() || key.len() > 256 {
            return Err(invalid(
                "interact requires an idempotencyKey of 1..256 bytes",
            ));
        }
        let signature: [u8; 32] =
            Sha256::digest(serde_json::to_vec(args).expect("serializable input")).into();
        let key_hash: [u8; 32] = Sha256::digest(key.as_bytes()).into();
        // Recovery must not wait for an outstanding transport write to finish.
        if let Some(existing) = self
            .registry
            .lock()
            .expect("interaction registry")
            .executions
            .values()
            .find(|e| e.owner == owner && e.key_hash == key_hash)
        {
            return if existing.signature == signature && existing.runtime == runtime {
                Ok(existing.id.clone())
            } else {
                Err(error(
                    ControlErrorCode::Conflict,
                    "Idempotency key belongs to a different interaction",
                ))
            };
        }
        // Reservation and detached spawn have no await between them. Dropping an MCP
        // request after reservation cannot leave a retry free to send the input again.
        let guard = self
            .gate(runtime)
            .try_lock_owned()
            .map_err(|_| error(ControlErrorCode::Conflict, "Runtime input is busy"))?;
        {
            let registry = self.registry.lock().expect("interaction registry");
            if let Some(existing) = registry
                .executions
                .values()
                .find(|e| e.owner == owner && e.key_hash == key_hash)
            {
                return if existing.signature == signature && existing.runtime == runtime {
                    Ok(existing.id.clone())
                } else {
                    Err(error(
                        ControlErrorCode::Conflict,
                        "Idempotency key belongs to a different interaction",
                    ))
                };
            }
        }
        if !self.running(runtime)? {
            return Err(error(
                ControlErrorCode::Unavailable,
                "Runtime is not running",
            ));
        }
        if !self.active(runtime).is_empty() {
            return Err(error(
                ControlErrorCode::Conflict,
                "Runtime has an active interaction",
            ));
        }
        let screen = if args.input.needs_modes() || args.wait.prompt.is_some() {
            let snapshot = self
                .backend
                .screen_snapshot(runtime, TERMINAL_SCREEN_MAX_BYTES)
                .map_err(unavailable)?;
            if snapshot.truncated || snapshot.size_limited {
                return Err(error(
                    ControlErrorCode::Unavailable,
                    "Terminal screen is incomplete",
                ));
            }
            Some(snapshot)
        } else {
            None
        };
        let data = args.input.encode(screen.as_ref().map(|s| &s.modes))?;
        let start_cursor = self
            .backend
            .read_output(runtime, u64::MAX, 4)
            .map_err(unavailable)?
            .next_cursor;
        let execution = Arc::new(Execution {
            id: uuid::Uuid::new_v4().to_string(),
            owner: owner.into(),
            runtime: runtime.into(),
            key_hash,
            signature,
            start_cursor,
            created: Instant::now(),
            condition: args.wait.clone(),
            state: Mutex::new(ExecutionState {
                input: InputState::Sending,
                finish: None,
                progress: Progress::new(start_cursor),
            }),
            waiter: AsyncMutex::new(()),
        });
        {
            let mut registry = self.registry.lock().expect("interaction registry");
            // Check globally again: callers may race the same key on different Runtimes.
            if registry
                .executions
                .values()
                .any(|e| e.owner == owner && e.key_hash == key_hash)
            {
                return Err(error(
                    ControlErrorCode::Conflict,
                    "Idempotency key is already reserved",
                ));
            }
            if registry.executions.len() >= MAX_RECORDS {
                return Err(error(
                    ControlErrorCode::Unavailable,
                    "Interaction record limit reached for this application run",
                ));
            }
            registry
                .executions
                .insert(execution.id.clone(), execution.clone());
        }
        let backend = self.backend.clone();
        let id = execution.id.clone();
        tokio::spawn(async move {
            let _guard = guard;
            if execution
                .state
                .lock()
                .expect("execution state")
                .finish
                .is_some()
            {
                execution.state.lock().expect("execution state").input = InputState::NotSent;
                return;
            }
            let result = backend.write(&execution.runtime, &data).await;
            let mut state = execution.state.lock().expect("execution state");
            state.input = if result.is_ok() {
                InputState::Accepted
            } else {
                InputState::Uncertain
            };
            state.progress.last_output = Instant::now();
            if result.is_err() && state.finish.is_none() {
                state.finish = Some(WaitReason::WriteUncertain);
            }
        });
        Ok(id)
    }

    pub fn cancel(&self, owner: &str, runtime: &str, id: &str) -> ControlResult<serde_json::Value> {
        let execution = self.get(owner, runtime, id)?;
        let mut state = execution.state.lock().expect("execution state");
        let reason = *state.finish.get_or_insert(WaitReason::Cancelled);
        Ok(
            serde_json::json!({ "executionId": id, "reason": reason, "inputState": state.input, "terminalInterrupted": false }),
        )
    }

    pub fn read(
        &self,
        owner: &str,
        runtime: &str,
        args: ExecutionReadArguments,
    ) -> ControlResult<serde_json::Value> {
        validate_limits(0, args.max_bytes)?;
        let e = self.get(owner, runtime, &args.execution_id)?;
        let cursor = args.from_cursor.unwrap_or(e.start_cursor);
        if cursor < e.start_cursor {
            return Err(invalid("fromCursor precedes the interaction"));
        }
        let output = self
            .backend
            .read_output(runtime, cursor, args.max_bytes)
            .map_err(unavailable)?;
        let state = e.state.lock().expect("execution state");
        Ok(
            serde_json::json!({ "executionId": e.id, "inputState": state.input, "reason": state.finish,
            "startCursor": e.start_cursor, "waitCursor": state.progress.cursor,
            "elapsedMs": e.created.elapsed().as_millis() as u64, "output": output }),
        )
    }

    pub async fn wait(
        &self,
        owner: &str,
        runtime: &str,
        args: ExecutionWaitArguments,
    ) -> ControlResult<serde_json::Value> {
        validate_limits(args.timeout_ms, args.max_bytes)?;
        let e = self.get(owner, runtime, &args.execution_id)?;
        let _waiter = e.waiter.try_lock().map_err(|_| ControlError {
            code: ControlErrorCode::Conflict,
            message: "Execution already has a waiter; use execution.read".into(),
            retryable: true,
            details: Some(serde_json::json!({ "executionId": e.id })),
        })?;
        let cursor = e.state.lock().expect("execution state").progress.cursor;
        let observation = self
            .observe(
                runtime,
                cursor,
                &e.condition,
                args.timeout_ms,
                args.max_bytes,
                Some(&e),
            )
            .await?;
        let state = e.state.lock().expect("execution state");
        Ok(
            serde_json::json!({ "executionId": e.id, "inputState": state.input, "startCursor": e.start_cursor,
            "elapsedMs": e.created.elapsed().as_millis() as u64, "observation": observation }),
        )
    }

    pub async fn output_wait(
        &self,
        runtime: &str,
        args: OutputWaitArguments,
    ) -> ControlResult<Observation> {
        validate_limits(args.timeout_ms, args.max_bytes)?;
        args.wait.validate()?;
        self.observe(
            runtime,
            args.from_cursor,
            &args.wait,
            args.timeout_ms,
            args.max_bytes,
            None,
        )
        .await
    }

    async fn observe(
        &self,
        runtime: &str,
        cursor: u64,
        condition: &WaitCondition,
        timeout_ms: u64,
        max_bytes: usize,
        execution: Option<&Arc<Execution>>,
    ) -> ControlResult<Observation> {
        let prompt_matcher = condition
            .prompt
            .as_ref()
            .map(PromptWait::matcher)
            .transpose()
            .map_err(|e| invalid(&e))?;
        let mut prompt_observation = None;
        let started = Instant::now();
        let deadline = started + Duration::from_millis(timeout_ms);
        let mut progress = Progress::new(cursor);
        let mut result = TerminalRuntimeOutputReadResult {
            runtime_id: runtime.into(),
            requested_cursor: cursor,
            earliest_cursor: cursor,
            next_cursor: cursor,
            truncated: false,
            data: String::new(),
        };
        let mut reason = loop {
            if let Some(e) = execution {
                let (finish, input) = {
                    let state = e.state.lock().expect("execution state");
                    (state.finish, state.input)
                };
                if let Some(reason) = finish {
                    break reason;
                }
                if input == InputState::Sending {
                    if Instant::now() >= deadline {
                        break WaitReason::Timeout;
                    }
                    tokio::time::sleep(
                        POLL.min(deadline.saturating_duration_since(Instant::now())),
                    )
                    .await;
                    continue;
                }
            }
            let chunk = match self.backend.read_output(
                runtime,
                result.next_cursor,
                max_bytes - result.data.len(),
            ) {
                Ok(chunk) => chunk,
                Err(_) => {
                    break if self.running(runtime)? {
                        WaitReason::Unavailable
                    } else {
                        WaitReason::RuntimeExited
                    };
                }
            };
            if chunk.next_cursor < result.next_cursor {
                return Err(invalid("fromCursor is beyond current output"));
            }
            result.earliest_cursor = chunk.earliest_cursor;
            result.next_cursor = chunk.next_cursor;
            result.truncated |= chunk.truncated;
            result.data.push_str(&chunk.data);
            let had_output = !chunk.data.is_empty();
            let (matched, idle) = if let Some(e) = execution {
                let mut state = e.state.lock().expect("execution state");
                if let Some(reason) = state.finish {
                    break reason;
                }
                let matched = state.progress.observe(&chunk, condition);
                (matched, state.progress.last_output.elapsed())
            } else {
                let matched = progress.observe(&chunk, condition);
                (matched, progress.last_output.elapsed())
            };
            if chunk.truncated {
                break WaitReason::OutputGap;
            }
            // A bounded chunk ending in a prompt is not a suffix match if more
            // output already exists in the Runtime ring.
            if matched
                && (matches!(condition.match_mode, MatchMode::Contains)
                    || self
                        .backend
                        .read_output(runtime, u64::MAX, 4)
                        .map_err(unavailable)?
                        .next_cursor
                        == chunk.next_cursor)
            {
                break WaitReason::Matched;
            }
            if let (Some(config), Some(matcher)) = (&condition.prompt, &prompt_matcher) {
                let screen = self
                    .backend
                    .screen_snapshot(runtime, TERMINAL_SCREEN_MAX_BYTES)
                    .map_err(unavailable)?;
                let baseline = execution.map(|e| e.start_cursor).unwrap_or(cursor);
                let observed = matcher.observe(&screen);
                let matches = screen.output_cursor == result.next_cursor
                    && screen.output_cursor > baseline
                    && screen.cursor_line_cursor > baseline
                    && config.states.contains(&observed.state);
                prompt_observation = Some(observed);
                if matches {
                    break WaitReason::Prompt;
                }
            }
            // Backend reads round up to four bytes to preserve UTF-8. Stop early so
            // that the next read can never exceed this response's byte budget.
            if max_bytes - result.data.len() < 4 {
                break WaitReason::OutputLimit;
            }
            if had_output {
                continue;
            }
            if !self.running(runtime)? {
                break WaitReason::RuntimeExited;
            }
            if condition
                .idle_ms
                .is_some_and(|ms| idle >= Duration::from_millis(ms))
            {
                break WaitReason::Idle;
            }
            if Instant::now() >= deadline {
                break WaitReason::Timeout;
            }
            tokio::time::sleep(POLL.min(deadline.saturating_duration_since(Instant::now()))).await;
        };
        if let Some(e) = execution {
            if !matches!(
                reason,
                WaitReason::Timeout | WaitReason::OutputLimit | WaitReason::OutputGap
            ) {
                reason = *e
                    .state
                    .lock()
                    .expect("execution state")
                    .finish
                    .get_or_insert(reason);
            }
        }
        Ok(Observation {
            reason,
            output: result,
            elapsed_ms: started.elapsed().as_millis() as u64,
            prompt: prompt_observation,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_runtime_contract::TerminalScreenSnapshot;
    use crate::{
        terminal_backend::{
            TerminalBackendResult, TerminalRuntimeEventSink, standard_terminal_capabilities,
        },
        terminal_output::OutputBuffer,
        terminal_runtime_contract::{
            TerminalRuntime, TerminalRuntimeCreateRequest, TerminalTarget,
        },
    };
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    struct Backend {
        output: Mutex<OutputBuffer>,
        writes: Mutex<Vec<String>>,
        response: Mutex<String>,
        running: AtomicBool,
        fail: AtomicBool,
        delay: AtomicU64,
    }
    impl Backend {
        fn new(capacity: usize) -> Arc<Self> {
            Arc::new(Self {
                output: Mutex::new(OutputBuffer::new(capacity)),
                writes: Mutex::new(vec![]),
                response: Mutex::new(String::new()),
                running: AtomicBool::new(true),
                fail: AtomicBool::new(false),
                delay: AtomicU64::new(0),
            })
        }
        fn push(&self, text: &str) {
            self.output.lock().unwrap().push("r", text.into());
        }
    }
    #[async_trait::async_trait]
    impl TerminalBackend for Backend {
        fn set_event_sink(&self, _: TerminalRuntimeEventSink) {}
        fn targets(&self) -> TerminalBackendResult<Vec<TerminalTarget>> {
            Ok(vec![])
        }
        fn list(&self) -> TerminalBackendResult<Vec<TerminalRuntime>> {
            Ok(vec![TerminalRuntime {
                runtime_id: "r".into(),
                target_id: "t".into(),
                title: "test".into(),
                status: if self.running.load(Ordering::SeqCst) {
                    TerminalRuntimeStatus::Running
                } else {
                    TerminalRuntimeStatus::Exited
                },
                capabilities: standard_terminal_capabilities(false),
                context: None,
                managed_agent: None,
                error: None,
            }])
        }
        async fn create(
            &self,
            _: TerminalRuntimeCreateRequest,
        ) -> TerminalBackendResult<TerminalRuntime> {
            Err("unused".into())
        }
        async fn write(&self, _: &str, data: &str) -> TerminalBackendResult<()> {
            self.writes.lock().unwrap().push(data.into());
            tokio::time::sleep(Duration::from_millis(self.delay.load(Ordering::SeqCst))).await;
            self.push(&self.response.lock().unwrap());
            if self.fail.load(Ordering::SeqCst) {
                Err("sensitive backend diagnostic".into())
            } else {
                Ok(())
            }
        }
        async fn resize(&self, _: &str, _: u32, _: u32) -> TerminalBackendResult<()> {
            Ok(())
        }
        fn set_output_paused(&self, _: &str, _: bool) -> TerminalBackendResult<()> {
            Ok(())
        }
        async fn interrupt(&self, _: &str) -> TerminalBackendResult<()> {
            self.writes.lock().unwrap().push("\x03".into());
            Ok(())
        }
        async fn close(&self, _: &str) -> TerminalBackendResult<()> {
            self.running.store(false, Ordering::SeqCst);
            Ok(())
        }
        fn screen_snapshot(
            &self,
            runtime: &str,
            max_bytes: usize,
        ) -> TerminalBackendResult<TerminalScreenSnapshot> {
            Ok(self
                .output
                .lock()
                .unwrap()
                .screen_snapshot(runtime, max_bytes))
        }
        fn read_output(
            &self,
            runtime: &str,
            cursor: u64,
            max_bytes: usize,
        ) -> TerminalBackendResult<TerminalRuntimeOutputReadResult> {
            self.output.lock().unwrap().read(runtime, cursor, max_bytes)
        }
    }

    fn args() -> InteractArguments {
        serde_json::from_value(
            serde_json::json!({ "input": { "type": "text", "text": "SELECT 1;", "submit": true },
            "wait": { "text": "mysql> ", "matchMode": "suffix" }, "timeoutMs": 500 }),
        )
        .unwrap()
    }
    async fn wait(
        manager: &TerminalInteractionManager,
        id: &str,
        timeout_ms: u64,
        max_bytes: usize,
    ) -> serde_json::Value {
        manager
            .wait(
                "owner",
                "r",
                ExecutionWaitArguments {
                    execution_id: id.into(),
                    timeout_ms,
                    max_bytes,
                },
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn fast_output_excludes_history_and_retry_writes_once() {
        let backend = Backend::new(MAX_BYTES);
        backend.push("old mysql> ");
        *backend.response.lock().unwrap() = "1\r\nmysql> ".into();
        let manager = TerminalInteractionManager::new(backend.clone());
        let id = manager.start("owner", "r", "key", &args()).await.unwrap();
        let result = wait(&manager, &id, 500, 65536).await;
        assert_eq!(result["observation"]["reason"], "matched");
        assert_eq!(result["observation"]["output"]["data"], "1\r\nmysql> ");
        assert_eq!(
            manager.start("owner", "r", "key", &args()).await.unwrap(),
            id
        );
        assert_eq!(*backend.writes.lock().unwrap(), vec!["SELECT 1;\r"]);
        let mut different = args();
        different.timeout_ms = 42;
        assert_eq!(
            manager
                .start("owner", "r", "key", &different)
                .await
                .unwrap_err()
                .code,
            ControlErrorCode::Conflict
        );
    }

    #[tokio::test]
    async fn timeout_and_output_limit_resume_cross_chunk_utf8_matching() {
        let backend = Backend::new(MAX_BYTES);
        let manager = TerminalInteractionManager::new(backend.clone());
        let mut input = args();
        input.wait.text = Some("中done".into());
        let id = manager.start("owner", "r", "key", &input).await.unwrap();
        assert_eq!(
            wait(&manager, &id, 80, 4).await["observation"]["reason"],
            "timeout"
        );
        backend.push("a中do");
        let part = wait(&manager, &id, 80, 4).await;
        assert_eq!(part["observation"]["reason"], "outputLimit");
        assert_eq!(part["observation"]["output"]["data"], "a中");
        backend.push("ne");
        assert_eq!(
            wait(&manager, &id, 100, 64).await["observation"]["reason"],
            "matched"
        );
        assert_eq!(backend.writes.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn cancellation_does_not_interrupt_and_user_input_takes_over() {
        let backend = Backend::new(MAX_BYTES);
        let manager = TerminalInteractionManager::new(backend.clone());
        let id = manager.start("owner", "r", "first", &args()).await.unwrap();
        wait(&manager, &id, 80, 64).await;
        assert_eq!(
            manager.write("r", "danger", false).await.unwrap_err().code,
            ControlErrorCode::Conflict
        );
        assert_eq!(
            manager
                .start("owner", "r", "second", &args())
                .await
                .unwrap_err()
                .code,
            ControlErrorCode::Conflict
        );
        manager.cancel("owner", "r", &id).unwrap();
        manager.cancel("owner", "r", &id).unwrap();
        assert_eq!(
            wait(&manager, &id, 100, 64).await["observation"]["reason"],
            "cancelled"
        );
        assert_eq!(backend.writes.lock().unwrap().len(), 1);
        let next = manager
            .start("owner", "r", "second", &args())
            .await
            .unwrap();
        manager.write("r", "human", true).await.unwrap();
        assert_eq!(
            wait(&manager, &next, 100, 64).await["observation"]["reason"],
            "inputChanged"
        );
        assert_eq!(
            *backend.writes.lock().unwrap(),
            vec!["SELECT 1;\r", "SELECT 1;\r", "human"]
        );
    }

    #[tokio::test]
    async fn dropped_waiter_and_parallel_retries_do_not_resend() {
        let backend = Backend::new(MAX_BYTES);
        backend.delay.store(100, Ordering::SeqCst);
        let manager = TerminalInteractionManager::new(backend.clone());
        let id = manager.start("owner", "r", "key", &args()).await.unwrap();
        let m = manager.clone();
        let execution_id = id.clone();
        let waiter = tokio::spawn(async move { wait(&m, &execution_id, 1000, 64).await });
        tokio::time::sleep(Duration::from_millis(10)).await;
        waiter.abort();
        let _ = waiter.await;
        let input = args();
        let (a, b) = tokio::join!(
            manager.start("owner", "r", "key", &input),
            manager.start("owner", "r", "key", &input)
        );
        assert_eq!(a.unwrap(), id);
        assert_eq!(b.unwrap(), id);
        backend.push("mysql> ");
        assert_eq!(
            wait(&manager, &id, 500, 64).await["observation"]["reason"],
            "matched"
        );
        assert_eq!(backend.writes.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn partial_write_failure_is_retained_and_not_exposed() {
        let backend = Backend::new(MAX_BYTES);
        backend.fail.store(true, Ordering::SeqCst);
        let manager = TerminalInteractionManager::new(backend.clone());
        let id = manager.start("owner", "r", "key", &args()).await.unwrap();
        let result = wait(&manager, &id, 500, 64).await;
        assert_eq!(result["inputState"], "uncertain");
        assert_eq!(result["observation"]["reason"], "writeUncertain");
        assert!(!result.to_string().contains("sensitive"));
        assert_eq!(
            manager.start("owner", "r", "key", &args()).await.unwrap(),
            id
        );
        assert_eq!(backend.writes.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn automation_does_not_queue_behind_an_inflight_write() {
        let backend = Backend::new(MAX_BYTES);
        backend.delay.store(200, Ordering::SeqCst);
        let manager = TerminalInteractionManager::new(backend.clone());
        let id = manager.start("owner", "r", "first", &args()).await.unwrap();
        let result = tokio::time::timeout(
            Duration::from_millis(30),
            manager.write("r", "stale", false),
        )
        .await
        .unwrap();
        assert_eq!(result.unwrap_err().code, ControlErrorCode::Conflict);
        let input = args();
        let result = tokio::time::timeout(
            Duration::from_millis(30),
            manager.start("owner", "r", "second", &input),
        )
        .await
        .unwrap();
        assert_eq!(result.unwrap_err().code, ControlErrorCode::Conflict);
        assert_eq!(
            manager.start("owner", "r", "first", &input).await.unwrap(),
            id
        );
        wait(&manager, &id, 300, 64).await;
        assert_eq!(backend.writes.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn cancelled_legacy_or_human_input_retains_gate_until_write_finishes() {
        for human in [false, true] {
            let backend = Backend::new(MAX_BYTES);
            backend.delay.store(200, Ordering::SeqCst);
            let manager = TerminalInteractionManager::new(backend.clone());
            let m = manager.clone();
            let writer = tokio::spawn(async move { m.write("r", "old", human).await });
            tokio::time::timeout(Duration::from_millis(100), async {
                while backend.writes.lock().unwrap().is_empty() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            writer.abort();
            let _ = writer.await;
            assert_eq!(
                manager
                    .start("owner", "r", "new", &args())
                    .await
                    .unwrap_err()
                    .code,
                ControlErrorCode::Conflict
            );
            tokio::time::sleep(Duration::from_millis(220)).await;
            let id = manager.start("owner", "r", "new", &args()).await.unwrap();
            wait(&manager, &id, 300, 64).await;
            assert_eq!(*backend.writes.lock().unwrap(), vec!["old", "SELECT 1;\r"]);
        }
    }

    #[tokio::test]
    async fn suffix_match_does_not_match_the_end_of_a_partial_read() {
        let backend = Backend::new(MAX_BYTES);
        backend.push("mysql> still-running");
        let manager = TerminalInteractionManager::new(backend);
        let result = manager
            .output_wait(
                "r",
                OutputWaitArguments {
                    from_cursor: 0,
                    wait: args().wait,
                    timeout_ms: 100,
                    max_bytes: 7,
                },
            )
            .await
            .unwrap();
        assert_eq!(result.reason, WaitReason::OutputLimit);
        assert_eq!(result.output.data, "mysql> ");
    }

    #[tokio::test]
    async fn capacity_rejects_new_records_without_evicting_retry_reservations() {
        let backend = Backend::new(MAX_BYTES);
        let manager = TerminalInteractionManager::new(backend.clone());
        let mut first = String::new();
        for index in 0..MAX_RECORDS {
            let id = manager
                .start("owner", "r", &index.to_string(), &args())
                .await
                .unwrap();
            if index == 0 {
                first = id.clone();
            }
            manager.cancel("owner", "r", &id).unwrap();
            tokio::task::yield_now().await;
        }
        assert_eq!(
            manager
                .start("owner", "r", "overflow", &args())
                .await
                .unwrap_err()
                .code,
            ControlErrorCode::Unavailable
        );
        assert_eq!(
            manager.start("owner", "r", "0", &args()).await.unwrap(),
            first
        );
        assert!(backend.writes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn output_gap_idle_and_exit_are_distinct_observations() {
        let backend = Backend::new(8);
        backend.push("old old ");
        backend.push("mysql> ");
        let manager = TerminalInteractionManager::new(backend.clone());
        let read = |cursor, idle| OutputWaitArguments {
            from_cursor: cursor,
            wait: WaitCondition {
                idle_ms: idle,
                ..Default::default()
            },
            timeout_ms: 200,
            max_bytes: 64,
        };
        let gap = manager.output_wait("r", read(0, None)).await.unwrap();
        assert_eq!(gap.reason, WaitReason::OutputGap);
        assert!(gap.output.truncated);
        let idle = manager
            .output_wait("r", read(gap.output.next_cursor, Some(100)))
            .await
            .unwrap();
        assert_eq!(idle.reason, WaitReason::Idle);
        backend.running.store(false, Ordering::SeqCst);
        assert_eq!(
            manager
                .output_wait("r", read(idle.output.next_cursor, None))
                .await
                .unwrap()
                .reason,
            WaitReason::RuntimeExited
        );
    }

    #[tokio::test]
    async fn record_lookup_checks_both_owner_and_runtime() {
        let manager = TerminalInteractionManager::new(Backend::new(MAX_BYTES));
        let id = manager.start("owner", "r", "key", &args()).await.unwrap();
        assert_eq!(
            manager.cancel("other", "r", &id).unwrap_err().code,
            ControlErrorCode::NotFound
        );
        assert_eq!(
            manager.cancel("owner", "other", &id).unwrap_err().code,
            ControlErrorCode::NotFound
        );
        manager.cancel("owner", "r", &id).unwrap();
    }

    #[tokio::test]
    async fn mode_aware_input_and_retries_use_runtime_screen_state() {
        let backend = Backend::new(MAX_BYTES);
        let manager = TerminalInteractionManager::new(backend.clone());
        backend.push("\x1b[?1h");
        let arrow: InteractArguments = serde_json::from_value(
            serde_json::json!({"input":{"type":"key","key":"ArrowUp"},"timeoutMs":0}),
        )
        .unwrap();
        let id = manager.start("owner", "r", "arrow", &arrow).await.unwrap();
        wait(&manager, &id, 80, 64).await;
        manager.cancel("owner", "r", &id).unwrap();
        backend.push("\x1b[?1l");
        assert_eq!(
            manager.start("owner", "r", "arrow", &arrow).await.unwrap(),
            id
        );
        let id = manager
            .start("owner", "r", "arrow-normal", &arrow)
            .await
            .unwrap();
        wait(&manager, &id, 80, 64).await;
        manager.cancel("owner", "r", &id).unwrap();
        let paste: InteractArguments = serde_json::from_value(
            serde_json::json!({"input":{"type":"autoPaste","text":"one\ntwo"},"timeoutMs":0}),
        )
        .unwrap();
        assert_eq!(
            manager
                .start("owner", "r", "paste", &paste)
                .await
                .unwrap_err()
                .code,
            ControlErrorCode::InvalidArguments
        );
        assert_eq!(backend.writes.lock().unwrap().len(), 2);
        backend.push("\x1b[?2004h");
        let id = manager.start("owner", "r", "paste", &paste).await.unwrap();
        wait(&manager, &id, 80, 64).await;
        manager.cancel("owner", "r", &id).unwrap();
        backend.push("\x1b[?2004l");
        assert_eq!(
            manager.start("owner", "r", "paste", &paste).await.unwrap(),
            id
        );
        assert_eq!(
            *backend.writes.lock().unwrap(),
            vec!["\x1bOA", "\x1b[A", "\x1b[200~one\rtwo\x1b[201~"]
        );
    }

    #[tokio::test]
    async fn prompt_wait_uses_changed_screen_and_distinguishes_continuation() {
        let backend = Backend::new(MAX_BYTES);
        backend.push("mysql> ");
        let manager = TerminalInteractionManager::new(backend.clone());
        let input: InteractArguments = serde_json::from_value(serde_json::json!({
            "input":{"type":"text","text":"SELECT 1","submit":true},
            "wait":{"prompt":{"cli":{"profile":"mysql"},"states":["prompt","continuation"]}},"timeoutMs":0
        })).unwrap();
        let id = manager.start("owner", "r", "prompt", &input).await.unwrap();
        assert_eq!(
            wait(&manager, &id, 80, 64).await["observation"]["reason"],
            "timeout"
        );
        // An invisible OSC/mode update must not make the old prompt fresh.
        backend.push("\x1b]0;title\x07\x1b[?2004h");
        assert_eq!(
            wait(&manager, &id, 80, 64).await["observation"]["reason"],
            "timeout"
        );
        backend.push("\r\x1b[K\x1b[32m    -> \x1b[0m");
        let observed = wait(&manager, &id, 100, 65536).await;
        assert_eq!(observed["observation"]["reason"], "prompt");
        assert_eq!(observed["observation"]["prompt"]["state"], "continuation");
        assert_eq!(observed["observation"]["prompt"]["source"], "preset");
    }

    #[tokio::test]
    async fn prompt_wait_does_not_finish_before_unread_output_or_on_incomplete_screen() {
        let backend = Backend::new(MAX_BYTES);
        let manager = TerminalInteractionManager::new(backend.clone());
        let input: InteractArguments = serde_json::from_value(serde_json::json!({
            "input":{"type":"key","key":"Enter"}, "wait":{"prompt":{"cli":{"profile":"mysql"}}},"timeoutMs":0
        })).unwrap();
        let id = manager.start("owner", "r", "prompt", &input).await.unwrap();
        wait(&manager, &id, 80, 64).await;
        backend.push("result\r\nmysql> ");
        assert_eq!(
            wait(&manager, &id, 100, 4).await["observation"]["reason"],
            "outputLimit"
        );
        assert_eq!(
            wait(&manager, &id, 100, 65536).await["observation"]["reason"],
            "prompt"
        );
        backend.output.lock().unwrap().resize_screen(10000, 10000);
        assert_eq!(
            manager
                .start("owner", "r", "incomplete", &input)
                .await
                .unwrap_err()
                .code,
            ControlErrorCode::Unavailable
        );
    }

    #[tokio::test]
    async fn prompt_freshness_handles_identical_redraw_and_unrelated_rows() {
        let backend = Backend::new(MAX_BYTES);
        backend.push("mysql> ");
        let manager = TerminalInteractionManager::new(backend.clone());
        let input: InteractArguments = serde_json::from_value(serde_json::json!({
            "input":{"type":"key","key":"CtrlL"},"wait":{"prompt":{"cli":{"profile":"mysql"}}},"timeoutMs":0
        })).unwrap();
        let id = manager.start("owner", "r", "redraw", &input).await.unwrap();
        wait(&manager, &id, 80, 65536).await;
        backend.push("\x1b7\x1b[2;1Hbackground\x1b8");
        assert_eq!(
            wait(&manager, &id, 80, 65536).await["observation"]["reason"],
            "timeout"
        );
        backend.push("\rmysql> ");
        assert_eq!(
            wait(&manager, &id, 100, 65536).await["observation"]["reason"],
            "prompt"
        );
        let cursor = backend.output.lock().unwrap().next_cursor();
        backend.push("\u{9d}0;secret title\u{9c}");
        let result = manager
            .output_wait(
                "r",
                OutputWaitArguments {
                    from_cursor: cursor,
                    wait: input.wait,
                    timeout_ms: 80,
                    max_bytes: 65536,
                },
            )
            .await
            .unwrap();
        assert_eq!(result.reason, WaitReason::Timeout);
    }

    #[test]
    fn input_is_explicit_and_limits_are_enforced() {
        let parse = |v| {
            serde_json::from_value::<InteractionInput>(v)
                .unwrap()
                .encode(None)
        };
        assert_eq!(
            parse(serde_json::json!({"type":"paste","text":"a\r\nb\nc","bracketedPaste":true}))
                .unwrap(),
            "\x1b[200~a\rb\rc\x1b[201~"
        );
        assert!(parse(serde_json::json!({"type":"text","text":"a\nb","submit":true})).is_err());
        assert!(parse(serde_json::json!({"type":"paste","text":"\u{1b}[201~"})).is_err());
        assert!(validate_limits(30001, 64).is_err());
        assert!(validate_limits(1, 3).is_err());
        assert!(
            serde_json::from_value::<InteractArguments>(
                serde_json::json!({"input":{"type":"key","key":"Enter"},"unknown":true})
            )
            .is_err()
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn real_macos_screen_modes_drive_keys_and_automatic_paste() {
        use crate::local_pty_backend::InProcessLocalPtyTerminalBackend;
        let backend = InProcessLocalPtyTerminalBackend::new();
        let directory = std::env::temp_dir().join(format!("luna-screen-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let runtime = backend
            .create(TerminalRuntimeCreateRequest {
                runtime_id: None,
                context: None,
                target_id: "local:macos-shell".into(),
                title: Some("screen test".into()),
                cwd: Some(directory.to_string_lossy().into()),
                command: None,
                authentication: None,
                managed_agent: None,
                launch_environment: [
                    ("ZDOTDIR".into(), directory.to_string_lossy().into_owned()),
                    ("TERM".into(), "dumb".into()),
                ]
                .into(),
                cols: 80,
                rows: 24,
            })
            .await
            .unwrap();
        let manager = TerminalInteractionManager::new(backend.clone());
        let scenario: ControlResult<_> = async {
            manager.output_wait(&runtime.runtime_id, OutputWaitArguments {
                from_cursor: 0, wait: WaitCondition { idle_ms: Some(200), ..Default::default() }, timeout_ms: 2000, max_bytes: 65536,
            }).await?;
            backend.resize(&runtime.runtime_id, 100, 30).await.map_err(unavailable)?;
            let setup: InteractArguments = serde_json::from_value(serde_json::json!({
                "input":{"type":"text","submit":true,"text":"stty raw -echo; printf '\\033[?1049h\\033[?1h\\033[?2004h\\033[H%s%s' 'SCREEN_' 'READY'; dd bs=1 count=3 of=arrow.bin 2>/dev/null; printf '\\r\\n%s%s' 'KEY_' 'READ'; dd bs=1 count=15 of=paste.bin 2>/dev/null; printf '\\033[?1049l\\033[?1l\\033[?2004l'; stty sane; printf '%s%s' 'SCREEN_' 'DONE'"},
                "wait":{"text":"SCREEN_READY"}
            })).unwrap();
            let id = manager.start("owner", &runtime.runtime_id, "screen-setup", &setup).await?;
            let ready = manager.wait("owner", &runtime.runtime_id, ExecutionWaitArguments { execution_id:id,timeout_ms:5000,max_bytes:65536 }).await?;
            let screen = backend.screen_snapshot(&runtime.runtime_id,65536).map_err(unavailable)?;
            let arrow: InteractArguments = serde_json::from_value(serde_json::json!({"input":{"type":"key","key":"ArrowUp"},"wait":{"text":"KEY_READ"}})).unwrap();
            let id = manager.start("owner", &runtime.runtime_id, "screen-arrow", &arrow).await?;
            let key_read = manager.wait("owner", &runtime.runtime_id, ExecutionWaitArguments { execution_id:id,timeout_ms:5000,max_bytes:65536 }).await?;
            let paste: InteractArguments = serde_json::from_value(serde_json::json!({"input":{"type":"autoPaste","text":"a\nb"},"wait":{"text":"SCREEN_DONE"}})).unwrap();
            let id = manager.start("owner", &runtime.runtime_id, "screen-paste", &paste).await?;
            let done = manager.wait("owner", &runtime.runtime_id, ExecutionWaitArguments { execution_id:id,timeout_ms:5000,max_bytes:65536 }).await?;
            let main = backend.screen_snapshot(&runtime.runtime_id,65536).map_err(unavailable)?;
            Ok((ready, screen, key_read, done, main))
        }.await;
        backend.close(&runtime.runtime_id).await.unwrap();
        let arrow = std::fs::read(directory.join("arrow.bin"));
        let paste = std::fs::read(directory.join("paste.bin"));
        let _ = std::fs::remove_dir_all(directory);
        let (ready, screen, key_read, done, main) = scenario.unwrap();
        assert_eq!(ready["observation"]["reason"], "matched");
        assert_eq!((screen.rows, screen.cols), (30, 100));
        assert!(
            screen.modes.alternate_screen
                && screen.modes.application_cursor
                && screen.modes.bracketed_paste
        );
        assert!(screen.lines[0].contains("SCREEN_READY"));
        assert_eq!(key_read["observation"]["reason"], "matched");
        assert_eq!(done["observation"]["reason"], "matched");
        assert!(!main.modes.alternate_screen);
        assert_eq!(arrow.unwrap(), b"\x1bOA");
        assert_eq!(paste.unwrap(), b"\x1b[200~a\rb\x1b[201~");
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn real_macos_pty_interaction_resumes_without_resubmitting() {
        use crate::local_pty_backend::InProcessLocalPtyTerminalBackend;
        let backend = InProcessLocalPtyTerminalBackend::new();
        let directory =
            std::env::temp_dir().join(format!("luna-interaction-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let runtime = backend
            .create(TerminalRuntimeCreateRequest {
                runtime_id: None,
                context: None,
                target_id: "local:macos-shell".into(),
                title: Some("interaction test".into()),
                cwd: Some(directory.to_string_lossy().into()),
                command: None,
                authentication: None,
                managed_agent: None,
                launch_environment: [
                    ("ZDOTDIR".into(), directory.to_string_lossy().into_owned()),
                    ("TERM".into(), "dumb".into()),
                ]
                .into(),
                cols: 80,
                rows: 24,
            })
            .await
            .unwrap();
        let manager = TerminalInteractionManager::new(backend.clone());
        let scenario = async {
            manager.output_wait(&runtime.runtime_id, OutputWaitArguments {
                from_cursor: 0, wait: WaitCondition { idle_ms: Some(200), ..Default::default() }, timeout_ms: 2000, max_bytes: 65536,
            }).await.unwrap();
            let input: InteractArguments = serde_json::from_value(serde_json::json!({
                "input": { "type": "text", "text": "sleep 0.3; printf '%s%s\\n' 'LUNA_' 'INTERACTION_READY'", "submit": true },
                "wait": { "text": "LUNA_INTERACTION_READY" }, "timeoutMs": 0
            })).unwrap();
            let id = manager.start("owner", &runtime.runtime_id, "native-test", &input).await.unwrap();
            let first = manager.wait("owner", &runtime.runtime_id, ExecutionWaitArguments {
                execution_id: id.clone(), timeout_ms: 50, max_bytes: 65536,
            }).await.unwrap();
            let replay = manager.start("owner", &runtime.runtime_id, "native-test", &input).await.unwrap();
            let second = manager.wait("owner", &runtime.runtime_id, ExecutionWaitArguments {
                execution_id: id.clone(), timeout_ms: 5000, max_bytes: 65536,
            }).await.unwrap();
            // A raw-mode foreground command deliberately stops reading. The large
            // paste must not block this current-thread async runtime's wait timer.
            let prepare: InteractArguments = serde_json::from_value(serde_json::json!({
                "input": { "type": "text", "text": "stty raw -echo; printf '%s%s' 'RAW_' 'READY'; sleep 1; dd bs=1 count=65536 of=/dev/null 2>/dev/null; stty sane; printf '%s%s' 'RAW_' 'DONE'", "submit": true },
                "wait": { "text": "RAW_READY" }
            })).unwrap();
            let raw_id = manager.start("owner", &runtime.runtime_id, "raw-setup", &prepare).await.unwrap();
            let ready = manager.wait("owner", &runtime.runtime_id, ExecutionWaitArguments {
                execution_id: raw_id, timeout_ms: 2000, max_bytes: 65536,
            }).await.unwrap();
            assert_eq!(ready["observation"]["reason"], "matched");
            let paste: InteractArguments = serde_json::from_value(serde_json::json!({
                "input": { "type": "paste", "text": "x".repeat(65536) }, "wait": { "text": "RAW_DONE" }
            })).unwrap();
            let paste_id = manager.start("owner", &runtime.runtime_id, "raw-paste", &paste).await.unwrap();
            let timer = Instant::now();
            let blocked = manager.wait("owner", &runtime.runtime_id, ExecutionWaitArguments {
                execution_id: paste_id.clone(), timeout_ms: 50, max_bytes: 65536,
            }).await.unwrap();
            let timer_elapsed = timer.elapsed();
            let drained = manager.wait("owner", &runtime.runtime_id, ExecutionWaitArguments {
                execution_id: paste_id, timeout_ms: 5000, max_bytes: 65536,
            }).await.unwrap();
            (id, replay, first, second, blocked, timer_elapsed, drained)
        }.await;
        backend.close(&runtime.runtime_id).await.unwrap();
        let _ = std::fs::remove_dir_all(directory);
        let (id, replay, first, second, blocked, timer_elapsed, drained) = scenario;
        assert_eq!(id, replay);
        assert_eq!(first["observation"]["reason"], "timeout");
        assert_eq!(second["observation"]["reason"], "matched");
        assert_eq!(
            second["observation"]["output"]["data"]
                .as_str()
                .unwrap()
                .matches("LUNA_INTERACTION_READY")
                .count(),
            1
        );
        assert_eq!(blocked["observation"]["reason"], "timeout");
        assert!(
            timer_elapsed < Duration::from_millis(500),
            "PTY write blocked the async timer: {timer_elapsed:?}"
        );
        assert_eq!(drained["observation"]["reason"], "matched");
    }
}
