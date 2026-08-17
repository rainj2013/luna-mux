use std::{
    collections::HashMap,
    future::Future,
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::Emitter;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};
use uuid::Uuid;

const AGENT_BROWSER_TOOLS: &str = "core,network,debug,tabs";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(not(windows))]
const AGENT_BROWSER_SOCKET_DIR: &str = "/tmp/luna-mux-ab";

#[derive(Clone, Default)]
pub(crate) struct BrowserWarmupGate {
    sessions: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<Option<String>>>>>>,
}

impl BrowserWarmupGate {
    pub(crate) async fn warm_once<F, Fut>(
        &self,
        mux_session_id: &str,
        runtime_id: &str,
        warmup: F,
    ) -> Result<(), String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(), String>>,
    {
        // PreToolUse is emitted for every browser command. Serialize the first
        // probe and remember the concrete runtime so a normal workflow does not
        // spawn a fresh agent-browser probe for every command. A restarted
        // Browser Resource has a new runtime ID and is warmed again.
        let session = self
            .sessions
            .lock()
            .map_err(|_| "Browser 预热状态已损坏".to_string())?
            .entry(mux_session_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(None)))
            .clone();
        let mut warmed_runtime_id = session.lock().await;
        if warmed_runtime_id.as_deref() == Some(runtime_id) {
            return Ok(());
        }
        warmup().await?;
        *warmed_runtime_id = Some(runtime_id.to_string());
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserRuntimeRegistryEntry {
    mux_session_id: String,
    runtime_id: String,
    cdp_port: u16,
    process_id: u32,
    status: BrowserRuntimeStatus,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentBrowserMcpConfig {
    cdp: String,
    content_boundaries: bool,
    max_output: u32,
    pin_tab: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChromeInstallation {
    pub executable_path: String,
    pub version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserRuntime {
    pub id: String,
    pub mux_session_id: String,
    pub browser_resource_id: String,
    pub url: String,
    pub cdp_port: u16,
    pub profile_path: String,
    pub process_id: u32,
    pub status: BrowserRuntimeStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BrowserRuntimeStatus {
    Starting,
    Running,
    Stopped,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserRuntimeCreateRequest {
    pub mux_session_id: String,
    pub browser_resource_id: String,
    #[serde(default = "default_url")]
    pub url: String,
    #[serde(default)]
    pub temporary_profile: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BrowserRuntimeEvent {
    Started {
        runtime: BrowserRuntime,
    },
    Status {
        runtime_id: String,
        status: BrowserRuntimeStatus,
        error: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserMouseEvent {
    pub event_type: String,
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub button: String,
    #[serde(default)]
    pub buttons: i64,
    #[serde(default)]
    pub delta_x: f64,
    #[serde(default)]
    pub delta_y: f64,
    #[serde(default)]
    pub modifiers: i64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserKeyEvent {
    pub event_type: String,
    pub key: String,
    pub code: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub modifiers: i64,
}

fn default_url() -> String {
    "about:blank".into()
}

struct ManagedBrowserRuntime {
    summary: BrowserRuntime,
    child: Child,
    temporary_profile: bool,
    cdp_tx: mpsc::UnboundedSender<CdpCommand>,
}

struct CdpCommand {
    payload: Value,
    response: Option<oneshot::Sender<Result<Value, String>>>,
}

struct PageTarget {
    websocket_url: String,
}

type CdpSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

type BrowserRuntimeEventSink = Arc<dyn Fn(BrowserRuntimeEvent) + Send + Sync>;

struct ReservedCdpEndpoint {
    port: u16,
    reservation: Option<TcpListener>,
}

pub struct BrowserRuntimeManager {
    event_sink: BrowserRuntimeEventSink,
    profiles_root: PathBuf,
    runtimes: Mutex<HashMap<String, ManagedBrowserRuntime>>,
    start_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    session_endpoints: Mutex<HashMap<String, ReservedCdpEndpoint>>,
    registry_path: PathBuf,
}

impl BrowserRuntimeManager {
    pub fn new(app_handle: tauri::AppHandle, data_dir: &Path) -> Arc<Self> {
        let event_sink: BrowserRuntimeEventSink = Arc::new(move |event| {
            let _ = app_handle.emit("browser-runtime:event", event);
        });
        let profiles_root = data_dir.join("browser-profiles");
        let registry_path = data_dir.join("browser-runtimes.json");
        #[cfg(target_os = "macos")]
        {
            cleanup_stale_managed_chrome(&profiles_root);
            let _ = std::fs::remove_file(&registry_path);
        }
        Arc::new(Self {
            event_sink,
            profiles_root,
            runtimes: Mutex::new(HashMap::new()),
            start_locks: Mutex::new(HashMap::new()),
            session_endpoints: Mutex::new(HashMap::new()),
            registry_path,
        })
    }

    #[cfg(test)]
    fn new_for_test(data_dir: &Path) -> Arc<Self> {
        Arc::new(Self {
            event_sink: Arc::new(|_| {}),
            profiles_root: data_dir.join("browser-profiles"),
            runtimes: Mutex::new(HashMap::new()),
            start_locks: Mutex::new(HashMap::new()),
            session_endpoints: Mutex::new(HashMap::new()),
            registry_path: data_dir.join("browser-runtimes.json"),
        })
    }

    pub fn discover_chrome(&self) -> Option<ChromeInstallation> {
        discover_chrome()
    }

    pub fn registry_path(&self) -> String {
        self.registry_path.to_string_lossy().into_owned()
    }

    fn resource_profile_path(&self, mux_session_id: &str, browser_resource_id: &str) -> PathBuf {
        self.profiles_root
            .join("sessions")
            .join(mux_session_id)
            .join(browser_resource_id)
    }

    pub fn session_cdp_port(&self, mux_session_id: &str) -> Result<u16, String> {
        let mux_session_id = validate_id("muxSessionId", mux_session_id)?;
        if let Some(port) = self
            .runtimes
            .lock()
            .map_err(|_| "浏览器 Runtime 状态已损坏".to_string())?
            .values()
            .find(|runtime| {
                runtime.summary.mux_session_id == mux_session_id
                    && runtime.summary.status == BrowserRuntimeStatus::Running
            })
            .map(|runtime| runtime.summary.cdp_port)
        {
            return Ok(port);
        }
        let mut endpoints = self
            .session_endpoints
            .lock()
            .map_err(|_| "浏览器 Session 端点状态已损坏".to_string())?;
        if let Some(endpoint) = endpoints.get(&mux_session_id) {
            return Ok(endpoint.port);
        }
        let listener = reserve_loopback_listener()?;
        let port = listener
            .local_addr()
            .map_err(|error| format!("无法读取 Chrome CDP 端口: {error}"))?
            .port();
        endpoints.insert(
            mux_session_id,
            ReservedCdpEndpoint {
                port,
                reservation: Some(listener),
            },
        );
        Ok(port)
    }

    pub async fn create(
        self: &Arc<Self>,
        request: BrowserRuntimeCreateRequest,
    ) -> Result<BrowserRuntime, String> {
        let mux_session_id = validate_id("muxSessionId", &request.mux_session_id)?;
        let browser_resource_id = validate_id("browserResourceId", &request.browser_resource_id)?;
        let start_lock = self
            .start_locks
            .lock()
            .map_err(|_| "浏览器 Runtime 启动锁已损坏".to_string())?
            .entry(mux_session_id.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _start_guard = start_lock.lock().await;
        let stale_runtime = {
            let mut runtimes = self
                .runtimes
                .lock()
                .map_err(|_| "浏览器 Runtime 状态已损坏".to_string())?;
            let existing = runtimes.iter_mut().find_map(|(id, runtime)| {
                (runtime.summary.browser_resource_id == browser_resource_id).then(|| {
                    let running = !matches!(runtime.child.try_wait(), Ok(Some(_)));
                    (id.clone(), running, runtime.summary.clone())
                })
            });
            match existing {
                Some((_, true, summary)) => return Ok(summary),
                Some((id, false, _)) => runtimes.remove(&id),
                None => None,
            }
        };
        if let Some(runtime) = stale_runtime
            && runtime.temporary_profile
        {
            let _ = std::fs::remove_dir_all(runtime.summary.profile_path);
        }
        let another_runtime = self
            .runtimes
            .lock()
            .map_err(|_| "浏览器 Runtime 状态已损坏".to_string())?
            .values_mut()
            .find_map(|runtime| {
                if runtime.summary.mux_session_id != mux_session_id
                    || runtime.summary.browser_resource_id == browser_resource_id
                {
                    return None;
                }
                (!matches!(runtime.child.try_wait(), Ok(Some(_)))).then(|| runtime.summary.clone())
            });
        if another_runtime.is_some() {
            return Err(
                "当前会话已有其他正在运行的浏览器。请先停止当前浏览器，再启动这个浏览器。".into(),
            );
        }
        let installation = discover_chrome().ok_or_else(|| {
            "未找到 Google Chrome。请先在本机安装 Chrome 后再创建浏览器资源。".to_string()
        })?;
        let url = normalize_url(&request.url)?;
        let runtime_id = Uuid::new_v4().to_string();
        let profile_path = if request.temporary_profile {
            self.profiles_root.join("temporary").join(&runtime_id)
        } else {
            self.resource_profile_path(&mux_session_id, &browser_resource_id)
        };
        std::fs::create_dir_all(&profile_path).map_err(|error| {
            format!(
                "无法创建 Chrome 配置目录 {}: {error}",
                profile_path.display()
            )
        })?;
        mark_chrome_profile_clean(&profile_path)?;
        let port = self.take_session_cdp_port(&mux_session_id)?;
        let mut command = Command::new(&installation.executable_path);
        command
            .arg(format!("--remote-debugging-port={port}"))
            .arg("--remote-debugging-address=127.0.0.1")
            .arg(format!("--user-data-dir={}", profile_path.display()))
            .arg("--new-window")
            // chrome://newtab is not an agent-browser controllable target.
            // about:blank gives the Agent one reusable tab without a request.
            .arg(&url)
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-background-mode")
            .arg("--disable-session-crashed-bubble")
            .arg("--hide-crash-restore-bubble")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        configure_process_group(&mut command);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                self.reserve_session_cdp_port(&mux_session_id, port);
                return Err(format!("无法启动 Chrome: {error}"));
            }
        };
        let process_id = child.id();
        let mut summary = BrowserRuntime {
            id: runtime_id.clone(),
            mux_session_id: mux_session_id.clone(),
            browser_resource_id,
            url: url.clone(),
            cdp_port: port,
            profile_path: profile_path.to_string_lossy().into_owned(),
            process_id,
            status: BrowserRuntimeStatus::Starting,
            error: None,
        };
        if let Err(error) = wait_for_cdp(port, Duration::from_secs(8)).await {
            cleanup_failed_browser_start(&mut child, &profile_path, request.temporary_profile);
            summary.status = BrowserRuntimeStatus::Error;
            summary.error = Some(error.clone());
            self.reserve_session_cdp_port(&mux_session_id, port);
            return Err(error);
        }
        let websocket_url = match page_websocket_url(port, &url).await {
            Ok(websocket_url) => websocket_url,
            Err(error) => {
                cleanup_failed_browser_start(&mut child, &profile_path, request.temporary_profile);
                self.reserve_session_cdp_port(&mux_session_id, port);
                return Err(error);
            }
        };
        let cdp_socket = match connect_async(&websocket_url).await {
            Ok((socket, _)) => socket,
            Err(error) => {
                cleanup_failed_browser_start(&mut child, &profile_path, request.temporary_profile);
                self.reserve_session_cdp_port(&mux_session_id, port);
                return Err(format!("无法连接 Chrome CDP: {error}"));
            }
        };
        let (cdp_tx, cdp_rx) = mpsc::unbounded_channel();
        summary.status = BrowserRuntimeStatus::Running;
        self.runtimes
            .lock()
            .map_err(|_| "浏览器 Runtime 状态已损坏".to_string())?
            .insert(
                runtime_id,
                ManagedBrowserRuntime {
                    summary: summary.clone(),
                    child,
                    temporary_profile: request.temporary_profile,
                    cdp_tx,
                },
            );
        self.write_registry();
        (self.event_sink)(BrowserRuntimeEvent::Started {
            runtime: summary.clone(),
        });
        start_cdp_runtime(
            Arc::downgrade(self),
            self.event_sink.clone(),
            summary.clone(),
            cdp_socket,
            cdp_rx,
        );
        Ok(summary)
    }

    pub fn list(&self) -> Result<Vec<BrowserRuntime>, String> {
        let mut guard = self
            .runtimes
            .lock()
            .map_err(|_| "浏览器 Runtime 状态已损坏".to_string())?;
        let mut items = Vec::with_capacity(guard.len());
        for runtime in guard.values_mut() {
            if matches!(runtime.child.try_wait(), Ok(Some(_))) {
                runtime.summary.status = BrowserRuntimeStatus::Stopped;
            }
            items.push(runtime.summary.clone());
        }
        drop(guard);
        self.write_registry();
        Ok(items)
    }

    pub async fn close(&self, runtime_id: &str) -> Result<(), String> {
        let runtime = self
            .runtimes
            .lock()
            .map_err(|_| "浏览器 Runtime 状态已损坏".to_string())?
            .remove(runtime_id);
        if let Some(runtime) = runtime {
            let mux_session_id = runtime.summary.mux_session_id.clone();
            let port = runtime.summary.cdp_port;
            close_agent_browser_session(&mux_session_id).await;
            close_managed_runtime(runtime).await;
            self.reserve_session_cdp_port(&mux_session_id, port);
        }
        self.write_registry();
        Ok(())
    }

    pub async fn close_all(&self) {
        let runtimes = self
            .runtimes
            .lock()
            .map(|mut guard| {
                guard
                    .drain()
                    .map(|(_, runtime)| runtime)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for runtime in runtimes {
            close_agent_browser_session(&runtime.summary.mux_session_id).await;
            close_managed_runtime(runtime).await;
        }
        self.write_registry();
    }

    pub fn force_cleanup_managed_processes(&self) {
        #[cfg(target_os = "macos")]
        {
            cleanup_stale_managed_chrome(&self.profiles_root);
            let _ = std::fs::remove_file(&self.registry_path);
        }
    }

    fn write_registry(&self) {
        let entries = self
            .runtimes
            .lock()
            .map(|runtimes| {
                runtimes
                    .values()
                    .filter(|runtime| runtime.summary.status == BrowserRuntimeStatus::Running)
                    .map(|runtime| BrowserRuntimeRegistryEntry {
                        mux_session_id: runtime.summary.mux_session_id.clone(),
                        runtime_id: runtime.summary.id.clone(),
                        cdp_port: runtime.summary.cdp_port,
                        process_id: runtime.summary.process_id,
                        status: runtime.summary.status.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let _ = std::fs::create_dir_all(
            self.registry_path
                .parent()
                .unwrap_or_else(|| Path::new(".")),
        );
        if entries.is_empty() {
            let _ = std::fs::remove_file(&self.registry_path);
        } else if let Ok(contents) = serde_json::to_vec(&entries) {
            let _ = std::fs::write(&self.registry_path, contents);
        }
    }

    fn take_session_cdp_port(&self, mux_session_id: &str) -> Result<u16, String> {
        let port = self.session_cdp_port(mux_session_id)?;
        if let Some(endpoint) = self
            .session_endpoints
            .lock()
            .map_err(|_| "浏览器 Session 端点状态已损坏".to_string())?
            .get_mut(mux_session_id)
        {
            endpoint.reservation.take();
        }
        Ok(port)
    }

    fn reserve_session_cdp_port(&self, mux_session_id: &str, port: u16) {
        let reservation =
            TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)).ok();
        if let Ok(mut endpoints) = self.session_endpoints.lock() {
            endpoints.insert(
                mux_session_id.to_string(),
                ReservedCdpEndpoint { port, reservation },
            );
        }
    }

    pub fn navigate(&self, runtime_id: &str, url: &str) -> Result<(), String> {
        let url = normalize_url(url)?;
        self.send_cdp(runtime_id, "Page.navigate", json!({"url": url}))
    }

    pub fn focus_external_window(&self, runtime_id: &str) -> Result<(), String> {
        let process_id = self
            .runtimes
            .lock()
            .map_err(|_| "浏览器 Runtime 状态已损坏".to_string())?
            .get(runtime_id)
            .map(|runtime| runtime.summary.process_id)
            .ok_or_else(|| "Browser Runtime 不存在或已经停止".to_string())?;
        focus_chrome_window(process_id)
    }

    pub fn resize(&self, runtime_id: &str, width: u32, height: u32) -> Result<(), String> {
        let width = width.clamp(320, 3840);
        let height = height.clamp(200, 2160);
        self.send_cdp(
            runtime_id,
            "Emulation.setDeviceMetricsOverride",
            json!({"width": width, "height": height, "deviceScaleFactor": 1, "mobile": false}),
        )
    }

    pub fn mouse(&self, runtime_id: &str, event: BrowserMouseEvent) -> Result<(), String> {
        if !matches!(
            event.event_type.as_str(),
            "mousePressed" | "mouseReleased" | "mouseMoved" | "mouseWheel"
        ) {
            return Err("不支持的浏览器鼠标事件".into());
        }
        self.send_cdp(
            runtime_id,
            "Input.dispatchMouseEvent",
            json!({
                "type": event.event_type,
                "x": event.x.max(0.0),
                "y": event.y.max(0.0),
                "button": if event.button.is_empty() { "none" } else { &event.button },
                "buttons": event.buttons,
                "deltaX": event.delta_x,
                "deltaY": event.delta_y,
                "modifiers": event.modifiers
            }),
        )
    }

    pub fn key(&self, runtime_id: &str, event: BrowserKeyEvent) -> Result<(), String> {
        if !matches!(event.event_type.as_str(), "keyDown" | "keyUp" | "char") {
            return Err("不支持的浏览器键盘事件".into());
        }
        self.send_cdp(
            runtime_id,
            "Input.dispatchKeyEvent",
            json!({
                "type": event.event_type,
                "key": event.key,
                "code": event.code,
                "text": event.text,
                "unmodifiedText": event.text,
                "modifiers": event.modifiers
            }),
        )
    }

    pub fn insert_text(&self, runtime_id: &str, text: &str) -> Result<(), String> {
        if text.len() > 256 * 1024 {
            return Err("浏览器输入文本超过 256 KiB 限制".into());
        }
        self.send_cdp(runtime_id, "Input.insertText", json!({"text": text}))
    }

    pub async fn click(&self, runtime_id: &str, selector: &str) -> Result<bool, String> {
        let selector = validate_selector(selector)?;
        let encoded = serde_json::to_string(selector).map_err(|error| error.to_string())?;
        let result = self
            .evaluate(
                runtime_id,
                &format!(
                    "(() => {{ const element = document.querySelector({encoded}); if (!element) return false; element.scrollIntoView({{ block: 'center', inline: 'center' }}); element.click(); return true; }})()"
                ),
            )
            .await?;
        Ok(result.get("value").and_then(Value::as_bool) == Some(true))
    }

    pub async fn type_text(
        &self,
        runtime_id: &str,
        selector: &str,
        text: &str,
        clear: bool,
    ) -> Result<bool, String> {
        let selector = validate_selector(selector)?;
        if text.len() > 256 * 1024 {
            return Err("浏览器输入文本超过 256 KiB 限制".into());
        }
        let encoded = serde_json::to_string(selector).map_err(|error| error.to_string())?;
        let focus = self
            .evaluate(
                runtime_id,
                &format!(
                    "(() => {{ const element = document.querySelector({encoded}); if (!element) return false; element.scrollIntoView({{ block: 'center', inline: 'center' }}); element.focus(); {} return true; }})()",
                    if clear {
                        "if ('value' in element) { const prototype = Object.getPrototypeOf(element); const setter = Object.getOwnPropertyDescriptor(prototype, 'value')?.set; if (setter) setter.call(element, ''); else element.value = ''; element.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'deleteContentBackward', data: null })); } else if (element.isContentEditable) { element.textContent = ''; element.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'deleteContentBackward', data: null })); }"
                    } else {
                        ""
                    }
                ),
            )
            .await?;
        if focus.get("value").and_then(Value::as_bool) != Some(true) {
            return Ok(false);
        }
        self.insert_text(runtime_id, text)?;
        Ok(true)
    }

    pub fn press(&self, runtime_id: &str, shortcut: &str) -> Result<(), String> {
        let (key, code, text, modifiers, virtual_key_code) = parse_shortcut(shortcut)?;
        for event_type in ["rawKeyDown", "keyUp"] {
            self.send_cdp(
                runtime_id,
                "Input.dispatchKeyEvent",
                json!({
                    "type": event_type,
                    "key": key,
                    "code": code,
                    "text": if event_type == "rawKeyDown" { text.as_str() } else { "" },
                    "unmodifiedText": if event_type == "rawKeyDown" { text.as_str() } else { "" },
                    "modifiers": modifiers,
                    "windowsVirtualKeyCode": virtual_key_code,
                    "nativeVirtualKeyCode": virtual_key_code
                }),
            )?;
        }
        Ok(())
    }

    pub fn scroll(
        &self,
        runtime_id: &str,
        delta_x: f64,
        delta_y: f64,
        x: f64,
        y: f64,
    ) -> Result<(), String> {
        if ![delta_x, delta_y, x, y]
            .iter()
            .all(|value| value.is_finite())
        {
            return Err("浏览器滚动参数必须是有限数字".into());
        }
        if delta_x == 0.0 && delta_y == 0.0 {
            return Err("浏览器滚动距离不能同时为 0".into());
        }
        self.send_cdp(
            runtime_id,
            "Input.dispatchMouseEvent",
            json!({
                "type": "mouseWheel",
                "x": x.max(0.0),
                "y": y.max(0.0),
                "deltaX": delta_x.clamp(-100_000.0, 100_000.0),
                "deltaY": delta_y.clamp(-100_000.0, 100_000.0)
            }),
        )
    }

    pub async fn screenshot(&self, runtime_id: &str) -> Result<String, String> {
        let result = self
            .call_cdp(
                runtime_id,
                "Page.captureScreenshot",
                json!({"format": "png", "fromSurface": true, "captureBeyondViewport": false}),
            )
            .await?;
        let data = result
            .get("data")
            .and_then(Value::as_str)
            .ok_or_else(|| "Chrome 截图响应缺少图像数据".to_string())?;
        BASE64
            .decode(data)
            .map_err(|error| format!("Chrome 截图数据无效: {error}"))?;
        Ok(format!("data:image/png;base64,{data}"))
    }

    pub async fn snapshot(&self, runtime_id: &str) -> Result<Value, String> {
        self.call_cdp(
            runtime_id,
            "Accessibility.getFullAXTree",
            json!({"depth": 12}),
        )
        .await
    }

    pub async fn evaluate(&self, runtime_id: &str, expression: &str) -> Result<Value, String> {
        if expression.len() > 256 * 1024 {
            return Err("浏览器表达式超过 256 KiB 限制".into());
        }
        let result = self
            .call_cdp(
                runtime_id,
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "awaitPromise": true,
                    "returnByValue": true,
                    "userGesture": true
                }),
            )
            .await?;
        if let Some(exception) = result.get("exceptionDetails") {
            return Err(format!("页面脚本执行失败: {exception}"));
        }
        Ok(result.get("result").cloned().unwrap_or(Value::Null))
    }

    pub async fn wait_for_selector(
        &self,
        runtime_id: &str,
        selector: &str,
        timeout_ms: u64,
    ) -> Result<bool, String> {
        let selector = validate_selector(selector)?;
        let deadline = Instant::now() + Duration::from_millis(timeout_ms.clamp(100, 30_000));
        let encoded = serde_json::to_string(selector).map_err(|error| error.to_string())?;
        while Instant::now() < deadline {
            let result = self
                .evaluate(
                    runtime_id,
                    &format!("Boolean(document.querySelector({encoded}))"),
                )
                .await?;
            if result.get("value").and_then(Value::as_bool) == Some(true) {
                return Ok(true);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(false)
    }

    fn send_cdp(&self, runtime_id: &str, method: &str, params: Value) -> Result<(), String> {
        let guard = self
            .runtimes
            .lock()
            .map_err(|_| "浏览器 Runtime 状态已损坏".to_string())?;
        let runtime = guard
            .get(runtime_id)
            .ok_or_else(|| "Browser Runtime 不存在或已经停止".to_string())?;
        runtime
            .cdp_tx
            .send(CdpCommand {
                payload: json!({"id": next_command_id(), "method": method, "params": params}),
                response: None,
            })
            .map_err(|_| "Chrome CDP 连接已经关闭".to_string())
    }

    async fn call_cdp(
        &self,
        runtime_id: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, String> {
        let id = next_command_id();
        let (response_tx, response_rx) = oneshot::channel();
        {
            let guard = self
                .runtimes
                .lock()
                .map_err(|_| "浏览器 Runtime 状态已损坏".to_string())?;
            let runtime = guard
                .get(runtime_id)
                .ok_or_else(|| "Browser Runtime 不存在或已经停止".to_string())?;
            runtime
                .cdp_tx
                .send(CdpCommand {
                    payload: json!({"id": id, "method": method, "params": params}),
                    response: Some(response_tx),
                })
                .map_err(|_| "Chrome CDP 连接已经关闭".to_string())?;
        }
        tokio::time::timeout(Duration::from_secs(10), response_rx)
            .await
            .map_err(|_| format!("Chrome CDP 操作 {method} 超时"))?
            .map_err(|_| "Chrome CDP 响应通道已经关闭".to_string())?
    }
}

impl Drop for BrowserRuntimeManager {
    fn drop(&mut self) {
        let Ok(runtimes) = self.runtimes.get_mut() else {
            return;
        };
        for (_, mut runtime) in runtimes.drain() {
            if !matches!(runtime.child.try_wait(), Ok(Some(_))) {
                close_process_tree(&mut runtime.child);
                let _ = runtime.child.kill();
            }
            let profile_path = PathBuf::from(&runtime.summary.profile_path);
            let _ = runtime.child.wait();
            if runtime.temporary_profile {
                let _ = std::fs::remove_dir_all(profile_path);
            } else {
                let _ = mark_chrome_profile_clean(&profile_path);
            }
        }
    }
}

pub fn try_run_mcp_browser(args: &[String]) -> Option<i32> {
    if args.get(1).map(String::as_str) != Some("mcp")
        || !matches!(args.get(2).map(String::as_str), Some("browser" | "chrome"))
    {
        return None;
    }
    let availability_probe = args.get(3).map(String::as_str) == Some("available");
    let session_id = match std::env::var("LUNA_MUX_SESSION_ID") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            if !availability_probe {
                eprintln!(
                    "Luna Mux Browser MCP 需要在 Luna Mux 终端中启动，并继承 LUNA_MUX_SESSION_ID。\n"
                );
            }
            return Some(1);
        }
    };
    let reserved_port = std::env::var("LUNA_MUX_BROWSER_CDP_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|port| *port > 0);
    let registry_path = browser_registry_path();
    let entries = std::fs::read_to_string(&registry_path)
        .ok()
        .and_then(|contents| {
            serde_json::from_str::<Vec<BrowserRuntimeRegistryEntry>>(&contents).ok()
        })
        .unwrap_or_default();
    let matching = entries
        .into_iter()
        .filter(|entry| {
            entry.mux_session_id == session_id && entry.status == BrowserRuntimeStatus::Running
        })
        .collect::<Vec<_>>();
    if matching.len() > 1 {
        if !availability_probe {
            eprintln!(
                "当前 Session 同时运行了多个 Chrome。请只保留一个 Browser Runtime 后重试。\n"
            );
        }
        return Some(1);
    }
    let cdp_port = matching
        .first()
        .map(|runtime| runtime.cdp_port)
        .or(reserved_port);
    let Some(cdp_port) = cdp_port else {
        if !availability_probe {
            eprintln!("当前终端没有 Luna Mux Browser CDP 端点。请在 Luna Mux 中重新启动此终端。\n");
        }
        return Some(1);
    };
    let agent_browser = match resolve_agent_browser_binary() {
        Ok(path) => path,
        Err(error) => {
            if !availability_probe {
                eprintln!("Luna Mux 无法启动 Browser MCP：{error}");
            }
            return Some(1);
        }
    };
    if availability_probe {
        return Some(0);
    }
    let scope = agent_browser_scope(&session_id);
    let config_path = match create_agent_browser_mcp_config(&scope, cdp_port) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("Luna Mux 无法创建 Browser MCP 配置：{error}");
            return Some(1);
        }
    };
    let mut command = Command::new(&agent_browser);
    configure_agent_browser_command(&mut command);
    command
        .arg("mcp")
        .args(["--tools", AGENT_BROWSER_TOOLS])
        .env("AGENT_BROWSER_CONFIG", &config_path)
        .env("AGENT_BROWSER_SESSION", &scope)
        .env("AGENT_BROWSER_NAMESPACE", &scope)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let status = command.status();
    let _ = std::fs::remove_file(&config_path);
    match status {
        Ok(status) => Some(status.code().unwrap_or(1)),
        Err(error) => {
            eprintln!(
                "无法通过 {} 启动 agent-browser MCP: {error}",
                agent_browser.display()
            );
            Some(1)
        }
    }
}

const REMOTE_MCP_BRIDGE_PREAMBLE: &str = "LUNA_MUX_BROWSER_MCP_V1 ";

pub(crate) async fn bridge_remote_agent_browser_mcp<S>(
    mut stream: S,
    expected_token: &str,
    mux_session_id: &str,
    cdp_port: u16,
) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut preamble = Vec::with_capacity(128);
    loop {
        if preamble.len() >= 512 {
            return Err("远程 Browser MCP 认证信息过长".into());
        }
        let byte = tokio::time::timeout(Duration::from_secs(5), stream.read_u8())
            .await
            .map_err(|_| "远程 Browser MCP 认证超时".to_string())?
            .map_err(|error| error.to_string())?;
        if byte == b'\n' {
            break;
        }
        preamble.push(byte);
    }
    let expected = format!("{REMOTE_MCP_BRIDGE_PREAMBLE}{expected_token}");
    if preamble != expected.as_bytes() {
        return Err("远程 Browser MCP 认证失败".into());
    }

    let binary = resolve_agent_browser_binary()?;
    let scope = agent_browser_scope(mux_session_id);
    let config_path = create_agent_browser_mcp_config(&scope, cdp_port)?;
    let mut command = Command::new(binary);
    configure_agent_browser_command(&mut command);
    command
        .arg("mcp")
        .args(["--tools", AGENT_BROWSER_TOOLS])
        .env("AGENT_BROWSER_CONFIG", &config_path)
        .env("AGENT_BROWSER_SESSION", &scope)
        .env("AGENT_BROWSER_NAMESPACE", &scope)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = match tokio::process::Command::from(command).spawn() {
        Ok(child) => child,
        Err(error) => {
            let _ = std::fs::remove_file(config_path);
            return Err(format!("无法启动远程 agent-browser MCP：{error}"));
        }
    };
    let mut child_stdin = child
        .stdin
        .take()
        .ok_or_else(|| "无法打开远程 agent-browser MCP 输入".to_string())?;
    let mut child_stdout = child
        .stdout
        .take()
        .ok_or_else(|| "无法打开远程 agent-browser MCP 输出".to_string())?;
    let (mut remote_read, mut remote_write) = tokio::io::split(stream);
    let upstream = async {
        let result = tokio::io::copy(&mut remote_read, &mut child_stdin).await;
        let _ = child_stdin.shutdown().await;
        result
    };
    let downstream = async {
        let result = tokio::io::copy(&mut child_stdout, &mut remote_write).await;
        let _ = remote_write.shutdown().await;
        result
    };
    tokio::pin!(upstream);
    tokio::pin!(downstream);
    let result = tokio::select! {
        result = &mut upstream => result.map(|_| ()),
        result = &mut downstream => result.map(|_| ()),
    };
    let _ = child.kill().await;
    let _ = child.wait().await;
    let _ = std::fs::remove_file(config_path);
    result.map_err(|error| format!("远程 Browser MCP 桥接失败：{error}"))
}

#[cfg(test)]
mod remote_mcp_bridge_tests {
    use super::bridge_remote_agent_browser_mcp;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn remote_bridge_rejects_the_wrong_token_before_starting_a_sidecar() {
        let (mut client, server) = tokio::io::duplex(1024);
        let request = tokio::spawn(async move {
            client
                .write_all(b"LUNA_MUX_BROWSER_MCP_V1 wrong-token\n")
                .await
                .unwrap();
        });
        let error = bridge_remote_agent_browser_mcp(server, "expected-token", "session-1", 43129)
            .await
            .unwrap_err();
        request.await.unwrap();
        assert_eq!(error, "远程 Browser MCP 认证失败");
    }
}

fn resolve_agent_browser_binary() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("LUNA_MUX_AGENT_BROWSER_PATH")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return path
            .is_file()
            .then_some(path.clone())
            .ok_or_else(|| format!("显式配置的 agent-browser 路径不存在：{}", path.display()));
    }

    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let runtime_name = if cfg!(windows) {
        "agent-browser.exe"
    } else {
        "agent-browser"
    };
    let source_name = agent_browser_sidecar_source_name();
    let mut candidates = Vec::new();
    if let Some(parent) = executable.parent() {
        candidates.push(parent.join(runtime_name));
        candidates.push(parent.join(source_name));
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("binaries")
            .join(source_name),
    );
    if let Some(path) = candidates.into_iter().find(|path| path.is_file()) {
        return Ok(path);
    }

    #[cfg(windows)]
    let discovered = {
        let mut discovery_command = Command::new("where.exe");
        configure_agent_browser_command(&mut discovery_command);
        discovery_command
            .arg("agent-browser.exe")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| first_existing_path(&output.stdout))
    };
    #[cfg(not(windows))]
    let discovered = Command::new("which")
        .arg("agent-browser")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| first_existing_path(&output.stdout));
    discovered
        .ok_or_else(|| "未找到原生 agent-browser sidecar；请重新运行应用构建准备步骤".to_string())
}

fn first_existing_path(output: &[u8]) -> Option<PathBuf> {
    String::from_utf8_lossy(output)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .find(|path| path.is_file())
}

fn agent_browser_sidecar_source_name() -> &'static str {
    #[cfg(all(windows, target_arch = "x86_64"))]
    return "agent-browser-x86_64-pc-windows-msvc.exe";
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return "agent-browser-x86_64-apple-darwin";
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return "agent-browser-aarch64-apple-darwin";
    #[allow(unreachable_code)]
    "agent-browser"
}

#[cfg(not(windows))]
fn configure_agent_browser_command(command: &mut Command) {
    command.env("AGENT_BROWSER_SOCKET_DIR", AGENT_BROWSER_SOCKET_DIR);
}

#[cfg(windows)]
fn configure_agent_browser_command(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    command.creation_flags(CREATE_NO_WINDOW);
}

fn agent_browser_scope(mux_session_id: &str) -> String {
    #[cfg(windows)]
    {
        let safe = mux_session_id
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>();
        format!("luna-mux-{safe}")
    }
    #[cfg(not(windows))]
    {
        use sha2::{Digest, Sha256};

        let digest = Sha256::digest(mux_session_id.as_bytes());
        format!(
            "lm-{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&digest[..12])
        )
    }
}

fn create_agent_browser_mcp_config(scope: &str, cdp_port: u16) -> Result<PathBuf, String> {
    let root = std::env::temp_dir().join("luna-mux").join("agent-browser");
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let path = root.join(format!("{scope}-{}.json", Uuid::new_v4()));
    let config = AgentBrowserMcpConfig {
        cdp: cdp_port.to_string(),
        content_boundaries: true,
        max_output: 50_000,
        pin_tab: true,
    };
    let contents = serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?;
    std::fs::write(&path, contents).map_err(|error| error.to_string())?;
    Ok(path)
}

pub(crate) async fn warm_agent_browser_session(
    mux_session_id: &str,
    cdp_port: u16,
) -> Result<(), String> {
    let bootstrap_target = sole_blank_page_target(cdp_port).await?;
    let binary = resolve_agent_browser_binary()?;
    let scope = agent_browser_scope(mux_session_id);
    let config_path = create_agent_browser_mcp_config(&scope, cdp_port)?;
    let result = tokio::task::spawn_blocking(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let run_probe = |arguments: &[&str], stage: &str| -> Result<(i32, String), String> {
            let stdout_path = std::env::temp_dir()
                .join(format!("luna-mux-ab-stdout-{}.log", Uuid::new_v4()));
            let stderr_path = std::env::temp_dir()
                .join(format!("luna-mux-ab-stderr-{}.log", Uuid::new_v4()));
            let stdout = std::fs::File::create(&stdout_path)
                .map_err(|error| format!("agent-browser {stage} log create failed: {error}"))?;
            let stderr = std::fs::File::create(&stderr_path)
                .map_err(|error| format!("agent-browser {stage} log create failed: {error}"))?;
            let mut command = Command::new(&binary);
            command
                .args(arguments)
                .env("AGENT_BROWSER_CONFIG", &config_path)
                .env("AGENT_BROWSER_SESSION", &scope)
                .env("AGENT_BROWSER_NAMESPACE", &scope)
                .stdin(Stdio::null())
                .stdout(Stdio::from(stdout))
                .stderr(Stdio::from(stderr));
            configure_agent_browser_command(&mut command);
            let mut child = command
                .spawn()
                .map_err(|error| format!("无法启动 agent-browser {stage}：{error}"))?;
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let stdout_output =
                            std::fs::read_to_string(&stdout_path).unwrap_or_default();
                        let stderr_output =
                            std::fs::read_to_string(&stderr_path).unwrap_or_default();
                        let _ = std::fs::remove_file(&stdout_path);
                        let _ = std::fs::remove_file(&stderr_path);
                        let mut output = stdout_output.trim().to_string();
                        if !stderr_output.trim().is_empty() {
                            if !output.is_empty() {
                                output.push_str("; ");
                            }
                            output.push_str(stderr_output.trim());
                        }
                        return Ok((status.code().unwrap_or(1), output));
                    }
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    Ok(None) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = std::fs::remove_file(&stdout_path);
                        let _ = std::fs::remove_file(&stderr_path);
                        return Err(format!("agent-browser {stage}超时"));
                    }
                    Err(error) => {
                        let _ = std::fs::remove_file(&stdout_path);
                        let _ = std::fs::remove_file(&stderr_path);
                        return Err(format!("无法读取 agent-browser {stage}状态：{error}"));
                    }
                }
            }
        };
        let result = (|| {
            let (initial_code, initial_output) =
                run_probe(&["--json", "get", "url"], "连接预热")?;
            if initial_code == 0 {
                return Ok(());
            }

            // A managed Chrome restart invalidates agent-browser's persisted CDP
            // target ID. Temporarily relax the sticky pin so it adopts an existing
            // page, then immediately restore strict Session-to-tab binding.
            let (rebind_code, rebind_output) = run_probe(
                &["--no-pin-tab", "--json", "get", "url"],
                "旧标签页重新绑定",
            )?;
            if rebind_code != 0 {
                let details = [initial_output, rebind_output]
                    .into_iter()
                    .filter(|output| !output.is_empty())
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(format!(
                    "agent-browser 连接预热失败（初始退出码 {initial_code}，重新绑定退出码 {rebind_code}）{}",
                    if details.is_empty() {
                        String::new()
                    } else {
                        format!("：{details}")
                    }
                ));
            }
            let (repin_code, repin_output) = run_probe(
                &["--pin-tab", "--json", "get", "url"],
                "Session 标签页重新固定",
            )?;
            if repin_code != 0 {
                return Err(format!(
                    "agent-browser 已重新绑定标签页，但重新固定失败，退出码 {repin_code}{}",
                    if repin_output.is_empty() {
                        String::new()
                    } else {
                        format!("：{repin_output}")
                    }
                ));
            }
            Ok(())
        })();
        let _ = std::fs::remove_file(config_path);
        result
    })
    .await
    .map_err(|error| format!("agent-browser 连接预热任务失败：{error}"))?;
    result?;
    if let Some(target_id) = bootstrap_target {
        close_replaced_bootstrap_target(cdp_port, &target_id).await?;
    }
    Ok(())
}

async fn close_agent_browser_session(mux_session_id: &str) {
    let Ok(binary) = resolve_agent_browser_binary() else {
        return;
    };
    let scope = agent_browser_scope(mux_session_id);
    let _ = tokio::task::spawn_blocking(move || {
        let mut command = Command::new(binary);
        configure_agent_browser_command(&mut command);
        command
            .env("AGENT_BROWSER_SESSION", &scope)
            .env("AGENT_BROWSER_NAMESPACE", &scope)
            .arg("close")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    })
    .await;
}

fn browser_registry_path() -> PathBuf {
    if let Some(path) = std::env::var_os("LUNA_MUX_BROWSER_REGISTRY_PATH") {
        return PathBuf::from(path);
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("com.luna.mux")
        .join("browser-runtimes.json")
}

fn validate_selector(selector: &str) -> Result<&str, String> {
    let selector = selector.trim();
    if selector.is_empty() || selector.len() > 4096 {
        Err("浏览器 selector 无效".into())
    } else {
        Ok(selector)
    }
}

fn parse_shortcut(shortcut: &str) -> Result<(String, String, String, i64, u32), String> {
    let parts = shortcut
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let Some(key_part) = parts.last().copied() else {
        return Err("浏览器按键不能为空".into());
    };
    let mut modifiers = 0_i64;
    for modifier in &parts[..parts.len().saturating_sub(1)] {
        match modifier.to_ascii_lowercase().as_str() {
            "alt" | "option" => modifiers |= 1,
            "control" | "ctrl" => modifiers |= 2,
            "meta" | "command" | "cmd" => modifiers |= 4,
            "shift" => modifiers |= 8,
            _ => return Err(format!("不支持的浏览器修饰键: {modifier}")),
        }
    }
    let (key, code, text, virtual_key_code) = match key_part.to_ascii_lowercase().as_str() {
        "enter" | "return" => ("Enter".into(), "Enter".into(), "\r".into(), 13),
        "tab" => ("Tab".into(), "Tab".into(), "".into(), 9),
        "escape" | "esc" => ("Escape".into(), "Escape".into(), "".into(), 27),
        "backspace" => ("Backspace".into(), "Backspace".into(), "".into(), 8),
        "delete" => ("Delete".into(), "Delete".into(), "".into(), 46),
        "arrowleft" | "left" => ("ArrowLeft".into(), "ArrowLeft".into(), "".into(), 37),
        "arrowup" | "up" => ("ArrowUp".into(), "ArrowUp".into(), "".into(), 38),
        "arrowright" | "right" => ("ArrowRight".into(), "ArrowRight".into(), "".into(), 39),
        "arrowdown" | "down" => ("ArrowDown".into(), "ArrowDown".into(), "".into(), 40),
        "home" => ("Home".into(), "Home".into(), "".into(), 36),
        "end" => ("End".into(), "End".into(), "".into(), 35),
        "pageup" => ("PageUp".into(), "PageUp".into(), "".into(), 33),
        "pagedown" => ("PageDown".into(), "PageDown".into(), "".into(), 34),
        "space" => (" ".into(), "Space".into(), " ".into(), 32),
        _ if key_part.chars().count() == 1 => {
            let character = key_part.chars().next().expect("single key");
            let upper = character.to_ascii_uppercase();
            let code = if character.is_ascii_alphabetic() {
                format!("Key{upper}")
            } else if character.is_ascii_digit() {
                format!("Digit{character}")
            } else {
                String::new()
            };
            (
                character.to_string(),
                code,
                if modifiers & (2 | 4) == 0 {
                    character.to_string()
                } else {
                    String::new()
                },
                upper as u32,
            )
        }
        _ => return Err(format!("不支持的浏览器按键: {key_part}")),
    };
    Ok((key, code, text, modifiers, virtual_key_code))
}

async fn close_managed_runtime(mut runtime: ManagedBrowserRuntime) {
    let _ = runtime.cdp_tx.send(CdpCommand {
        payload: json!({"id": next_command_id(), "method": "Browser.close"}),
        response: None,
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    let exited_cleanly = loop {
        match runtime.child.try_wait() {
            Ok(Some(_)) => break true,
            Ok(None) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
            _ => break false,
        }
    };
    if !exited_cleanly {
        close_process_tree(&mut runtime.child);
        let _ = runtime.child.kill();
    }
    let _ = tokio::task::spawn_blocking(move || {
        let profile_path = PathBuf::from(&runtime.summary.profile_path);
        let temporary_profile = runtime.temporary_profile;
        let _ = runtime.child.wait();
        if temporary_profile {
            let _ = std::fs::remove_dir_all(&profile_path);
        } else {
            let _ = mark_chrome_profile_clean(&profile_path);
        }
    })
    .await;
}

fn cleanup_failed_browser_start(child: &mut Child, profile_path: &Path, temporary_profile: bool) {
    close_process_tree(child);
    let _ = child.kill();
    let _ = child.wait();
    if temporary_profile {
        let _ = std::fs::remove_dir_all(profile_path);
    } else {
        let _ = mark_chrome_profile_clean(profile_path);
    }
}

fn mark_chrome_profile_clean(profile_path: &Path) -> Result<(), String> {
    let default_profile = profile_path.join("Default");
    std::fs::create_dir_all(&default_profile).map_err(|error| {
        format!(
            "无法准备 Chrome 默认配置目录 {}: {error}",
            default_profile.display()
        )
    })?;
    let preferences_path = default_profile.join("Preferences");
    let mut preferences = if preferences_path.is_file() {
        let contents = std::fs::read_to_string(&preferences_path).map_err(|error| {
            format!(
                "无法读取 Chrome 配置 {}: {error}",
                preferences_path.display()
            )
        })?;
        serde_json::from_str::<Value>(&contents).map_err(|error| {
            format!(
                "无法解析 Chrome 配置 {}: {error}",
                preferences_path.display()
            )
        })?
    } else {
        json!({})
    };
    let root = preferences
        .as_object_mut()
        .ok_or_else(|| format!("Chrome 配置 {} 不是 JSON 对象", preferences_path.display()))?;
    let profile = root.entry("profile").or_insert_with(|| json!({}));
    if !profile.is_object() {
        *profile = json!({});
    }
    let profile = profile
        .as_object_mut()
        .ok_or_else(|| "无法更新 Chrome 退出状态".to_string())?;
    profile.insert("exit_type".into(), Value::String("Normal".into()));
    profile.insert("exited_cleanly".into(), Value::Bool(true));
    let serialized = serde_json::to_vec(&preferences)
        .map_err(|error| format!("无法序列化 Chrome 配置: {error}"))?;
    std::fs::write(&preferences_path, serialized).map_err(|error| {
        format!(
            "无法更新 Chrome 配置 {}: {error}",
            preferences_path.display()
        )
    })
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) {}

#[cfg(target_os = "macos")]
fn cleanup_stale_managed_chrome(profiles_root: &Path) {
    let Ok(output) = Command::new("ps").args(["-axo", "pid=,command="]).output() else {
        return;
    };
    let process_ids =
        managed_chrome_process_ids(&String::from_utf8_lossy(&output.stdout), profiles_root);
    if process_ids.is_empty() {
        return;
    }

    for process_id in &process_ids {
        terminate_managed_chrome_process(*process_id, libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline && process_ids.iter().any(|pid| process_exists(*pid)) {
        std::thread::sleep(Duration::from_millis(40));
    }
    for process_id in process_ids {
        if process_exists(process_id) {
            terminate_managed_chrome_process(process_id, libc::SIGKILL);
        }
    }
}

#[cfg(target_os = "macos")]
fn managed_chrome_process_ids(process_list: &str, profiles_root: &Path) -> Vec<u32> {
    let profile_prefix = format!("--user-data-dir={}/", profiles_root.to_string_lossy());
    process_list
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let separator = line.find(char::is_whitespace)?;
            let process_id = line[..separator].parse::<u32>().ok()?;
            let command = line[separator..].trim_start();
            (command.contains("/Google Chrome.app/Contents/MacOS/Google Chrome ")
                && command.contains(&profile_prefix))
            .then_some(process_id)
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn terminate_managed_chrome_process(process_id: u32, signal: i32) {
    let Ok(process_id) = i32::try_from(process_id) else {
        return;
    };
    unsafe {
        if libc::killpg(process_id, signal) == -1 {
            let _ = libc::kill(process_id, signal);
        }
    }
}

#[cfg(target_os = "macos")]
fn process_exists(process_id: u32) -> bool {
    let Ok(process_id) = i32::try_from(process_id) else {
        return false;
    };
    unsafe {
        libc::kill(process_id, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

#[cfg(target_os = "windows")]
fn close_process_tree(child: &mut Child) {
    let _ = crate::local_pty_backend::windows_no_window_command("taskkill")
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(target_os = "windows")]
fn focus_chrome_window(process_id: u32) -> Result<(), String> {
    use windows::{
        Win32::{
            Foundation::{HWND, LPARAM},
            UI::WindowsAndMessaging::{
                EnumWindows, GetWindowThreadProcessId, IsWindowVisible, SW_RESTORE,
                SetForegroundWindow, ShowWindow,
            },
        },
        core::BOOL,
    };

    struct Search {
        process_id: u32,
        window: Option<HWND>,
    }
    unsafe extern "system" fn visit(window: HWND, state: LPARAM) -> BOOL {
        let search = unsafe { &mut *(state.0 as *mut Search) };
        let mut owner = 0_u32;
        unsafe { GetWindowThreadProcessId(window, Some(&mut owner)) };
        if owner == search.process_id && unsafe { IsWindowVisible(window) }.as_bool() {
            search.window = Some(window);
            return BOOL(0);
        }
        BOOL(1)
    }

    let mut search = Search {
        process_id,
        window: None,
    };
    let _ = unsafe { EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize)) };
    let window = search
        .window
        .ok_or_else(|| "未找到该 Browser Runtime 的 Chrome 窗口".to_string())?;
    let _ = unsafe { ShowWindow(window, SW_RESTORE) };
    let _ = unsafe { SetForegroundWindow(window) };
    Ok(())
}

#[cfg(target_os = "macos")]
fn focus_chrome_window(process_id: u32) -> Result<(), String> {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};

    let process_id = i32::try_from(process_id).map_err(|_| "Chrome 进程 ID 无效".to_string())?;
    let application = NSRunningApplication::runningApplicationWithProcessIdentifier(process_id)
        .ok_or_else(|| "未找到该 Browser Runtime 的 Chrome 应用实例".to_string())?;
    if application.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows) {
        Ok(())
    } else {
        Err("无法激活该 Browser Runtime 的 Chrome 窗口".into())
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn focus_chrome_window(_process_id: u32) -> Result<(), String> {
    Err("当前平台不支持激活 Chrome 窗口".into())
}

#[cfg(unix)]
fn close_process_tree(child: &mut Child) {
    unsafe {
        libc::killpg(child.id() as i32, libc::SIGKILL);
    }
}

#[cfg(not(any(target_os = "windows", unix)))]
fn close_process_tree(_child: &mut Child) {}

fn next_command_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

fn validate_id(name: &str, value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 128 {
        return Err(format!("{name} 无效"));
    }
    Ok(value.into())
}

fn normalize_url(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("about:blank") {
        return Ok("about:blank".into());
    }
    let with_scheme = if value.contains("://") {
        value.to_string()
    } else {
        format!("http://{value}")
    };
    let parsed =
        url::Url::parse(&with_scheme).map_err(|error| format!("浏览器 URL 无效: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("浏览器资源只允许 http、https 或 about:blank URL".into());
    }
    Ok(parsed.to_string())
}

#[cfg(test)]
fn reserve_loopback_port() -> Result<u16, String> {
    let listener = reserve_loopback_listener()?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| format!("无法读取 Chrome CDP 端口: {error}"))
}

fn reserve_loopback_listener() -> Result<TcpListener, String> {
    TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .map_err(|error| format!("无法分配 Chrome CDP 端口: {error}"))
}

async fn wait_for_cdp(port: u16, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .map_err(|error| format!("无法创建 CDP 检查客户端: {error}"))?;
    let endpoint = format!("http://127.0.0.1:{port}/json/version");
    while Instant::now() < deadline {
        if let Ok(response) = client.get(&endpoint).send().await
            && response.status().is_success()
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
    Err(format!(
        "Chrome 已启动，但 CDP 端口 {port} 未在限定时间内就绪"
    ))
}

async fn page_websocket_url(port: u16, preferred_url: &str) -> Result<String, String> {
    preferred_page_target(port, preferred_url)
        .await
        .map(|target| target.websocket_url)
}

async fn cdp_targets(port: u16) -> Result<Vec<Value>, String> {
    let endpoint = format!("http://127.0.0.1:{port}/json/list");
    reqwest::Client::new()
        .get(endpoint)
        .send()
        .await
        .map_err(|error| format!("无法读取 Chrome CDP 目标: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Chrome CDP 目标请求失败: {error}"))?
        .json::<Vec<Value>>()
        .await
        .map_err(|error| format!("无法解析 Chrome CDP 目标: {error}"))
}

async fn sole_blank_page_target(port: u16) -> Result<Option<String>, String> {
    let pages = cdp_targets(port)
        .await?
        .into_iter()
        .filter(|target| target.get("type").and_then(Value::as_str) == Some("page"))
        .collect::<Vec<_>>();
    if pages.len() != 1 || pages[0].get("url").and_then(Value::as_str) != Some("about:blank") {
        return Ok(None);
    }
    Ok(pages[0]
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string))
}

async fn close_replaced_bootstrap_target(
    port: u16,
    bootstrap_target_id: &str,
) -> Result<(), String> {
    let pages = cdp_targets(port)
        .await?
        .into_iter()
        .filter(|target| target.get("type").and_then(Value::as_str) == Some("page"))
        .collect::<Vec<_>>();
    let bootstrap_remains_blank = pages.iter().any(|target| {
        target.get("id").and_then(Value::as_str) == Some(bootstrap_target_id)
            && target.get("url").and_then(Value::as_str) == Some("about:blank")
    });
    let replacement_exists = pages.iter().any(|target| {
        target
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id != bootstrap_target_id)
    });
    if !bootstrap_remains_blank || !replacement_exists {
        return Ok(());
    }

    reqwest::Client::new()
        .get(format!(
            "http://127.0.0.1:{port}/json/close/{bootstrap_target_id}"
        ))
        .send()
        .await
        .map_err(|error| format!("无法关闭 Chrome 启动标签页: {error}"))?
        .error_for_status()
        .map_err(|error| format!("关闭 Chrome 启动标签页失败: {error}"))?;

    // /json/close acknowledges before Chrome has necessarily removed the
    // target. Waiting here prevents the caller (and the first MCP command)
    // from observing and selecting the soon-to-be-closed bootstrap page.
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        let target_still_exists = cdp_targets(port).await?.iter().any(|target| {
            target.get("type").and_then(Value::as_str) == Some("page")
                && target.get("id").and_then(Value::as_str) == Some(bootstrap_target_id)
        });
        if !target_still_exists {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    Err("Chrome 未能及时关闭启动标签页".into())
}

async fn preferred_page_target(port: u16, preferred_url: &str) -> Result<PageTarget, String> {
    let targets = cdp_targets(port).await?;
    let websocket_url = select_page_websocket_url(&targets, preferred_url)
        .ok_or_else(|| "Chrome 没有提供可控制的页面目标".to_string())?;
    Ok(PageTarget { websocket_url })
}

fn select_page_websocket_url(targets: &[Value], preferred_url: &str) -> Option<String> {
    targets
        .iter()
        .filter(|target| target.get("type").and_then(Value::as_str) == Some("page"))
        .filter_map(|target| {
            let websocket = target.get("webSocketDebuggerUrl")?.as_str()?;
            let url = target
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let priority = if url == preferred_url {
                0
            } else if !url.is_empty()
                && url != "about:blank"
                && !url.starts_with("chrome://")
                && !url.starts_with("chrome-extension://")
                && !url.starts_with("devtools://")
            {
                1
            } else {
                2
            };
            Some((priority, websocket))
        })
        .min_by_key(|(priority, _)| *priority)
        .map(|(_, websocket)| websocket.to_string())
}

fn start_cdp_runtime(
    manager: Weak<BrowserRuntimeManager>,
    event_sink: BrowserRuntimeEventSink,
    runtime: BrowserRuntime,
    initial_socket: CdpSocket,
    mut commands: mpsc::UnboundedReceiver<CdpCommand>,
) {
    tauri::async_runtime::spawn(async move {
        let mut socket = initial_socket;
        loop {
            let connection_closed = drive_cdp_page(&mut commands, socket).await;
            if !connection_closed {
                return;
            }

            let runtime_is_registered = manager.upgrade().is_some_and(|manager| {
                manager
                    .runtimes
                    .lock()
                    .is_ok_and(|runtimes| runtimes.contains_key(&runtime.id))
            });
            if !runtime_is_registered {
                return;
            }

            // Closing or replacing a tab closes its page-level websocket but
            // does not mean the managed Chrome process has stopped. This is a
            // normal part of agent-browser pin recovery, so rebind Luna's CDP
            // controls to another page before declaring the Runtime stopped.
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut replacement = None;
            while Instant::now() < deadline {
                if let Ok(target) = preferred_page_target(runtime.cdp_port, &runtime.url).await
                    && let Ok((candidate, _)) = connect_async(&target.websocket_url).await
                {
                    replacement = Some(candidate);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if let Some(candidate) = replacement {
                socket = candidate;
                continue;
            }

            event_sink(BrowserRuntimeEvent::Status {
                runtime_id: runtime.id.clone(),
                status: BrowserRuntimeStatus::Stopped,
                error: None,
            });
            drop(commands);
            if let Some(manager) = manager.upgrade() {
                let _ = manager.close(&runtime.id).await;
            }
            return;
        }
    });
}

/// Drives one page-level CDP connection. Returns `true` when the page socket
/// disappeared and the caller should try another target, or `false` when the
/// Runtime command channel was intentionally closed.
async fn drive_cdp_page(
    commands: &mut mpsc::UnboundedReceiver<CdpCommand>,
    socket: CdpSocket,
) -> bool {
    let (mut write, mut read) = socket.split();
    for command in [
        json!({"id": next_command_id(), "method": "Page.enable"}),
        json!({"id": next_command_id(), "method": "Runtime.enable"}),
    ] {
        if write
            .send(Message::Text(command.to_string().into()))
            .await
            .is_err()
        {
            return true;
        }
    }
    let mut pending = HashMap::<u64, oneshot::Sender<Result<Value, String>>>::new();
    let reconnect = loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break false };
                let id = command.payload.get("id").and_then(Value::as_u64);
                if write.send(Message::Text(command.payload.to_string().into())).await.is_err() {
                    if let Some(response) = command.response {
                        let _ = response.send(Err("Chrome CDP 页面连接已经关闭".into()));
                    }
                    break true;
                }
                if let (Some(id), Some(response)) = (id, command.response) {
                    pending.insert(id, response);
                }
            }
            message = read.next() => {
                let Some(Ok(message)) = message else { break true };
                let Message::Text(text) = message else { continue };
                let Ok(payload) = serde_json::from_str::<Value>(&text) else { continue };
                if let Some(id) = payload.get("id").and_then(Value::as_u64)
                    && let Some(response) = pending.remove(&id)
                {
                    let result = if let Some(error) = payload.get("error") {
                        Err(format!("Chrome CDP 操作失败: {error}"))
                    } else {
                        Ok(payload.get("result").cloned().unwrap_or(Value::Null))
                    };
                    let _ = response.send(result);
                }
            }
        }
    };
    for (_, response) in pending {
        let _ = response.send(Err("Chrome CDP 页面连接已经关闭".into()));
    }
    reconnect
}

#[cfg(target_os = "windows")]
fn discover_chrome() -> Option<ChromeInstallation> {
    let mut candidates = Vec::new();
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local_app_data).join("Google/Chrome/Application/chrome.exe"));
    }
    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        candidates.push(PathBuf::from(program_files).join("Google/Chrome/Application/chrome.exe"));
    }
    if let Some(program_files_x86) = std::env::var_os("ProgramFiles(x86)") {
        candidates
            .push(PathBuf::from(program_files_x86).join("Google/Chrome/Application/chrome.exe"));
    }
    installation_from_candidates(candidates)
}

#[cfg(target_os = "macos")]
fn discover_chrome() -> Option<ChromeInstallation> {
    let mut candidates = vec![PathBuf::from(
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    )];
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join("Applications/Google Chrome.app/Contents/MacOS/Google Chrome"));
    }
    installation_from_candidates(candidates)
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn discover_chrome() -> Option<ChromeInstallation> {
    installation_from_candidates([
        PathBuf::from("/usr/bin/google-chrome"),
        PathBuf::from("/usr/bin/google-chrome-stable"),
    ])
}

fn installation_from_candidates(
    candidates: impl IntoIterator<Item = PathBuf>,
) -> Option<ChromeInstallation> {
    candidates
        .into_iter()
        .find(|candidate| candidate.is_file())
        .map(|path| {
            let version = chrome_version(&path);
            ChromeInstallation {
                executable_path: path.to_string_lossy().into_owned(),
                version,
            }
        })
}

#[cfg(target_os = "windows")]
fn chrome_version(path: &Path) -> String {
    path.parent()
        .and_then(|directory| std::fs::read_dir(directory).ok())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| {
            let value = entry.file_name().to_string_lossy().into_owned();
            parse_version_parts(&value).map(|parts| (parts, value))
        })
        .max_by(|left, right| left.0.cmp(&right.0))
        .map(|(_, value)| value)
        .unwrap_or_default()
}

#[cfg(target_os = "windows")]
fn parse_version_parts(value: &str) -> Option<Vec<u64>> {
    let parts = value
        .split('.')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    (parts.len() >= 4).then_some(parts)
}

#[cfg(not(target_os = "windows"))]
fn chrome_version(path: &Path) -> String {
    Command::new(path)
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|output| output.trim().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader, Write},
        process::Stdio,
        sync::mpsc as std_mpsc,
        time::Duration,
    };
    #[cfg(windows)]
    use std::process::Command;

    use futures::{SinkExt, StreamExt};
    use serde_json::{Value, json};
    use tokio_tungstenite::{connect_async, tungstenite::Message};
    use uuid::Uuid;

    use super::{
        BrowserRuntimeCreateRequest, BrowserRuntimeManager, BrowserRuntimeStatus,
        BrowserWarmupGate, close_agent_browser_session, close_process_tree,
        configure_process_group, create_agent_browser_mcp_config, discover_chrome,
        mark_chrome_profile_clean, normalize_url, page_websocket_url, parse_shortcut,
        reserve_loopback_port, resolve_agent_browser_binary, select_page_websocket_url,
        wait_for_cdp, warm_agent_browser_session,
    };

    #[tokio::test]
    async fn browser_warmup_gate_runs_once_per_runtime_and_retries_failures() {
        let gate = BrowserWarmupGate::default();
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let first_attempts = attempts.clone();
        gate.warm_once("session-1", "runtime-1", || async move {
            first_attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap();
        gate.warm_once("session-1", "runtime-1", || async {
            panic!("the same runtime must not be warmed twice")
        })
        .await
        .unwrap();

        let restarted_attempts = attempts.clone();
        gate.warm_once("session-1", "runtime-2", || async move {
            restarted_attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);

        let failed_attempts = attempts.clone();
        assert!(
            gate.warm_once("session-2", "runtime-3", || async move {
                failed_attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err("probe failed".into())
            })
            .await
            .is_err()
        );
        let retry_attempts = attempts.clone();
        gate.warm_once("session-2", "runtime-3", || async move {
            retry_attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 4);
    }

    #[cfg(windows)]
    use super::{configure_agent_browser_command, parse_version_parts};

    #[cfg(windows)]
    #[test]
    fn agent_browser_commands_preserve_piped_output_without_a_console_window() {
        let mut command = Command::new("cmd.exe");
        configure_agent_browser_command(&mut command);
        let output = command
            .args(["/d", "/s", "/c", "echo luna-mux"])
            .output()
            .expect("run a hidden console command");

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "luna-mux");
    }

    #[test]
    fn normalizes_local_development_urls() {
        assert_eq!(
            normalize_url("localhost:3000").unwrap(),
            "http://localhost:3000/"
        );
        assert_eq!(normalize_url("").unwrap(), "about:blank");
        assert!(normalize_url("file:///tmp/private").is_err());
    }

    #[test]
    fn marks_managed_chrome_profile_clean_without_discarding_preferences() {
        let root = std::env::temp_dir().join(format!(
            "luna-mux-browser-preferences-{}",
            uuid::Uuid::new_v4()
        ));
        let default_profile = root.join("Default");
        std::fs::create_dir_all(&default_profile).unwrap();
        std::fs::write(
            default_profile.join("Preferences"),
            r#"{"bookmark_bar":{"show_on_all_tabs":true},"profile":{"exit_type":"Crashed","exited_cleanly":false}}"#,
        )
        .unwrap();

        mark_chrome_profile_clean(&root).unwrap();

        let preferences: Value = serde_json::from_str(
            &std::fs::read_to_string(default_profile.join("Preferences")).unwrap(),
        )
        .unwrap();
        assert_eq!(preferences["profile"]["exit_type"], "Normal");
        assert_eq!(preferences["profile"]["exited_cleanly"], true);
        assert_eq!(preferences["bookmark_bar"]["show_on_all_tabs"], true);
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn parses_numeric_chrome_installation_directories() {
        assert_eq!(
            parse_version_parts("151.0.7922.77"),
            Some(vec![151, 0, 7922, 77])
        );
        assert_eq!(parse_version_parts("150.0.1"), None);
        assert_eq!(parse_version_parts("User Data"), None);
    }

    #[test]
    fn parses_browser_shortcuts_without_platform_specific_key_codes() {
        assert_eq!(
            parse_shortcut("Ctrl+Shift+A").unwrap(),
            ("A".into(), "KeyA".into(), String::new(), 10, 65)
        );
        assert_eq!(
            parse_shortcut("Meta+Enter").unwrap(),
            ("Enter".into(), "Enter".into(), "\r".into(), 4, 13)
        );
        assert!(parse_shortcut("Hyper+K").is_err());
        assert!(parse_shortcut("").is_err());
    }

    #[test]
    fn cdp_endpoint_is_stable_per_session_and_isolated_across_sessions() {
        let root = std::env::temp_dir().join(format!(
            "luna-mux-browser-endpoint-{}",
            uuid::Uuid::new_v4()
        ));
        let manager = BrowserRuntimeManager::new_for_test(&root);
        let first = manager.session_cdp_port("session-1").unwrap();
        let second = manager.session_cdp_port("session-1").unwrap();
        let other_session = manager.session_cdp_port("session-2").unwrap();
        assert_eq!(first, second);
        assert_ne!(first, other_session);
        assert!(std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, first)).is_ok());
        assert!(
            std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, other_session)).is_ok()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn browser_resources_use_independent_persistent_profiles() {
        let root =
            std::env::temp_dir().join(format!("luna-mux-browser-profile-{}", uuid::Uuid::new_v4()));
        let manager = BrowserRuntimeManager::new_for_test(&root);
        let first = manager.resource_profile_path("session-1", "resource-1");
        let second = manager.resource_profile_path("session-2", "resource-2");
        assert_eq!(
            first,
            root.join("browser-profiles")
                .join("sessions")
                .join("session-1")
                .join("resource-1")
        );
        assert_ne!(first, second);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn agent_browser_mcp_config_forces_the_session_cdp_endpoint() {
        let path = create_agent_browser_mcp_config("luna-mux-session-1", 43_129).unwrap();
        let config: Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).expect("valid config JSON");
        assert_eq!(config["cdp"], "43129");
        assert_eq!(config["contentBoundaries"], true);
        assert_eq!(config["maxOutput"], 50_000);
        assert_eq!(config["pinTab"], true);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn agent_browser_scope_is_stable_and_platform_safe() {
        let first = super::agent_browser_scope("cc469b2c-f1b2-4397-8eb2-21847984b6c7");
        let repeated = super::agent_browser_scope("cc469b2c-f1b2-4397-8eb2-21847984b6c7");
        let other = super::agent_browser_scope("dc469b2c-f1b2-4397-8eb2-21847984b6c7");
        assert_eq!(first, repeated);
        assert_ne!(first, other);

        #[cfg(not(windows))]
        {
            assert!(first.starts_with("lm-"));
            assert_eq!(first.len(), 19);
        }
        #[cfg(windows)]
        assert_eq!(first, "luna-mux-cc469b2c-f1b2-4397-8eb2-21847984b6c7");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn stale_chrome_detection_matches_only_luna_mux_main_processes() {
        let profiles_root = std::path::Path::new(
            "/Users/example/Library/Application Support/com.luna.mux/browser-profiles",
        );
        let processes = r#"
  101 /Applications/Google Chrome.app/Contents/MacOS/Google Chrome --remote-debugging-port=43128 --user-data-dir=/Users/example/Library/Application Support/com.luna.mux/browser-profiles/sessions/session/resource
  102 /Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Helpers/Google Chrome Helper.app/Contents/MacOS/Google Chrome Helper --type=renderer --user-data-dir=/Users/example/Library/Application Support/com.luna.mux/browser-profiles/sessions/session/resource
  103 /Applications/Google Chrome.app/Contents/MacOS/Google Chrome --user-data-dir=/Users/example/Library/Application Support/Google/Chrome
  104 /Applications/Google Chrome.app/Contents/MacOS/Google Chrome --user-data-dir=/Users/example/Library/Application Support/com.luna.mux/browser-profiles-other/session/resource
  105 /usr/bin/some-tool --user-data-dir=/Users/example/Library/Application Support/com.luna.mux/browser-profiles/sessions/session/resource
"#;
        assert_eq!(
            super::managed_chrome_process_ids(processes, profiles_root),
            vec![101]
        );
    }

    #[tokio::test]
    #[ignore = "requires installed Chrome and the bundled agent-browser sidecar"]
    async fn real_agent_browser_mcp_recovers_a_stale_pin_and_reuses_managed_chrome() {
        let chrome = discover_chrome().expect("Chrome installation");
        let session_id = format!("agent-browser-mcp-test-{}", Uuid::new_v4());
        let scope = super::agent_browser_scope(&session_id);
        let profile =
            std::env::temp_dir().join(format!("luna-mux-agent-browser-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&profile).unwrap();
        let port = reserve_loopback_port().unwrap();
        let mut chrome_command = std::process::Command::new(chrome.executable_path);
        chrome_command
            .arg(format!("--remote-debugging-port={port}"))
            .arg("--remote-debugging-address=127.0.0.1")
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("--headless=new")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        configure_process_group(&mut chrome_command);
        let mut chrome_child = chrome_command.spawn().unwrap();
        wait_for_cdp(port, Duration::from_secs(8)).await.unwrap();

        let warm_result = warm_agent_browser_session(&session_id, port).await;
        let client = reqwest::Client::new();
        let pages = client
            .get(format!("http://127.0.0.1:{port}/json/list"))
            .send()
            .await
            .unwrap()
            .json::<Vec<Value>>()
            .await
            .unwrap();
        warm_result.as_ref().expect("agent-browser daemon warmup");
        assert_eq!(
            pages.iter().filter(|page| page["type"] == "page").count(),
            1,
            "warmup must remove Chrome's replaced bootstrap page: {pages:?}"
        );
        let pinned_target = pages
            .iter()
            .find(|page| page["type"] == "page")
            .and_then(|page| page["id"].as_str())
            .unwrap()
            .to_string();
        client
            .put(format!("http://127.0.0.1:{port}/json/new?about:blank"))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        client
            .get(format!(
                "http://127.0.0.1:{port}/json/close/{pinned_target}"
            ))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let stale_pin_recovery = warm_agent_browser_session(&session_id, port).await;
        let rebound_pages = client
            .get(format!("http://127.0.0.1:{port}/json/list"))
            .send()
            .await
            .unwrap()
            .json::<Vec<Value>>()
            .await
            .unwrap();
        let rebound_target = rebound_pages
            .iter()
            .find(|page| page["type"] == "page")
            .and_then(|page| page["id"].as_str())
            .unwrap()
            .to_string();
        let config_path = create_agent_browser_mcp_config(&scope, port).unwrap();
        let binary = resolve_agent_browser_binary().unwrap();
        let mut mcp_command = std::process::Command::new(binary);
        super::configure_agent_browser_command(&mut mcp_command);
        let mut mcp = mcp_command
            .args(["mcp", "--tools", "core"])
            .env("AGENT_BROWSER_CONFIG", &config_path)
            .env("AGENT_BROWSER_SESSION", &scope)
            .env("AGENT_BROWSER_NAMESPACE", &scope)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut stdin = mcp.stdin.take().unwrap();
        let stdout = mcp.stdout.take().unwrap();
        let (line_tx, line_rx) = std_mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if line_tx.send(line).is_err() {
                    break;
                }
            }
        });
        writeln!(
            stdin,
            "{}",
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": { "name": "luna-mux-test", "version": "1.0" }
                }
            })
        )
        .unwrap();
        stdin.flush().unwrap();
        let initialize = line_rx.recv_timeout(Duration::from_secs(5));
        writeln!(
            stdin,
            "{}",
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized", "params": {} })
        )
        .unwrap();
        writeln!(
            stdin,
            "{}",
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "agent_browser_get_url",
                    "arguments": { "timeoutMs": 5_000 }
                }
            })
        )
        .unwrap();
        stdin.flush().unwrap();
        let get_url_tool = line_rx.recv_timeout(Duration::from_secs(10));
        let pages_after_get = client
            .get(format!("http://127.0.0.1:{port}/json/list"))
            .send()
            .await
            .unwrap()
            .json::<Vec<Value>>()
            .await
            .unwrap();
        writeln!(
            stdin,
            "{}",
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "agent_browser_open",
                    "arguments": {
                        "url": "data:text/html,<title>Luna Mux reused tab</title>",
                        "timeoutMs": 5_000
                    }
                }
            })
        )
        .unwrap();
        stdin.flush().unwrap();
        let open_tool = line_rx.recv_timeout(Duration::from_secs(10));
        let pages_after_open = client
            .get(format!("http://127.0.0.1:{port}/json/list"))
            .send()
            .await
            .unwrap()
            .json::<Vec<Value>>()
            .await
            .unwrap();

        drop(stdin);
        for _ in 0..50 {
            if mcp.try_wait().is_ok_and(|status| status.is_some()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = mcp.kill();
        let _ = mcp.wait();
        close_agent_browser_session(&session_id).await;
        close_process_tree(&mut chrome_child);
        let _ = chrome_child.kill();
        let _ = chrome_child.wait();
        let _ = std::fs::remove_file(config_path);
        let _ = std::fs::remove_dir_all(profile);

        stale_pin_recovery.expect("stale agent-browser tab binding recovery");
        let initialize: Value = serde_json::from_str(&initialize.expect("initialize response"))
            .expect("valid initialize JSON");
        assert_eq!(initialize["result"]["serverInfo"]["name"], "agent-browser");
        let get_url_tool: Value =
            serde_json::from_str(&get_url_tool.expect("get URL tool response"))
                .expect("valid get URL tool JSON");
        assert_eq!(get_url_tool["id"], 2);
        assert_eq!(get_url_tool["result"]["isError"], false);
        assert_eq!(
            pages_after_get
                .iter()
                .filter(|page| page["type"] == "page")
                .count(),
            1,
            "get URL must reuse the bound page: {pages_after_get:?}"
        );
        assert_eq!(
            get_url_tool["result"]["structuredContent"]["response"]["data"]["lifecycle"]["reused"],
            true
        );
        let open_tool: Value = serde_json::from_str(&open_tool.expect("open tool response"))
            .expect("valid open tool JSON");
        assert_eq!(open_tool["id"], 3);
        assert_eq!(open_tool["result"]["isError"], false);
        let page_targets = pages_after_open
            .iter()
            .filter(|page| page["type"] == "page")
            .collect::<Vec<_>>();
        assert_eq!(page_targets.len(), 1, "open must reuse the bound page");
        assert_eq!(page_targets[0]["id"], rebound_target);
    }

    #[tokio::test]
    #[ignore = "requires installed Chrome and the bundled agent-browser sidecar"]
    async fn real_managed_browser_survives_agent_browser_replacing_its_bootstrap_page() {
        let root =
            std::env::temp_dir().join(format!("luna-mux-browser-rebind-test-{}", Uuid::new_v4()));
        let manager = BrowserRuntimeManager::new_for_test(&root);
        let session_id = format!("managed-browser-rebind-{}", Uuid::new_v4());
        let runtime = manager
            .create(BrowserRuntimeCreateRequest {
                mux_session_id: session_id.clone(),
                browser_resource_id: "browser-resource".into(),
                url: "about:blank".into(),
                temporary_profile: true,
            })
            .await
            .unwrap();

        let warm_result = warm_agent_browser_session(&session_id, runtime.cdp_port).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let runtime_after_warm = manager
            .list()
            .unwrap()
            .into_iter()
            .find(|candidate| candidate.id == runtime.id);
        let evaluate_after_warm = manager.evaluate(&runtime.id, "1 + 1").await;

        manager.close(&runtime.id).await.unwrap();
        let _ = std::fs::remove_dir_all(root);

        warm_result.expect("agent-browser warmup");
        assert_eq!(
            runtime_after_warm.map(|runtime| runtime.status),
            Some(BrowserRuntimeStatus::Running),
            "replacing the bootstrap tab must not stop the managed Chrome runtime"
        );
        assert_eq!(
            evaluate_after_warm.unwrap()["value"],
            2,
            "Luna CDP controls must reconnect to agent-browser's pinned tab"
        );
    }

    #[test]
    fn selects_the_requested_page_before_chrome_internal_targets() {
        let targets = json!([
            { "type": "page", "url": "chrome://newtab/", "webSocketDebuggerUrl": "ws://internal" },
            { "type": "page", "url": "http://localhost:3000/", "webSocketDebuggerUrl": "ws://requested" },
            { "type": "page", "url": "http://localhost:4000/", "webSocketDebuggerUrl": "ws://other" }
        ]);
        assert_eq!(
            select_page_websocket_url(targets.as_array().unwrap(), "http://localhost:3000/")
                .as_deref(),
            Some("ws://requested")
        );
    }

    #[tokio::test]
    #[ignore = "requires an installed Chrome and creates a real managed browser process"]
    async fn real_chrome_supports_an_on_demand_screenshot() {
        let chrome = discover_chrome().expect("Chrome installation");
        let profile =
            std::env::temp_dir().join(format!("luna-mux-browser-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&profile).unwrap();
        let port = reserve_loopback_port().unwrap();
        let mut command = std::process::Command::new(chrome.executable_path);
        command
            .arg(format!("--remote-debugging-port={port}"))
            .arg("--remote-debugging-address=127.0.0.1")
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("--new-window")
            .arg("data:text/html,<main style='font:40px sans-serif'>Luna Mux Browser Test</main>")
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-background-mode")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        configure_process_group(&mut command);
        let mut child = command.spawn().unwrap();
        wait_for_cdp(port, Duration::from_secs(8)).await.unwrap();
        let websocket = page_websocket_url(
            port,
            "data:text/html,<main style='font:40px sans-serif'>Luna Mux Browser Test</main>",
        )
        .await
        .unwrap();
        let (socket, _) = connect_async(websocket).await.unwrap();
        let (mut write, mut read) = socket.split();
        write
            .send(Message::Text(
                json!({"id":1,"method":"Page.captureScreenshot","params":{"format":"png"}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let screenshot = tokio::time::timeout(Duration::from_secs(8), async {
            while let Some(Ok(Message::Text(text))) = read.next().await {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value.get("id").and_then(Value::as_u64) == Some(1) {
                    return value;
                }
            }
            panic!("CDP connection ended before the screenshot arrived")
        })
        .await
        .expect("screenshot timeout");
        assert!(
            screenshot["result"]["data"]
                .as_str()
                .is_some_and(|data| data.len() > 100)
        );
        close_process_tree(&mut child);
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(profile);
    }

    #[tokio::test]
    #[ignore = "requires an installed Chrome and creates a real managed browser process"]
    async fn real_chrome_supports_high_level_agent_operations() {
        let root =
            std::env::temp_dir().join(format!("luna-mux-browser-manager-test-{}", Uuid::new_v4()));
        let manager = BrowserRuntimeManager::new_for_test(&root);
        let html = "<main><label>Name <input id='name'></label><button id='save' onclick=\"this.textContent='Saved'\">Save</button><div id='result'></div></main>";
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut request = [0_u8; 4096];
                let _ = socket.read(&mut request).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    html.len(),
                    html
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
        let runtime = manager
            .create(BrowserRuntimeCreateRequest {
                mux_session_id: "integration-session".into(),
                browser_resource_id: "browser-resource".into(),
                url: format!("http://127.0.0.1:{port}"),
                temporary_profile: false,
            })
            .await
            .unwrap();
        manager.focus_external_window(&runtime.id).unwrap();
        assert!(
            manager
                .wait_for_selector(&runtime.id, "#save", 5_000)
                .await
                .unwrap()
        );
        assert!(
            manager
                .type_text(&runtime.id, "#name", "Luna Mux", true)
                .await
                .unwrap()
        );
        manager.press(&runtime.id, "End").unwrap();
        manager.scroll(&runtime.id, 0.0, 120.0, 50.0, 50.0).unwrap();
        assert!(manager.click(&runtime.id, "#save").await.unwrap());
        let values = manager
            .evaluate(
                &runtime.id,
                "({ name: document.querySelector('#name').value, saved: document.querySelector('#save').textContent })",
            )
            .await
            .unwrap();
        assert_eq!(values["value"]["name"], "Luna Mux");
        assert_eq!(values["value"]["saved"], "Saved");
        let snapshot = manager.snapshot(&runtime.id).await.unwrap();
        assert!(
            snapshot["nodes"]
                .as_array()
                .is_some_and(|nodes| !nodes.is_empty())
        );
        let screenshot = manager.screenshot(&runtime.id).await.unwrap();
        assert!(screenshot.starts_with("data:image/png;base64,"));
        assert!(screenshot.len() > 100);

        let second_runtime = manager
            .create(BrowserRuntimeCreateRequest {
                mux_session_id: "integration-session-2".into(),
                browser_resource_id: "browser-resource-2".into(),
                url: "about:blank".into(),
                temporary_profile: false,
            })
            .await
            .unwrap();
        assert_ne!(runtime.process_id, second_runtime.process_id);
        assert_ne!(runtime.cdp_port, second_runtime.cdp_port);
        assert_ne!(runtime.profile_path, second_runtime.profile_path);
        assert_eq!(manager.list().unwrap().len(), 2);

        let profile = runtime.profile_path.clone();
        manager.close(&runtime.id).await.unwrap();
        assert_eq!(manager.list().unwrap().len(), 1);
        assert!(std::path::Path::new(&profile).exists());
        manager.close(&second_runtime.id).await.unwrap();
        assert!(manager.list().unwrap().is_empty());
        assert!(std::path::Path::new(&profile).exists());
        server.abort();
        let _ = std::fs::remove_dir_all(root);
    }
}
