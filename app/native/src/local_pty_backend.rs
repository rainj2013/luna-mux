#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
use std::path::PathBuf;
use std::{
    collections::HashMap,
    io::{Read, Write},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use async_trait::async_trait;
use portable_pty::{Child, ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use uuid::Uuid;

use crate::{
    sessions::Utf8Decoder,
    terminal_backend::{
        TerminalBackend, TerminalBackendResult, TerminalRuntimeEventSink,
        standard_terminal_capabilities,
    },
    terminal_output::{OUTPUT_CAPACITY_BYTES, OutputBuffer},
    terminal_runtime_contract::{
        TerminalCapabilities, TerminalRuntime, TerminalRuntimeCreateRequest, TerminalRuntimeEvent,
        TerminalRuntimeExitEvent, TerminalRuntimeExitReason, TerminalRuntimeOutputReadResult,
        TerminalRuntimeStatus, TerminalRuntimeStatusEvent, TerminalTarget, TerminalTargetKind,
        TerminalTransport,
    },
};

const LOCAL_TARGET_PREFIX: &str = "local:";
const POWERSHELL_TARGET: &str = "local:powershell";
const POWERSHELL5_TARGET: &str = "local:powershell5";
const WSL_TARGET_PREFIX: &str = "local:wsl:";
const MACOS_SHELL_TARGET: &str = "local:macos-shell";
const XTERM_256COLOR: &str = "xterm-256color";
const TRUECOLOR: &str = "truecolor";

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

struct RuntimeRecord {
    runtime: Mutex<TerminalRuntime>,
    output: Mutex<OutputBuffer>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    #[cfg(windows)]
    process_id: Option<u32>,
    #[cfg(target_os = "macos")]
    process_group_id: Option<i32>,
    paused: AtomicBool,
    close_requested: AtomicBool,
    reader_done: AtomicBool,
}

pub struct InProcessLocalPtyTerminalBackend {
    runtimes: RwLock<HashMap<String, Arc<RuntimeRecord>>>,
    event_sink: Arc<RwLock<Option<TerminalRuntimeEventSink>>>,
}

impl InProcessLocalPtyTerminalBackend {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            runtimes: RwLock::new(HashMap::new()),
            event_sink: Arc::new(RwLock::new(None)),
        })
    }

    pub fn is_local_target(target_id: &str) -> bool {
        target_id.starts_with(LOCAL_TARGET_PREFIX)
    }

    pub fn capabilities() -> TerminalCapabilities {
        standard_terminal_capabilities(false)
    }

    fn emit(&self, event: TerminalRuntimeEvent) {
        Self::emit_to_sink(&self.event_sink, event);
    }

    fn emit_to_sink(
        sink: &Arc<RwLock<Option<TerminalRuntimeEventSink>>>,
        event: TerminalRuntimeEvent,
    ) {
        if let Some(sink) = sink.read().expect("local event sink lock").as_ref() {
            sink(event);
        }
    }

    fn command_for_request(
        request: &TerminalRuntimeCreateRequest,
    ) -> TerminalBackendResult<CommandBuilder> {
        if is_powershell_target(&request.target_id) {
            #[cfg(windows)]
            {
                let executable = if request.target_id == POWERSHELL5_TARGET {
                    windows_powershell5_executable()
                        .ok_or_else(|| "未找到 Windows PowerShell 5.1（powershell.exe）".to_string())?
                } else {
                    windows_powershell7_executable()
                        .ok_or_else(|| "未找到 PowerShell 7（pwsh.exe）".to_string())?
                };
                let mut command = CommandBuilder::new(executable);
                command.args(["-NoLogo", "-NoExit"]);
                if let Some(bootstrap) = request
                    .launch_environment
                    .get("LUNA_MUX_AGENT_BOOTSTRAP")
                    .or_else(|| request.launch_environment.get("LUNA_MUX_CODEX_BOOTSTRAP"))
                    .filter(|value| !value.trim().is_empty())
                {
                    // Profiles are loaded explicitly so the bootstrap runs after fnm and
                    // other profile-managed PATH changes, while still preserving the
                    // standard PowerShell profile order.
                    let script = format!(
                        "{}; . '{}'",
                        crate::agent_command::powershell_profile_load_script(
                            request.target_id == POWERSHELL5_TARGET,
                        ),
                        bootstrap.replace('\'', "''")
                    );
                    command.args(["-NoProfile", "-Command", &script]);
                }
                return Ok(command);
            }
            #[cfg(not(windows))]
            {
                return Err("PowerShell 目标仅在 Windows 上可用".into());
            }
        }
        if let Some(distribution) = request.target_id.strip_prefix(WSL_TARGET_PREFIX) {
            #[cfg(windows)]
            {
                if distribution.trim().is_empty() {
                    return Err("WSL 目标缺少发行版名称".into());
                }
                let mut command = CommandBuilder::new("wsl.exe");
                command.args(["--distribution", distribution]);
                if let Some(cwd) = request
                    .cwd
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    command.args(["--cd", cwd]);
                }
                return Ok(command);
            }
            #[cfg(not(windows))]
            {
                let _ = distribution;
                return Err("WSL 目标仅在 Windows 上可用".into());
            }
        }
        if request.target_id == MACOS_SHELL_TARGET {
            #[cfg(target_os = "macos")]
            {
                let shell = macos_supported_shell()
                    .ok_or_else(|| "未找到受支持的 macOS Shell（zsh 或 bash）".to_string())?;
                let mut command = CommandBuilder::new(shell);
                command.args(["-l"]);
                return Ok(command);
            }
            #[cfg(not(target_os = "macos"))]
            {
                return Err("macOS Shell 目标仅在 macOS 上可用".into());
            }
        }
        Err("本地终端目标不存在".into())
    }

    #[cfg(windows)]
    fn discover_wsl_distributions() -> Vec<String> {
        // Reading the per-user WSL registration is effectively instantaneous and
        // does not wake the WSL service. `wsl.exe --list --quiet` can block for a
        // long time while the service or a distribution is cold-starting.
        let Ok(output) = windows_no_window_command("reg.exe")
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Lxss",
                "/s",
                "/v",
                "DistributionName",
            ])
            .output()
        else {
            return Vec::new();
        };
        if !output.status.success() {
            return Vec::new();
        }
        parse_wsl_registry_output(&decode_windows_command_output(&output.stdout))
    }

    fn spawn_worker(
        runtime_id: String,
        record: Arc<RuntimeRecord>,
        sink: Arc<RwLock<Option<TerminalRuntimeEventSink>>>,
        mut reader: Box<dyn Read + Send>,
        mut child: Box<dyn Child + Send + Sync>,
    ) {
        let reader_record = record.clone();
        let reader_sink = sink.clone();
        let reader_runtime_id = runtime_id.clone();
        thread::spawn(move || {
            let mut decoder = Utf8Decoder::default();
            let mut bytes = [0_u8; 8192];
            loop {
                while reader_record.paused.load(Ordering::Acquire) {
                    thread::sleep(std::time::Duration::from_millis(5));
                }
                match reader.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(size) => {
                        let text = decoder.push(&bytes[..size]);
                        if !text.is_empty() {
                            let event = {
                                let mut output =
                                    reader_record.output.lock().expect("local output lock");
                                let event = output.push(&reader_runtime_id, text.clone());
                                Some(event)
                            };
                            if let Some(event) = event {
                                Self::emit_to_sink(
                                    &reader_sink,
                                    TerminalRuntimeEvent::Output(event),
                                );
                            }
                        }
                    }
                    Err(_error) => {
                        let text = decoder.finish();
                        if !text.is_empty() {
                            let mut output =
                                reader_record.output.lock().expect("local output lock");
                            let event = output.push(&reader_runtime_id, text);
                            Self::emit_to_sink(&reader_sink, TerminalRuntimeEvent::Output(event));
                        }
                        reader_record.reader_done.store(true, Ordering::Release);
                        return;
                    }
                }
            }
            let tail = decoder.finish();
            if !tail.is_empty() {
                let mut output = reader_record.output.lock().expect("local output lock");
                let event = output.push(&reader_runtime_id, tail);
                Self::emit_to_sink(&reader_sink, TerminalRuntimeEvent::Output(event));
            }
            reader_record.reader_done.store(true, Ordering::Release);
        });
        thread::spawn(move || {
            let status = child.wait().ok();
            record.paused.store(false, Ordering::Release);
            let _ = record
                .writer
                .lock()
                .ok()
                .and_then(|mut writer| writer.take());
            let _ = record
                .master
                .lock()
                .ok()
                .and_then(|mut master| master.take());
            while !record.reader_done.load(Ordering::Acquire) {
                thread::sleep(std::time::Duration::from_millis(5));
            }
            let signal = status
                .as_ref()
                .and_then(|value| value.signal().map(str::to_owned));
            Self::finish_record(
                &runtime_id,
                &record,
                &sink,
                status.map(|value| value.exit_code() as i32),
                signal,
            );
        });
    }

    fn finish_record(
        runtime_id: &str,
        record: &RuntimeRecord,
        sink: &Arc<RwLock<Option<TerminalRuntimeEventSink>>>,
        exit_code: Option<i32>,
        signal: Option<String>,
    ) {
        let (runtime, reason, cursor) = {
            let mut runtime = record.runtime.lock().expect("local runtime lock");
            if matches!(
                runtime.status,
                TerminalRuntimeStatus::Exited | TerminalRuntimeStatus::Error
            ) {
                return;
            }
            runtime.status = TerminalRuntimeStatus::Exited;
            let reason = if record.close_requested.load(Ordering::Acquire) {
                TerminalRuntimeExitReason::Closed
            } else if signal.is_some() {
                TerminalRuntimeExitReason::Signaled
            } else {
                TerminalRuntimeExitReason::Exited
            };
            let cursor = record
                .output
                .lock()
                .expect("local output lock")
                .next_cursor();
            (runtime.clone(), reason, cursor)
        };
        Self::emit_to_sink(
            sink,
            TerminalRuntimeEvent::Status(TerminalRuntimeStatusEvent { runtime }),
        );
        Self::emit_to_sink(
            sink,
            TerminalRuntimeEvent::Exit(TerminalRuntimeExitEvent {
                runtime_id: runtime_id.into(),
                reason,
                cursor,
                exit_code,
                signal,
                message: None,
            }),
        );
    }

    pub async fn close_all(&self) {
        let ids = self
            .list()
            .unwrap_or_default()
            .into_iter()
            .map(|runtime| runtime.runtime_id)
            .collect::<Vec<_>>();
        for id in ids {
            let _ = self.close(&id).await;
        }
    }

    fn initial_input(request: &TerminalRuntimeCreateRequest) -> Option<String> {
        let command = request
            .command
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        command.map(|command| format!("{command}\r"))
    }
}

fn configure_terminal_environment(command: &mut CommandBuilder, target_id: &str) {
    if target_id == MACOS_SHELL_TARGET {
        // Finder-launched applications do not inherit TERM from a parent terminal.
        command.env("TERM", XTERM_256COLOR);
        command.env("COLORTERM", TRUECOLOR);
    }
}

#[cfg(windows)]
fn is_real_powershell7_executable(path: &std::path::Path) -> bool {
    windows_no_window_command(path)
        .args(["-NoLogo", "-NoProfile", "-Command", "exit 0"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
pub(crate) fn windows_powershell7_executable() -> Option<String> {
    std::env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .map(|root| root.join("PowerShell").join("7").join("pwsh.exe"))
        .filter(|candidate| is_real_powershell7_executable(candidate))
        .or_else(|| {
            std::env::var_os("PATH").and_then(|path| {
                std::env::split_paths(&path)
                    .map(|directory| directory.join("pwsh.exe"))
                    .find(|candidate| is_real_powershell7_executable(candidate))
            })
        })
        .or_else(|| {
            first_command_path("pwsh.exe")
                .filter(|candidate| is_real_powershell7_executable(candidate))
        })
        .map(|path| path.to_string_lossy().into_owned())
}

#[cfg(windows)]
pub(crate) fn windows_powershell5_executable() -> Option<String> {
    std::env::var_os("PATH")
        .and_then(|path| {
            std::env::split_paths(&path)
                .map(|directory| directory.join("powershell.exe"))
                .find(|candidate| candidate.is_file())
        })
        .or_else(|| first_command_path("powershell.exe"))
        .or_else(|| {
            std::env::var_os("WINDIR")
                .or_else(|| std::env::var_os("SystemRoot"))
                .map(PathBuf::from)
                .map(|root| {
                    root.join("System32")
                        .join("WindowsPowerShell")
                        .join("v1.0")
                        .join("powershell.exe")
                })
                .filter(|candidate| candidate.is_file())
        })
        .map(|path| path.to_string_lossy().into_owned())
}

pub(crate) fn is_powershell_target(target_id: &str) -> bool {
    target_id == POWERSHELL_TARGET || target_id == POWERSHELL5_TARGET
}

#[cfg(windows)]
fn first_command_path(command: &str) -> Option<PathBuf> {
    let output = windows_no_window_command("where.exe")
        .arg(command)
        .output()
        .ok()?;
    output.status.success().then_some(())?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .find(|candidate| candidate.is_file())
}

#[cfg(windows)]
pub(crate) fn windows_no_window_command(
    program: impl AsRef<std::ffi::OsStr>,
) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

fn wsl_environment_bridge(request: &TerminalRuntimeCreateRequest) -> String {
    let mut names = std::env::var("WSLENV")
        .unwrap_or_default()
        .split(':')
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    names.extend(request.launch_environment.keys().cloned());
    if request.managed_agent.is_some() {
        names.extend(
            [
                "LUNA_MUX_SESSION_ID",
                "LUNA_MUX_PANE_ID",
                "LUNA_MUX_RUNTIME_ID",
                "LUNA_MUX_AGENT_ID",
                "LUNA_MUX_LAUNCH_PROFILE_ID",
            ]
            .into_iter()
            .map(str::to_owned),
        );
    }
    names.sort();
    names.dedup();
    names.join(":")
}

#[async_trait]
impl TerminalBackend for InProcessLocalPtyTerminalBackend {
    fn set_event_sink(&self, sink: TerminalRuntimeEventSink) {
        *self.event_sink.write().expect("local event sink lock") = Some(sink);
    }

    fn targets(&self) -> TerminalBackendResult<Vec<TerminalTarget>> {
        let mut targets = Vec::new();
        #[cfg(windows)]
        {
            if windows_powershell7_executable().is_some() {
                targets.push(TerminalTarget {
                    id: POWERSHELL_TARGET.into(),
                    label: "PowerShell 7".into(),
                    transport: TerminalTransport::LocalPty,
                    kind: TerminalTargetKind::Powershell,
                    capabilities: Self::capabilities(),
                });
            }
            if windows_powershell5_executable().is_some() {
                targets.push(TerminalTarget {
                    id: POWERSHELL5_TARGET.into(),
                    label: "PowerShell 5.1".into(),
                    transport: TerminalTransport::LocalPty,
                    kind: TerminalTargetKind::Powershell,
                    capabilities: Self::capabilities(),
                });
            }
            targets.extend(
                Self::discover_wsl_distributions()
                    .into_iter()
                    .map(|distribution| TerminalTarget {
                        id: format!("{WSL_TARGET_PREFIX}{distribution}"),
                        label: format!("WSL · {distribution}"),
                        transport: TerminalTransport::LocalPty,
                        kind: TerminalTargetKind::Wsl,
                        capabilities: Self::capabilities(),
                    }),
            );
        }
        #[cfg(target_os = "macos")]
        if let Some(shell) = macos_supported_shell() {
            let label = std::path::Path::new(&shell)
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| format!("macOS {name}"))
                .unwrap_or_else(|| "macOS Shell".into());
            targets.push(TerminalTarget {
                id: MACOS_SHELL_TARGET.into(),
                label,
                transport: TerminalTransport::LocalPty,
                kind: TerminalTargetKind::MacosShell,
                capabilities: Self::capabilities(),
            });
        }
        Ok(targets)
    }

    fn list(&self) -> TerminalBackendResult<Vec<TerminalRuntime>> {
        Ok(self
            .runtimes
            .read()
            .map_err(|_| "本地 Runtime 状态锁已损坏")?
            .values()
            .map(|record| record.runtime.lock().expect("local runtime lock").clone())
            .collect())
    }

    async fn create(
        &self,
        mut request: TerminalRuntimeCreateRequest,
    ) -> TerminalBackendResult<TerminalRuntime> {
        if request.authentication.is_some() {
            return Err("本地终端不支持该认证方式".into());
        }
        if !request.target_id.starts_with(WSL_TARGET_PREFIX)
            && let Some(context) = request.context.as_ref()
            && let Some(shim_dir) = crate::agent_adapters::install_runtime_shims(
                context,
                &request.target_id,
                request
                    .launch_environment
                    .get("LUNA_MUX_HOOK_ENDPOINT")
                    .map(String::as_str),
                request
                    .launch_environment
                    .get("LUNA_MUX_MCP_ENDPOINT")
                    .map(String::as_str),
            )?
        {
            let current_path = std::env::var_os("PATH").unwrap_or_default();
            let mut paths = vec![shim_dir.clone()];
            paths.extend(std::env::split_paths(&current_path));
            let joined = std::env::join_paths(paths).map_err(|error| error.to_string())?;
            request
                .launch_environment
                .insert("PATH".into(), joined.to_string_lossy().into_owned());
            if is_powershell_target(&request.target_id) {
                request.launch_environment.insert(
                    "LUNA_MUX_AGENT_BOOTSTRAP".into(),
                    shim_dir
                        .join("bootstrap.ps1")
                        .to_string_lossy()
                        .into_owned(),
                );
            }
            #[cfg(target_os = "macos")]
            if request.target_id == MACOS_SHELL_TARGET
                && macos_supported_shell().is_some_and(|shell| {
                    std::path::Path::new(&shell)
                        .file_name()
                        .and_then(|name| name.to_str())
                        == Some("zsh")
                })
            {
                request
                    .launch_environment
                    .insert("ZDOTDIR".into(), shim_dir.to_string_lossy().into_owned());
                if let Some(user_zdotdir) = std::env::var_os("ZDOTDIR")
                    .filter(|value| !value.is_empty())
                    .or_else(|| std::env::var_os("HOME"))
                {
                    request.launch_environment.insert(
                        "LUNA_MUX_USER_ZDOTDIR".into(),
                        user_zdotdir.to_string_lossy().into_owned(),
                    );
                }
            }
            #[cfg(target_os = "macos")]
            if request.target_id == MACOS_SHELL_TARGET
                && macos_supported_shell().is_some_and(|shell| {
                    std::path::Path::new(&shell)
                        .file_name()
                        .and_then(|name| name.to_str())
                        == Some("bash")
                })
            {
                let bootstrap = format!(
                    ". {}",
                    crate::shell_quoting::posix_shell_quote(
                        &shim_dir.join("bootstrap.sh").to_string_lossy()
                    )
                );
                request.command = Some(match request.command.take() {
                    Some(command) if !command.trim().is_empty() => {
                        format!("{bootstrap}; {command}")
                    }
                    _ => bootstrap,
                });
            }
        }
        let mut command = Self::command_for_request(&request)?;
        for (key, value) in &request.launch_environment {
            command.env(key, value);
        }
        configure_terminal_environment(&mut command, &request.target_id);
        if let Some(agent) = &request.managed_agent {
            command.env("LUNA_MUX_SESSION_ID", &agent.mux_session_id);
            command.env("LUNA_MUX_PANE_ID", &agent.pane_id);
            command.env("LUNA_MUX_RUNTIME_ID", &agent.runtime_id);
            command.env("LUNA_MUX_AGENT_ID", &agent.agent_id);
            command.env("LUNA_MUX_LAUNCH_PROFILE_ID", &agent.launch_profile_id);
        }
        if request.target_id.starts_with(WSL_TARGET_PREFIX) {
            command.env("WSLENV", wsl_environment_bridge(&request));
        }
        if !request.target_id.starts_with(WSL_TARGET_PREFIX)
            && let Some(cwd) = request
                .cwd
                .as_deref()
                .filter(|value| !value.trim().is_empty())
        {
            command.cwd(cwd);
        }
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: request.rows.clamp(1, u16::MAX as u32) as u16,
                cols: request.cols.clamp(1, u16::MAX as u32) as u16,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| error.to_string())?;
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| error.to_string())?;
        #[cfg(windows)]
        let process_id = child.process_id();
        #[cfg(target_os = "macos")]
        let process_group_id = pair.master.process_group_leader();
        let killer = child.clone_killer();
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| error.to_string())?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| error.to_string())?;
        let runtime = TerminalRuntime {
            runtime_id: request
                .runtime_id
                .clone()
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            target_id: request.target_id.clone(),
            title: request.title.clone().unwrap_or_else(|| "本地终端".into()),
            status: TerminalRuntimeStatus::Running,
            capabilities: Self::capabilities(),
            context: request.context.clone(),
            managed_agent: request.managed_agent.clone(),
            error: None,
        };
        let record = Arc::new(RuntimeRecord {
            runtime: Mutex::new(runtime.clone()),
            output: Mutex::new(OutputBuffer::new(OUTPUT_CAPACITY_BYTES)),
            writer: Mutex::new(Some(writer)),
            master: Mutex::new(Some(pair.master)),
            killer: Mutex::new(killer),
            #[cfg(windows)]
            process_id,
            #[cfg(target_os = "macos")]
            process_group_id,
            paused: AtomicBool::new(false),
            close_requested: AtomicBool::new(false),
            reader_done: AtomicBool::new(false),
        });
        self.runtimes
            .write()
            .map_err(|_| "本地 Runtime 状态锁已损坏")?
            .insert(runtime.runtime_id.clone(), record);
        self.emit(TerminalRuntimeEvent::Status(TerminalRuntimeStatusEvent {
            runtime: runtime.clone(),
        }));
        let record = self
            .runtimes
            .read()
            .map_err(|_| "本地 Runtime 状态锁已损坏")?
            .get(&runtime.runtime_id)
            .cloned()
            .ok_or_else(|| "本地 Runtime 不存在".to_string())?;
        Self::spawn_worker(
            runtime.runtime_id.clone(),
            record,
            self.event_sink.clone(),
            reader,
            child,
        );
        if let Some(input) = Self::initial_input(&request) {
            let _ = self.write(&runtime.runtime_id, &input).await;
        }
        Ok(runtime)
    }

    async fn write(&self, runtime_id: &str, data: &str) -> TerminalBackendResult<()> {
        let runtimes = self
            .runtimes
            .read()
            .map_err(|_| "本地 Runtime 状态锁已损坏")?;
        let record = runtimes
            .get(runtime_id)
            .ok_or_else(|| "终端 Runtime 不存在".to_string())?;
        let mut writer_guard = record
            .writer
            .lock()
            .map_err(|_| "本地 PTY writer 锁已损坏")?;
        let writer = writer_guard
            .as_mut()
            .ok_or_else(|| "本地 PTY 已退出".to_string())?;
        writer
            .write_all(data.as_bytes())
            .map_err(|error| error.to_string())?;
        writer.flush().map_err(|error| error.to_string())
    }

    async fn resize(&self, runtime_id: &str, cols: u32, rows: u32) -> TerminalBackendResult<()> {
        let runtimes = self
            .runtimes
            .read()
            .map_err(|_| "本地 Runtime 状态锁已损坏")?;
        let record = runtimes
            .get(runtime_id)
            .ok_or_else(|| "终端 Runtime 不存在".to_string())?;
        record
            .master
            .lock()
            .map_err(|_| "本地 PTY master 锁已损坏")?
            .as_ref()
            .ok_or_else(|| "本地 PTY 已退出".to_string())?
            .resize(PtySize {
                rows: rows.clamp(1, u16::MAX as u32) as u16,
                cols: cols.clamp(1, u16::MAX as u32) as u16,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| error.to_string())
    }

    fn set_output_paused(&self, runtime_id: &str, paused: bool) -> TerminalBackendResult<()> {
        let record = self
            .runtimes
            .read()
            .map_err(|_| "本地 Runtime 状态锁已损坏")?
            .get(runtime_id)
            .cloned()
            .ok_or_else(|| "终端 Runtime 不存在".to_string())?;
        record.paused.store(paused, Ordering::Release);
        Ok(())
    }

    async fn interrupt(&self, runtime_id: &str) -> TerminalBackendResult<()> {
        self.write(runtime_id, "\u{3}").await
    }

    async fn close(&self, runtime_id: &str) -> TerminalBackendResult<()> {
        let record = self
            .runtimes
            .read()
            .map_err(|_| "本地 Runtime 状态锁已损坏")?
            .get(runtime_id)
            .cloned()
            .ok_or_else(|| "终端 Runtime 不存在".to_string())?;
        if matches!(
            record
                .runtime
                .lock()
                .map_err(|_| "本地 runtime 锁已损坏")?
                .status,
            TerminalRuntimeStatus::Exited | TerminalRuntimeStatus::Error
        ) {
            return Ok(());
        }
        record.close_requested.store(true, Ordering::Release);
        record.paused.store(false, Ordering::Release);
        #[cfg(windows)]
        if let Some(process_id) = record.process_id {
            let _ = windows_no_window_command("taskkill")
                .args(["/PID", &process_id.to_string(), "/T", "/F"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        #[cfg(target_os = "macos")]
        if let Some(process_group_id) = record.process_group_id {
            // portable-pty starts the shell as a new session leader; kill the whole
            // group so background commands do not survive closing their pane.
            unsafe {
                libc::killpg(process_group_id, libc::SIGKILL);
            }
        }
        let kill_result = record
            .killer
            .lock()
            .map_err(|_| "本地 child killer 锁已损坏")?
            .kill();
        #[cfg(windows)]
        if let Err(error) = &kill_result {
            if error.raw_os_error() == Some(0) {
                return Ok(());
            }
        }
        kill_result.map_err(|error| error.to_string())
    }

    fn read_output(
        &self,
        runtime_id: &str,
        from_cursor: u64,
        max_bytes: usize,
    ) -> TerminalBackendResult<TerminalRuntimeOutputReadResult> {
        let record = self
            .runtimes
            .read()
            .map_err(|_| "本地 Runtime 状态锁已损坏")?
            .get(runtime_id)
            .cloned()
            .ok_or_else(|| "终端 Runtime 不存在".to_string())?;
        record
            .output
            .lock()
            .map_err(|_| "本地 output 锁已损坏")?
            .read(runtime_id, from_cursor, max_bytes)
    }
}

#[cfg(windows)]
pub(crate) fn decode_windows_command_output(bytes: &[u8]) -> String {
    let looks_utf16 = bytes.len() >= 2
        && bytes.len().is_multiple_of(2)
        && bytes
            .iter()
            .skip(1)
            .step_by(2)
            .filter(|byte| **byte == 0)
            .count()
            * 2
            >= bytes.len() / 2;
    if looks_utf16 {
        let units = bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]));
        String::from_utf16_lossy(&units.collect::<Vec<_>>())
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

#[cfg(windows)]
fn parse_wsl_registry_output(output: &str) -> Vec<String> {
    let mut distributions = output
        .lines()
        .filter_map(|line| line.split_once("REG_SZ").map(|(_, value)| value.trim()))
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    distributions.sort_unstable();
    distributions.dedup();
    distributions
}

#[cfg(target_os = "macos")]
pub(crate) fn macos_supported_shell() -> Option<String> {
    if let Ok(shell) = std::env::var("SHELL")
        && supported_macos_shell(&shell)
    {
        return Some(shell);
    }
    unsafe {
        let account = libc::getpwuid(libc::geteuid());
        if !account.is_null() && !(*account).pw_shell.is_null() {
            let shell = std::ffi::CStr::from_ptr((*account).pw_shell)
                .to_string_lossy()
                .into_owned();
            if supported_macos_shell(&shell) {
                return Some(shell);
            }
        }
    }
    ["/bin/zsh", "/bin/bash"]
        .into_iter()
        .find(|shell| std::path::Path::new(shell).is_file())
        .map(str::to_owned)
}

#[cfg(target_os = "macos")]
fn supported_macos_shell(shell: &str) -> bool {
    std::path::Path::new(shell.trim())
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, "zsh" | "bash"))
}

#[cfg(test)]
mod terminal_environment_tests {
    use std::ffi::OsStr;
    #[cfg(target_os = "macos")]
    use std::time::Duration;

    use super::*;

    #[test]
    fn macos_shell_uses_the_emulated_terminal_capabilities() {
        let mut command = CommandBuilder::new("zsh");
        command.env("TERM", "dumb");

        configure_terminal_environment(&mut command, MACOS_SHELL_TARGET);

        assert_eq!(command.get_env("TERM"), Some(OsStr::new(XTERM_256COLOR)));
        assert_eq!(command.get_env("COLORTERM"), Some(OsStr::new(TRUECOLOR)));
    }

    #[test]
    fn powershell_and_wsl_keep_their_platform_environment() {
        for target_id in [POWERSHELL_TARGET, POWERSHELL5_TARGET, "local:wsl:Ubuntu"] {
            let mut command = CommandBuilder::new("shell");
            command.env("TERM", "platform-default");
            command.env("COLORTERM", "platform-color");

            configure_terminal_environment(&mut command, target_id);

            assert_eq!(
                command.get_env("TERM"),
                Some(OsStr::new("platform-default"))
            );
            assert_eq!(
                command.get_env("COLORTERM"),
                Some(OsStr::new("platform-color"))
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn macos_zsh_backspace_emits_cursor_controls_without_a_parent_term() {
        let Some(shell) = macos_supported_shell() else {
            return;
        };
        if std::path::Path::new(&shell)
            .file_name()
            .and_then(|name| name.to_str())
            != Some("zsh")
        {
            return;
        }

        let zdotdir = std::env::temp_dir().join(format!("luna-mux-zsh-test-{}", Uuid::new_v4()));
        std::fs::create_dir(&zdotdir).expect("create empty ZDOTDIR");
        let mut launch_environment = std::collections::BTreeMap::new();
        launch_environment.insert("TERM".into(), "dumb".into());
        launch_environment.insert("ZDOTDIR".into(), zdotdir.to_string_lossy().into_owned());
        let request = TerminalRuntimeCreateRequest {
            runtime_id: None,
            context: None,
            target_id: MACOS_SHELL_TARGET.into(),
            title: Some("zsh backspace test".into()),
            cwd: None,
            command: None,
            authentication: None,
            managed_agent: None,
            launch_environment,
            cols: 80,
            rows: 24,
        };
        let backend = InProcessLocalPtyTerminalBackend::new();
        let runtime = backend.create(request).await.expect("start zsh PTY");

        std::thread::sleep(Duration::from_millis(150));
        let prompt = backend
            .read_output(&runtime.runtime_id, 0, 64 * 1024)
            .expect("read initial zsh prompt");
        backend
            .write(&runtime.runtime_id, "abc\u{7f}")
            .await
            .expect("write text and Backspace");

        let mut output = String::new();
        let mut cursor = prompt.next_cursor;
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(25));
            let next = backend
                .read_output(&runtime.runtime_id, cursor, 64 * 1024)
                .expect("read zsh echo");
            output.push_str(&next.data);
            cursor = next.next_cursor;
            if output.contains("\u{8} \u{8}") {
                break;
            }
        }

        backend
            .close(&runtime.runtime_id)
            .await
            .expect("close zsh PTY");
        let _ = std::fs::remove_dir(&zdotdir);
        assert!(
            output.contains("\u{8} \u{8}"),
            "zsh did not emit a visual erase sequence: {output:?}"
        );
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::time::Duration;

    fn powershell_test_target_id() -> &'static str {
        if windows_powershell7_executable().is_some() {
            POWERSHELL_TARGET
        } else {
            POWERSHELL5_TARGET
        }
    }

    fn request(command: Option<&str>) -> TerminalRuntimeCreateRequest {
        request_for_target(powershell_test_target_id(), command)
    }

    fn request_for_target(target_id: &str, command: Option<&str>) -> TerminalRuntimeCreateRequest {
        TerminalRuntimeCreateRequest {
            runtime_id: None,
            context: None,
            target_id: target_id.into(),
            title: Some("PTY test".into()),
            cwd: None,
            command: command.map(str::to_owned),
            authentication: None,
            managed_agent: None,
            launch_environment: Default::default(),
            cols: 100,
            rows: 30,
        }
    }

    #[tokio::test]
    async fn powershell_runtime_preserves_unicode_and_exits() {
        let mut target_ids = Vec::new();
        if windows_powershell7_executable().is_some() {
            target_ids.push(POWERSHELL_TARGET);
        }
        if windows_powershell5_executable().is_some() {
            target_ids.push(POWERSHELL5_TARGET);
        }
        assert!(!target_ids.is_empty(), "no PowerShell target is available");

        for target_id in target_ids {
            assert_powershell_runtime_preserves_unicode_and_exits(target_id).await;
        }
    }

    async fn assert_powershell_runtime_preserves_unicode_and_exits(target_id: &str) {
        let backend = InProcessLocalPtyTerminalBackend::new();
        let runtime = backend
            .create(request_for_target(
                target_id,
                Some("Write-Output 'LunaMux local ✓'; exit 0"),
            ))
            .await
            .unwrap_or_else(|error| panic!("create {target_id} runtime: {error}"));
        let mut cursor = 0;
        let mut collected = String::new();
        for _ in 0..100 {
            let output = backend
                .read_output(&runtime.runtime_id, cursor, 64 * 1024)
                .expect("read PTY output");
            if output.next_cursor > cursor {
                if output.data.contains("\u{1b}[6n") {
                    let _ = backend.write(&runtime.runtime_id, "\u{1b}[1;1R").await;
                }
                collected.push_str(&output.data);
                cursor = output.next_cursor;
            }
            if backend
                .list()
                .expect("list runtimes")
                .iter()
                .find(|item| item.runtime_id == runtime.runtime_id)
                .is_some_and(|item| item.status == TerminalRuntimeStatus::Exited)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(
            collected.contains("LunaMux local"),
            "{target_id} output was: {collected:?}"
        );
        assert_eq!(
            backend
                .list()
                .expect("list runtimes")
                .into_iter()
                .find(|item| item.runtime_id == runtime.runtime_id)
                .expect("runtime remains queryable")
                .status,
            TerminalRuntimeStatus::Exited
        );
    }

    #[tokio::test]
    async fn close_kills_powershell_runtime() {
        let backend = InProcessLocalPtyTerminalBackend::new();
        let runtime = backend
            .create(request(None))
            .await
            .expect("create PowerShell runtime");
        backend
            .close(&runtime.runtime_id)
            .await
            .expect("close runtime");
        for _ in 0..100 {
            if backend
                .list()
                .expect("list runtimes")
                .iter()
                .find(|item| item.runtime_id == runtime.runtime_id)
                .is_some_and(|item| item.status == TerminalRuntimeStatus::Exited)
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("PowerShell runtime did not exit after close");
    }

    #[test]
    fn decodes_wsl_utf16_distribution_output() {
        let bytes = "Ubuntu\r\nDebian\r\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        assert_eq!(
            decode_windows_command_output(&bytes),
            "Ubuntu\r\nDebian\r\n"
        );
    }

    #[test]
    fn parses_wsl_distributions_from_registry_output() {
        let output = "    DistributionName    REG_SZ    Ubuntu 24.04\r\n    DistributionName    REG_SZ    Debian\r\n";
        assert_eq!(
            parse_wsl_registry_output(output),
            ["Debian", "Ubuntu 24.04"]
        );
    }

    #[test]
    fn powershell_loads_profiles_before_the_codex_bootstrap() {
        let mut request = request(None);
        request.launch_environment.insert(
            "LUNA_MUX_CODEX_BOOTSTRAP".into(),
            r"C:\Temp\luna-mux\runtime\bin\bootstrap.ps1".into(),
        );

        let command = InProcessLocalPtyTerminalBackend::command_for_request(&request)
            .expect("build PowerShell command");
        let argv = command
            .get_argv()
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert!(argv.iter().any(|value| value == "-NoProfile"));
        let script = argv
            .last()
            .expect("PowerShell initialization command is present");
        let profile_index = script
            .find("$PROFILE.CurrentUserCurrentHost")
            .expect("normal PowerShell profiles are loaded");
        let bootstrap_index = script
            .find(r"C:\Temp\luna-mux\runtime\bin\bootstrap.ps1")
            .expect("Luna Codex bootstrap is loaded");
        assert!(profile_index < bootstrap_index);
    }

    #[test]
    fn builds_wsl_target_with_distribution_and_cwd() {
        let request = TerminalRuntimeCreateRequest {
            runtime_id: None,
            context: None,
            target_id: format!("{WSL_TARGET_PREFIX}Ubuntu"),
            title: None,
            cwd: Some("D:\\code\\project".into()),
            command: Some("codex".into()),
            authentication: None,
            managed_agent: None,
            launch_environment: Default::default(),
            cols: 100,
            rows: 30,
        };
        let command = InProcessLocalPtyTerminalBackend::command_for_request(&request)
            .expect("build WSL command");
        assert_eq!(
            command
                .get_argv()
                .iter()
                .map(|value| value.to_string_lossy())
                .collect::<Vec<_>>(),
            [
                "wsl.exe",
                "--distribution",
                "Ubuntu",
                "--cd",
                "D:\\code\\project"
            ]
        );
        assert_eq!(
            InProcessLocalPtyTerminalBackend::initial_input(&request),
            Some("codex\r".into())
        );
    }

    #[test]
    fn bridges_managed_agent_and_hook_environment_into_wsl() {
        let mut request = TerminalRuntimeCreateRequest {
            runtime_id: Some("runtime-1".into()),
            context: None,
            target_id: format!("{WSL_TARGET_PREFIX}Ubuntu"),
            title: None,
            cwd: None,
            command: Some("codex".into()),
            authentication: None,
            managed_agent: Some(
                crate::terminal_runtime_contract::TerminalManagedAgentContext {
                    mux_session_id: "session-1".into(),
                    pane_id: "pane-1".into(),
                    runtime_id: "runtime-1".into(),
                    agent_id: "agent-1".into(),
                    launch_profile_id: "codex.default".into(),
                },
            ),
            launch_environment: Default::default(),
            cols: 100,
            rows: 30,
        };
        request.launch_environment.insert(
            "LUNA_MUX_HOOK_ENDPOINT".into(),
            "http://127.0.0.1:1234/v1/hooks".into(),
        );
        request
            .launch_environment
            .insert("LUNA_MUX_HOOK_AUTHORIZATION".into(), "secret".into());
        request.launch_environment.insert(
            "LUNA_MUX_MCP_ENDPOINT".into(),
            "http://127.0.0.1:1235/mcp".into(),
        );
        request
            .launch_environment
            .insert("LUNA_MUX_MCP_AUTHORIZATION".into(), "mcp-secret".into());
        let bridge = wsl_environment_bridge(&request);
        for name in [
            "LUNA_MUX_HOOK_ENDPOINT",
            "LUNA_MUX_HOOK_AUTHORIZATION",
            "LUNA_MUX_MCP_ENDPOINT",
            "LUNA_MUX_MCP_AUTHORIZATION",
            "LUNA_MUX_SESSION_ID",
            "LUNA_MUX_RUNTIME_ID",
            "LUNA_MUX_AGENT_ID",
        ] {
            assert!(bridge.split(':').any(|value| value == name));
        }
        assert!(!bridge.contains("secret"));
        assert!(!bridge.contains("mcp-secret"));
    }
}
