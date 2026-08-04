use std::{
    collections::HashMap,
    io,
    pin::Pin,
    sync::{
        Arc, Mutex as StdMutex, RwLock,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, anyhow};
use async_recursion::async_recursion;
use base64::{Engine, engine::general_purpose::STANDARD};
use russh::{
    ChannelMsg, Disconnect, client,
    keys::{self, HashAlg, PrivateKeyWithHashAlg},
};
use russh_sftp::{client::SftpSession, protocol::FileType};
use tauri::{AppHandle, Emitter};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite},
    sync::{Mutex, Notify, mpsc, oneshot},
};
use uuid::Uuid;

use crate::{database::Database, models::*};

trait AsyncTransport: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncTransport for T {}
type Transport = Pin<Box<dyn AsyncTransport>>;

const REMOTE_EXEC_TIMEOUT: Duration = Duration::from_secs(12);
const REMOTE_EXEC_OUTPUT_LIMIT: usize = 16 * 1024 * 1024;
const DEFAULT_TERMINAL_SIZE: (u32, u32) = (100, 30);
const REMOTE_LIST_MODE_UNKNOWN: u8 = 0;
const REMOTE_LIST_MODE_EXEC: u8 = 1;
const REMOTE_LIST_MODE_SFTP: u8 = 2;
const REMOTE_LIST_MARKER: &str = "__SSH_CLIENT_REMOTE_LIST_V1__";
const REMOTE_LIST_SCRIPT: &str = r#"import base64,json,os,sys
path=os.fsdecode(base64.b64decode(sys.argv[1]))
items=[]
with os.scandir(path) as entries:
 for entry in entries:
  try:
   kind='symlink' if entry.is_symlink() else 'directory' if entry.is_dir(follow_symlinks=False) else 'file' if entry.is_file(follow_symlinks=False) else 'other'
   items.append({'name':entry.name,'path':os.path.join(path,entry.name),'kind':kind})
  except OSError:
   items.append({'name':entry.name,'path':os.path.join(path,entry.name),'kind':'other'})
json_bytes=json.dumps(items,separators=(',',':'),ensure_ascii=False).encode('utf-8','replace')
payload=base64.b64encode(json_bytes).decode()
sys.stdout.write('__SSH_CLIENT_REMOTE_LIST_V1__'+payload+'\n')"#;

#[derive(Debug)]
enum RemoteExecError {
    Unavailable,
    Timeout,
    Failed(String),
}

struct ClientHandler {
    session_id: String,
    bookmark: Bookmark,
    app: AppHandle,
    db: Arc<Database>,
    decisions: Arc<StdMutex<HashMap<String, oneshot::Sender<bool>>>>,
    forwarded_routes: ForwardedRoutes,
}

type ForwardedRoutes =
    Arc<StdMutex<HashMap<(String, u32), mpsc::UnboundedSender<russh::Channel<client::Msg>>>>>;

impl client::Handler for ClientHandler {
    type Error = anyhow::Error;

    async fn check_server_key(&mut self, key: &keys::PublicKey) -> Result<bool, Self::Error> {
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let known = self
            .db
            .known_host(&self.bookmark.host, self.bookmark.port)
            .map_err(|error| anyhow!(error))?;
        if known.as_deref() == Some(&fingerprint) {
            return Ok(true);
        }
        let (send, receive) = oneshot::channel();
        self.decisions
            .lock()
            .map_err(|_| anyhow!("Host Key 状态锁已损坏"))?
            .insert(self.session_id.clone(), send);
        self.app.emit(
            "app:event",
            AppEvent::HostKey(HostKeyPrompt {
                session_id: self.session_id.clone(),
                host: self.bookmark.host.clone(),
                port: self.bookmark.port,
                fingerprint: fingerprint.clone(),
                status: if known.is_some() {
                    HostKeyStatus::Changed
                } else {
                    HostKeyStatus::Unknown
                },
                previous_fingerprint: known,
            }),
        )?;
        let accepted = tokio::time::timeout(Duration::from_secs(60), receive)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(false);
        self.decisions
            .lock()
            .map_err(|_| anyhow!("Host Key 状态锁已损坏"))?
            .remove(&self.session_id);
        if accepted {
            self.db
                .trust_host(&self.bookmark.host, self.bookmark.port, &fingerprint)
                .map_err(|error| anyhow!(error))?;
        }
        Ok(accepted)
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: russh::Channel<client::Msg>,
        _connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        if let Ok(routes) = self.forwarded_routes.lock() {
            if let Some(sender) = routes.get(&(self.session_id.clone(), connected_port)) {
                let _ = sender.send(channel);
            }
        }
        Ok(())
    }
}

pub struct ActiveSession {
    summary: RwLock<SessionSummary>,
    pub bookmark: Bookmark,
    handle: Mutex<Option<client::Handle<ClientHandler>>>,
    jump_handle: Mutex<Option<client::Handle<ClientHandler>>>,
    shell: Mutex<Option<russh::ChannelWriteHalf<client::Msg>>>,
    terminal_size: StdMutex<(u32, u32)>,
    sftp: Mutex<Option<Arc<SftpSession>>>,
    remote_home: Mutex<Option<String>>,
    remote_list_mode: AtomicU8,
    paused: AtomicBool,
    resume: Notify,
    closing: AtomicBool,
}

impl ActiveSession {
    fn summary(&self) -> SessionSummary {
        self.summary.read().expect("session summary lock").clone()
    }
    fn set_summary(&self, status: SessionStatus, error: Option<String>) {
        let mut summary = self.summary.write().expect("session summary lock");
        summary.status = status;
        summary.error = error;
    }
}

pub struct SessionManager {
    app: AppHandle,
    db: Arc<Database>,
    sessions: RwLock<HashMap<String, Arc<ActiveSession>>>,
    decisions: Arc<StdMutex<HashMap<String, oneshot::Sender<bool>>>>,
    forwarded_routes: ForwardedRoutes,
}

impl SessionManager {
    pub fn new(app: AppHandle, db: Arc<Database>) -> Arc<Self> {
        Arc::new(Self {
            app,
            db,
            sessions: RwLock::new(HashMap::new()),
            decisions: Arc::new(StdMutex::new(HashMap::new())),
            forwarded_routes: Arc::new(StdMutex::new(HashMap::new())),
        })
    }
    fn emit(&self, event: AppEvent) {
        let _ = self.app.emit("app:event", event);
    }
    pub fn list(&self) -> Vec<SessionSummary> {
        self.sessions
            .read()
            .expect("sessions lock")
            .values()
            .map(|session| session.summary())
            .collect()
    }
    pub fn get(&self, id: &str) -> Result<Arc<ActiveSession>, String> {
        self.sessions
            .read()
            .map_err(|_| "会话状态锁已损坏")?
            .get(id)
            .cloned()
            .ok_or_else(|| "SSH 会话未连接".into())
    }
    pub fn bookmark_id(&self, id: &str) -> Result<String, String> {
        Ok(self.get(id)?.bookmark.id.clone())
    }

    pub fn connect(self: &Arc<Self>, input: ConnectInput) -> Result<SessionSummary, String> {
        let bookmark = self
            .db
            .get_bookmark(&input.bookmark_id)?
            .ok_or_else(|| "连接不存在".to_string())?;
        if !input.new_session {
            if let Some(existing) = self
                .sessions
                .read()
                .map_err(|_| "会话状态锁已损坏")?
                .values()
                .find(|session| {
                    session.bookmark.id == bookmark.id
                        && matches!(
                            session.summary().status,
                            SessionStatus::Connecting | SessionStatus::Connected
                        )
                })
            {
                return Ok(existing.summary());
            }
        }
        let session_id = Uuid::new_v4().to_string();
        let summary = SessionSummary {
            id: session_id.clone(),
            bookmark_id: bookmark.id.clone(),
            title: bookmark.name.clone(),
            status: SessionStatus::Connecting,
            error: None,
        };
        let active = Arc::new(ActiveSession {
            summary: RwLock::new(summary.clone()),
            bookmark,
            handle: Mutex::new(None),
            jump_handle: Mutex::new(None),
            shell: Mutex::new(None),
            terminal_size: StdMutex::new(DEFAULT_TERMINAL_SIZE),
            sftp: Mutex::new(None),
            remote_home: Mutex::new(None),
            remote_list_mode: AtomicU8::new(REMOTE_LIST_MODE_UNKNOWN),
            paused: AtomicBool::new(false),
            resume: Notify::new(),
            closing: AtomicBool::new(false),
        });
        self.sessions
            .write()
            .map_err(|_| "会话状态锁已损坏")?
            .insert(session_id, active.clone());
        self.emit(AppEvent::Session(summary.clone()));
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = manager.establish(active.clone(), input).await {
                manager.fail(&active, error.to_string());
            }
        });
        Ok(summary)
    }

    async fn establish(
        self: &Arc<Self>,
        active: Arc<ActiveSession>,
        input: ConnectInput,
    ) -> anyhow::Result<()> {
        let jump_bookmark = if active.bookmark.jump_bookmark_id.is_empty() {
            None
        } else {
            Some(
                self.db
                    .get_bookmark(&active.bookmark.jump_bookmark_id)
                    .map_err(|error| anyhow!(error))?
                    .ok_or_else(|| anyhow!("配置的跳板机连接不存在"))?,
            )
        };
        if jump_bookmark
            .as_ref()
            .is_some_and(|bookmark| !bookmark.jump_bookmark_id.is_empty())
        {
            return Err(anyhow!("当前仅支持单级跳板机"));
        }
        let transport: Transport = if let Some(jump) = jump_bookmark {
            let jump_credential = input
                .jump_credential
                .clone()
                .or_else(|| self.db.get_credential(&jump.id));
            let mut jump_handle = self
                .open_connection(&active.summary().id, &jump, None)
                .await
                .context("跳板机连接失败")?;
            self.authenticate(&mut jump_handle, &jump, jump_credential.as_deref())
                .await
                .context("跳板机认证失败")?;
            if input.remember_jump_credential {
                if let Some(secret) = &input.jump_credential {
                    self.db
                        .save_credential(&jump.id, secret)
                        .map_err(|error| anyhow!(error))?;
                }
            }
            let channel = jump_handle
                .channel_open_direct_tcpip(
                    active.bookmark.host.clone(),
                    active.bookmark.port as u32,
                    "127.0.0.1",
                    0,
                )
                .await?;
            *active.jump_handle.lock().await = Some(jump_handle);
            Box::pin(channel.into_stream())
        } else {
            let stream = tokio::time::timeout(
                Duration::from_secs(20),
                tokio::net::TcpStream::connect((
                    active.bookmark.host.as_str(),
                    active.bookmark.port,
                )),
            )
            .await
            .map_err(|_| anyhow!("连接超时"))??;
            stream.set_nodelay(true)?;
            Box::pin(stream)
        };
        let credential = input
            .credential
            .clone()
            .or_else(|| self.db.get_credential(&active.bookmark.id));
        let mut handle = self
            .open_connection(&active.summary().id, &active.bookmark, Some(transport))
            .await?;
        self.authenticate(&mut handle, &active.bookmark, credential.as_deref())
            .await?;
        if input.remember_credential {
            if let Some(secret) = &input.credential {
                self.db
                    .save_credential(&active.bookmark.id, secret)
                    .map_err(|error| anyhow!(error))?;
            }
        }
        let channel = handle.channel_open_session().await?;
        let mut shell = active.shell.lock().await;
        let (cols, rows) = *active
            .terminal_size
            .lock()
            .map_err(|_| anyhow!("终端尺寸锁已损坏"))?;
        channel
            .request_pty(false, "xterm-256color", cols, rows, 0, 0, &[])
            .await?;
        channel.request_shell(true).await?;
        let (reader, writer) = channel.split();
        *active.handle.lock().await = Some(handle);
        *shell = Some(writer);
        drop(shell);
        active.set_summary(SessionStatus::Connected, None);
        self.db
            .mark_connected(&active.bookmark.id)
            .map_err(|error| anyhow!(error))?;
        self.emit(AppEvent::Session(active.summary()));
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            manager.read_terminal(active, reader).await;
        });
        Ok(())
    }

    async fn open_connection(
        &self,
        session_id: &str,
        bookmark: &Bookmark,
        stream: Option<Transport>,
    ) -> anyhow::Result<client::Handle<ClientHandler>> {
        let config = client::Config {
            inactivity_timeout: Some(Duration::from_secs(20)),
            keepalive_interval: bookmark
                .keepalive_enabled
                .then(|| Duration::from_secs(bookmark.keepalive_interval_seconds as u64)),
            keepalive_max: bookmark.keepalive_count_max as usize,
            nodelay: true,
            ..Default::default()
        };
        let handler = ClientHandler {
            session_id: session_id.into(),
            bookmark: bookmark.clone(),
            app: self.app.clone(),
            db: self.db.clone(),
            decisions: self.decisions.clone(),
            forwarded_routes: self.forwarded_routes.clone(),
        };
        match stream {
            Some(stream) => client::connect_stream(Arc::new(config), stream, handler).await,
            None => {
                client::connect(
                    Arc::new(config),
                    (bookmark.host.as_str(), bookmark.port),
                    handler,
                )
                .await
            }
        }
    }

    async fn authenticate(
        &self,
        handle: &mut client::Handle<ClientHandler>,
        bookmark: &Bookmark,
        credential: Option<&str>,
    ) -> anyhow::Result<()> {
        let success = match bookmark.auth_type {
            AuthType::Password => {
                let secret = credential.ok_or_else(|| anyhow!("连接需要输入密码"))?;
                if handle
                    .authenticate_password(bookmark.username.clone(), secret)
                    .await?
                    .success()
                {
                    true
                } else {
                    self.keyboard_interactive(handle, &bookmark.username, secret)
                        .await?
                }
            }
            AuthType::PrivateKey => {
                if bookmark.private_key_path.is_empty() {
                    return Err(anyhow!("连接未配置私钥文件"));
                }
                let key = keys::load_secret_key(&bookmark.private_key_path, credential)
                    .map_err(|error| anyhow!(error.to_string()))?;
                let hash = handle.best_supported_rsa_hash().await?.flatten();
                handle
                    .authenticate_publickey(
                        bookmark.username.clone(),
                        PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                    )
                    .await?
                    .success()
            }
            AuthType::Agent => self.authenticate_agent(handle, &bookmark.username).await?,
        };
        if success {
            Ok(())
        } else {
            Err(anyhow!("SSH 认证失败"))
        }
    }

    async fn keyboard_interactive(
        &self,
        handle: &mut client::Handle<ClientHandler>,
        username: &str,
        secret: &str,
    ) -> anyhow::Result<bool> {
        let mut response = handle
            .authenticate_keyboard_interactive_start(username, None)
            .await?;
        loop {
            match response {
                client::KeyboardInteractiveAuthResponse::Success => return Ok(true),
                client::KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
                client::KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                    response = handle
                        .authenticate_keyboard_interactive_respond(
                            prompts.into_iter().map(|_| secret.to_string()).collect(),
                        )
                        .await?;
                }
            }
        }
    }

    #[cfg(unix)]
    async fn authenticate_agent(
        &self,
        handle: &mut client::Handle<ClientHandler>,
        username: &str,
    ) -> anyhow::Result<bool> {
        let mut agent = keys::agent::client::AgentClient::connect_env()
            .await
            .map_err(|error| anyhow!(error.to_string()))?;
        for key in agent
            .request_identities()
            .await
            .map_err(|error| anyhow!(error.to_string()))?
        {
            let hash = handle.best_supported_rsa_hash().await?.flatten();
            if handle
                .authenticate_publickey_with(username, key, hash, &mut agent)
                .await
                .map_err(|error| anyhow!(error.to_string()))?
                .success()
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    #[cfg(windows)]
    async fn authenticate_agent(
        &self,
        handle: &mut client::Handle<ClientHandler>,
        username: &str,
    ) -> anyhow::Result<bool> {
        let mut agent =
            keys::agent::client::AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent")
                .await
                .map_err(|error| anyhow!(error.to_string()))?;
        for key in agent
            .request_identities()
            .await
            .map_err(|error| anyhow!(error.to_string()))?
        {
            let hash = handle.best_supported_rsa_hash().await?.flatten();
            if handle
                .authenticate_publickey_with(username, key, hash, &mut agent)
                .await
                .map_err(|error| anyhow!(error.to_string()))?
                .success()
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn read_terminal(
        self: Arc<Self>,
        active: Arc<ActiveSession>,
        mut reader: russh::ChannelReadHalf,
    ) {
        let mut decoder = Utf8Decoder::default();
        while let Some(message) = reader.wait().await {
            while active.paused.load(Ordering::Acquire) && !active.closing.load(Ordering::Acquire) {
                active.resume.notified().await;
            }
            match message {
                ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                    let text = decoder.push(&data);
                    if !text.is_empty() {
                        self.emit(AppEvent::TerminalData(TerminalData {
                            session_id: active.summary().id,
                            data: text,
                        }));
                    }
                }
                ChannelMsg::Close | ChannelMsg::Eof => break,
                _ => {}
            }
        }
        let tail = decoder.finish();
        if !tail.is_empty() {
            self.emit(AppEvent::TerminalData(TerminalData {
                session_id: active.summary().id,
                data: tail,
            }));
        }
        if !active.closing.swap(true, Ordering::AcqRel) {
            active.set_summary(SessionStatus::Disconnected, None);
            self.emit(AppEvent::Session(active.summary()));
            self.sessions
                .write()
                .expect("sessions lock")
                .remove(&active.summary().id);
        }
    }

    fn fail(&self, active: &Arc<ActiveSession>, message: String) {
        if active.closing.swap(true, Ordering::AcqRel) {
            return;
        }
        active.set_summary(SessionStatus::Error, Some(message));
        self.emit(AppEvent::Session(active.summary()));
        self.sessions
            .write()
            .expect("sessions lock")
            .remove(&active.summary().id);
    }

    pub async fn write(&self, id: &str, data: String) -> Result<(), String> {
        let active = self.get(id)?;
        let shell = active.shell.lock().await;
        let writer = shell.as_ref().ok_or_else(|| "终端尚未就绪".to_string())?;
        writer
            .data(data.as_bytes())
            .await
            .map_err(|e| e.to_string())
    }
    pub async fn resize(&self, id: &str, cols: u32, rows: u32) -> Result<(), String> {
        let active = self.get(id)?;
        let size = (cols.max(1), rows.max(1));
        *active
            .terminal_size
            .lock()
            .map_err(|_| "终端尺寸锁已损坏".to_string())? = size;
        let shell = active.shell.lock().await;
        match shell.as_ref() {
            Some(writer) => writer
                .window_change(size.0, size.1, 0, 0)
                .await
                .map_err(|e| e.to_string()),
            None => Ok(()),
        }
    }
    pub fn flow(&self, id: &str, paused: bool) -> Result<(), String> {
        let active = self.get(id)?;
        active.paused.store(paused, Ordering::Release);
        if !paused {
            active.resume.notify_waiters();
        }
        Ok(())
    }
    pub async fn disconnect(&self, id: &str) -> Result<(), String> {
        let active = self.get(id)?;
        active.closing.store(true, Ordering::Release);
        active.resume.notify_waiters();
        if let Some(shell) = active.shell.lock().await.as_ref() {
            let _ = shell.close().await;
        }
        if let Some(handle) = active.handle.lock().await.as_ref() {
            let _ = handle
                .disconnect(Disconnect::ByApplication, "", "English")
                .await;
        }
        if let Some(handle) = active.jump_handle.lock().await.as_ref() {
            let _ = handle
                .disconnect(Disconnect::ByApplication, "", "English")
                .await;
        }
        if let Ok(mut routes) = self.forwarded_routes.lock() {
            routes.retain(|(session_id, _), _| session_id != id);
        }
        active.set_summary(SessionStatus::Disconnected, None);
        self.emit(AppEvent::Session(active.summary()));
        self.sessions
            .write()
            .map_err(|_| "会话状态锁已损坏")?
            .remove(id);
        Ok(())
    }
    pub fn host_key_decision(&self, id: &str, accept: bool) {
        if let Ok(mut decisions) = self.decisions.lock() {
            if let Some(send) = decisions.remove(id) {
                let _ = send.send(accept);
            }
        }
    }
    pub async fn disconnect_all(&self) {
        let ids: Vec<_> = self
            .sessions
            .read()
            .expect("sessions lock")
            .keys()
            .cloned()
            .collect();
        for id in ids {
            let _ = self.disconnect(&id).await;
        }
    }

    pub async fn sftp(&self, id: &str) -> Result<Arc<SftpSession>, String> {
        let active = self.get(id)?;
        if let Some(sftp) = active.sftp.lock().await.as_ref() {
            return Ok(sftp.clone());
        }
        let channel = {
            let handle = active.handle.lock().await;
            handle
                .as_ref()
                .ok_or_else(|| "SSH 会话未连接".to_string())?
                .channel_open_session()
                .await
                .map_err(|e| e.to_string())?
        };
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| e.to_string())?;
        let sftp = Arc::new(
            SftpSession::new(channel.into_stream())
                .await
                .map_err(|e| e.to_string())?,
        );
        sftp.set_timeout(20);
        *active.sftp.lock().await = Some(sftp.clone());
        Ok(sftp)
    }
    pub async fn direct_tcpip(
        &self,
        id: &str,
        host: String,
        port: u16,
        origin_host: String,
        origin_port: u16,
    ) -> Result<russh::Channel<client::Msg>, String> {
        let active = self.get(id)?;
        let handle = active.handle.lock().await;
        handle
            .as_ref()
            .ok_or_else(|| "SSH 会话未连接".to_string())?
            .channel_open_direct_tcpip(host, port as u32, origin_host, origin_port as u32)
            .await
            .map_err(|e| e.to_string())
    }
    pub async fn request_remote_forward(
        &self,
        id: &str,
        address: String,
        port: u16,
    ) -> Result<(u16, mpsc::UnboundedReceiver<russh::Channel<client::Msg>>), String> {
        let active = self.get(id)?;
        let assigned = {
            let mut handle = active.handle.lock().await;
            handle
                .as_mut()
                .ok_or_else(|| "SSH 会话未连接".to_string())?
                .tcpip_forward(address, port as u32)
                .await
                .map_err(|e| e.to_string())?
        };
        let actual = if assigned == 0 { port as u32 } else { assigned };
        let actual = u16::try_from(actual).map_err(|_| "服务器返回了无效端口".to_string())?;
        let (sender, receiver) = mpsc::unbounded_channel();
        self.forwarded_routes
            .lock()
            .map_err(|_| "远端转发路由锁已损坏")?
            .insert((id.to_string(), actual as u32), sender);
        Ok((actual, receiver))
    }
    pub async fn cancel_remote_forward(
        &self,
        id: &str,
        address: String,
        port: u16,
    ) -> Result<(), String> {
        if let Ok(mut routes) = self.forwarded_routes.lock() {
            routes.remove(&(id.to_string(), port as u32));
        }
        let active = self.get(id)?;
        let handle = active.handle.lock().await;
        handle
            .as_ref()
            .ok_or_else(|| "SSH 会话未连接".to_string())?
            .cancel_tcpip_forward(address, port as u32)
            .await
            .map_err(|e| e.to_string())
    }
    pub async fn remote_home(&self, id: &str) -> Result<String, String> {
        let active = self.get(id)?;
        let mut cached = active.remote_home.lock().await;
        if let Some(path) = cached.as_ref() {
            return Ok(path.clone());
        }
        if let Ok(output) = self.exec_remote(&active, "printf '%s' \"$HOME\"").await {
            let path = output.trim().to_string();
            if path.starts_with('/') {
                *cached = Some(path.clone());
                return Ok(path);
            }
        }
        let path = self
            .sftp(id)
            .await?
            .canonicalize(".")
            .await
            .map_err(|e| e.to_string())?;
        *cached = Some(path.clone());
        Ok(path)
    }
    pub async fn list_remote(&self, id: &str, path: String) -> Result<Vec<DirectoryEntry>, String> {
        let active = self.get(id)?;
        if active.remote_list_mode.load(Ordering::Acquire) != REMOTE_LIST_MODE_SFTP {
            match self.list_remote_with_exec(&active, &path).await {
                Ok(entries) => {
                    active
                        .remote_list_mode
                        .store(REMOTE_LIST_MODE_EXEC, Ordering::Release);
                    return Ok(entries);
                }
                Err(RemoteExecError::Unavailable) => {
                    active
                        .remote_list_mode
                        .store(REMOTE_LIST_MODE_SFTP, Ordering::Release);
                }
                Err(RemoteExecError::Timeout) => {}
                Err(RemoteExecError::Failed(message)) => return Err(message),
            }
        }
        self.list_remote_with_sftp(id, path).await
    }
    async fn list_remote_with_exec(
        &self,
        active: &ActiveSession,
        path: &str,
    ) -> Result<Vec<DirectoryEntry>, RemoteExecError> {
        let encoded_path = STANDARD.encode(path.as_bytes());
        let command = format!(
            "python3 -c {} {}",
            shell_quote(REMOTE_LIST_SCRIPT),
            shell_quote(&encoded_path)
        );
        let output = self.exec_remote(active, &command).await?;
        let parsed = parse_remote_entries(&output);
        #[cfg(debug_assertions)]
        match &parsed {
            Ok(entries) => eprintln!(
                "remote directory Python protocol succeeded: path={path:?}, entries={}",
                entries.len()
            ),
            Err(_) => eprintln!(
                "remote directory Python protocol failed: path={path:?}, bytes={}, response={:?}",
                output.len(),
                output.chars().take(512).collect::<String>()
            ),
        }
        parsed
    }
    async fn list_remote_with_sftp(
        &self,
        id: &str,
        path: String,
    ) -> Result<Vec<DirectoryEntry>, String> {
        let entries = self
            .sftp(id)
            .await?
            .read_dir(path)
            .await
            .map_err(|e| e.to_string())?;
        Ok(entries
            .map(|entry| {
                let metadata = entry.metadata();
                let kind = match entry.file_type() {
                    FileType::Dir => EntryKind::Directory,
                    FileType::File => EntryKind::File,
                    FileType::Symlink => EntryKind::Symlink,
                    _ => EntryKind::Other,
                };
                DirectoryEntry {
                    name: entry.file_name(),
                    path: entry.path(),
                    kind,
                    size: metadata.size,
                    modified_at: metadata.mtime.map(|value| value as i64 * 1000),
                }
            })
            .collect())
    }
    async fn exec_remote(
        &self,
        active: &ActiveSession,
        command: &str,
    ) -> Result<String, RemoteExecError> {
        let deadline = tokio::time::Instant::now() + REMOTE_EXEC_TIMEOUT;
        let mut channel = tokio::time::timeout_at(deadline, async {
            let handle = active.handle.lock().await;
            let channel = handle
                .as_ref()
                .ok_or(RemoteExecError::Unavailable)?
                .channel_open_session()
                .await
                .map_err(|_| RemoteExecError::Unavailable)?;
            Ok::<_, RemoteExecError>(channel)
        })
        .await
        .map_err(|_| RemoteExecError::Timeout)??;
        match tokio::time::timeout_at(deadline, channel.exec(true, command)).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                let _ = channel.close().await;
                return Err(RemoteExecError::Unavailable);
            }
            Err(_) => {
                let _ = channel.close().await;
                return Err(RemoteExecError::Timeout);
            }
        }
        let read_output = async {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let mut exit_status = None;
            while let Some(message) = channel.wait().await {
                match message {
                    ChannelMsg::Data { data } => {
                        if stdout.len().saturating_add(data.len()) > REMOTE_EXEC_OUTPUT_LIMIT {
                            let _ = channel.close().await;
                            return Err(RemoteExecError::Failed("远端目录内容过大".into()));
                        }
                        stdout.extend_from_slice(&data);
                    }
                    ChannelMsg::ExtendedData { data, .. } => {
                        if stderr.len().saturating_add(data.len()) <= REMOTE_EXEC_OUTPUT_LIMIT {
                            stderr.extend_from_slice(&data);
                        }
                    }
                    ChannelMsg::ExitStatus {
                        exit_status: status,
                    } => exit_status = Some(status),
                    ChannelMsg::ExitSignal { error_message, .. } => {
                        return Err(RemoteExecError::Failed(if error_message.is_empty() {
                            "远端目录命令被终止".into()
                        } else {
                            error_message
                        }));
                    }
                    _ => {}
                }
            }
            if let Some(status) = exit_status.filter(|status| *status != 0) {
                let message = String::from_utf8_lossy(&stderr).trim().to_string();
                let message = if message.is_empty() {
                    format!("远端命令退出码 {status}")
                } else {
                    message
                };
                return Err(if status == 127 {
                    RemoteExecError::Unavailable
                } else {
                    RemoteExecError::Failed(message)
                });
            }
            String::from_utf8(stdout)
                .map_err(|_| RemoteExecError::Failed("远端目录响应不是 UTF-8 文本".into()))
        };
        match tokio::time::timeout_at(deadline, read_output).await {
            Ok(result) => result,
            Err(_) => {
                let _ = channel.close().await;
                Err(RemoteExecError::Timeout)
            }
        }
    }
    pub async fn remote_tree(
        &self,
        id: &str,
        root: &str,
    ) -> Result<HashMap<String, DirectoryEntry>, String> {
        let sftp = self.sftp(id).await?;
        let mut result = HashMap::new();
        let mut pending = vec![(root.trim_end_matches('/').to_string(), String::new())];
        while let Some((directory, prefix)) = pending.pop() {
            for entry in sftp
                .read_dir(if directory.is_empty() {
                    "/".to_string()
                } else {
                    directory.clone()
                })
                .await
                .map_err(|e| e.to_string())?
            {
                let metadata = entry.metadata();
                if metadata.is_symlink() {
                    return Err(format!("暂不支持部署符号链接：{}", entry.path()));
                }
                let relative = if prefix.is_empty() {
                    entry.file_name()
                } else {
                    format!("{prefix}/{}", entry.file_name())
                };
                if metadata.is_dir() {
                    pending.push((entry.path(), relative));
                } else if metadata.is_regular() {
                    result.insert(
                        relative,
                        DirectoryEntry {
                            name: entry.file_name(),
                            path: entry.path(),
                            kind: EntryKind::File,
                            size: metadata.size,
                            modified_at: metadata.mtime.map(|value| value as i64 * 1000),
                        },
                    );
                }
            }
        }
        Ok(result)
    }
    pub async fn create_remote_directory(&self, id: &str, path: String) -> Result<(), String> {
        self.sftp(id)
            .await?
            .create_dir(path)
            .await
            .map_err(|e| e.to_string())
    }
    pub async fn rename_remote(&self, id: &str, from: String, to: String) -> Result<(), String> {
        self.sftp(id)
            .await?
            .rename(from, to)
            .await
            .map_err(|e| e.to_string())
    }
    pub async fn remove_remote(&self, id: &str, paths: Vec<String>) -> Result<(), String> {
        let sftp = self.sftp(id).await?;
        for path in paths {
            self.remove_remote_entry(&sftp, path).await?;
        }
        Ok(())
    }
    #[async_recursion]
    async fn remove_remote_entry(&self, sftp: &SftpSession, path: String) -> Result<(), String> {
        let metadata = sftp
            .symlink_metadata(path.clone())
            .await
            .map_err(|e| e.to_string())?;
        if metadata.is_dir() {
            let entries = sftp
                .read_dir(path.clone())
                .await
                .map_err(|e| e.to_string())?;
            for entry in entries {
                self.remove_remote_entry(sftp, entry.path()).await?;
            }
            sftp.remove_dir(path).await.map_err(|e| e.to_string())
        } else {
            sftp.remove_file(path).await.map_err(|e| e.to_string())
        }
    }
    pub async fn preview_remote(
        &self,
        id: &str,
        path: String,
        position: PreviewPosition,
    ) -> Result<FilePreview, String> {
        let sftp = self.sftp(id).await?;
        let metadata = sftp
            .symlink_metadata(path.clone())
            .await
            .map_err(|e| e.to_string())?;
        if !metadata.is_regular() {
            return Err("只能预览普通文件".into());
        }
        let size = metadata.len();
        let length = size.min(1024 * 1024) as usize;
        let mut file = sftp.open(path).await.map_err(|e| e.to_string())?;
        if matches!(position, PreviewPosition::End) {
            file.seek(io::SeekFrom::Start(size.saturating_sub(length as u64)))
                .await
                .map_err(|e| e.to_string())?;
        }
        let mut data = vec![0; length];
        file.read_exact(&mut data)
            .await
            .map_err(|e| e.to_string())?;
        let sample = &data[..data.len().min(8192)];
        let controls = sample
            .iter()
            .filter(|byte| **byte == 0 || **byte < 9 || (**byte > 13 && **byte < 32))
            .count();
        let binary = !sample.is_empty() && controls as f64 / sample.len() as f64 > 0.01;
        Ok(FilePreview {
            content: if binary {
                String::new()
            } else {
                String::from_utf8_lossy(&data).into_owned()
            },
            size,
            truncated: size > 1024 * 1024,
            position,
            binary,
        })
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn parse_remote_entries(output: &str) -> Result<Vec<DirectoryEntry>, RemoteExecError> {
    let (_, framed) = output
        .rsplit_once(REMOTE_LIST_MARKER)
        .ok_or(RemoteExecError::Unavailable)?;
    let payload = framed.lines().next().unwrap_or_default().trim();
    let json = STANDARD
        .decode(payload)
        .map_err(|_| RemoteExecError::Unavailable)?;
    serde_json::from_slice(&json).map_err(|_| RemoteExecError::Unavailable)
}

#[derive(Default)]
struct Utf8Decoder {
    pending: Vec<u8>,
}
impl Utf8Decoder {
    fn push(&mut self, data: &[u8]) -> String {
        self.pending.extend_from_slice(data);
        match std::str::from_utf8(&self.pending) {
            Ok(text) => {
                let value = text.to_string();
                self.pending.clear();
                value
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if error.error_len().is_none() {
                    let tail = self.pending.split_off(valid);
                    let value = String::from_utf8_lossy(&self.pending).into_owned();
                    self.pending = tail;
                    value
                } else {
                    let value = String::from_utf8_lossy(&self.pending).into_owned();
                    self.pending.clear();
                    value
                }
            }
        }
    }
    fn finish(self) -> String {
        String::from_utf8_lossy(&self.pending).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{EntryKind, REMOTE_LIST_MARKER, Utf8Decoder, parse_remote_entries, shell_quote};

    #[test]
    fn quotes_remote_command_arguments() {
        assert_eq!(shell_quote("a'b"), "'a'\"'\"'b'");
    }

    #[test]
    fn parses_fast_remote_directory_response() {
        let payload = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            r#"[{"name":"logs","path":"/srv/logs","kind":"directory"}]"#,
        );
        let entries = parse_remote_entries(&format!("{REMOTE_LIST_MARKER}{payload}\n"))
            .expect("valid directory response");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "logs");
        assert!(matches!(entries[0].kind, EntryKind::Directory));
        assert_eq!(entries[0].size, None);
        assert_eq!(entries[0].modified_at, None);
    }

    #[test]
    fn ignores_shell_output_around_remote_directory_response() {
        let payload = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            r#"[{"name":"logs","path":"/srv/logs","kind":"directory"}]"#,
        );
        let output = format!(
            "Authorized access only\n{REMOTE_LIST_MARKER}{payload}\nSession audit enabled\n"
        );
        assert_eq!(
            parse_remote_entries(&output)
                .expect("framed response")
                .first()
                .map(|entry| entry.name.as_str()),
            Some("logs")
        );
    }

    #[test]
    fn falls_back_to_sftp_when_remote_directory_frame_is_missing() {
        assert!(matches!(
            parse_remote_entries("Welcome to the server\n[]"),
            Err(super::RemoteExecError::Unavailable)
        ));
    }

    #[test]
    fn preserves_split_utf8_sequences() {
        let mut decoder = Utf8Decoder::default();
        let bytes = "你好".as_bytes();
        assert_eq!(decoder.push(&bytes[..2]), "");
        assert_eq!(decoder.push(&bytes[2..4]), "你");
        assert_eq!(decoder.push(&bytes[4..]), "好");
        assert_eq!(decoder.finish(), "");
    }
    #[test]
    fn replaces_invalid_sequences_without_losing_following_text() {
        let mut decoder = Utf8Decoder::default();
        assert_eq!(decoder.push(&[0xff, b'a']), "�a");
    }
}
