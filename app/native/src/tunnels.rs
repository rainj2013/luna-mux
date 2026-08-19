use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{Arc, Mutex},
};

use fast_socks5::{
    server::{AcceptAuthentication, Config, DenyAuthentication, Socks5Socket},
    util::target_addr::TargetAddr,
};
use tauri::{AppHandle, Emitter};
use tokio::{
    io::{AsyncWriteExt, copy_bidirectional},
    net::{TcpListener, TcpStream},
    sync::oneshot,
};
use uuid::Uuid;

use crate::{models::*, sessions::SessionManager};

struct ActiveTunnel {
    summary: Mutex<TunnelSummary>,
    cancel: Mutex<Option<oneshot::Sender<()>>>,
    remote_binding: Option<(String, u16)>,
}

pub struct TunnelManager {
    app: AppHandle,
    sessions: Arc<SessionManager>,
    tunnels: Mutex<HashMap<String, Arc<ActiveTunnel>>>,
}

impl TunnelManager {
    pub fn new(app: AppHandle, sessions: Arc<SessionManager>) -> Arc<Self> {
        Arc::new(Self {
            app,
            sessions,
            tunnels: Mutex::new(HashMap::new()),
        })
    }

    pub fn list(&self, session_id: &str) -> Vec<TunnelSummary> {
        self.tunnels
            .lock()
            .map(|items| {
                items
                    .values()
                    .map(|item| item.summary.lock().expect("tunnel summary lock").clone())
                    .filter(|item| item.session_id == session_id)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn get(&self, tunnel_id: &str) -> Option<TunnelSummary> {
        self.tunnels
            .lock()
            .ok()
            .and_then(|items| items.get(tunnel_id).cloned())
            .and_then(|item| item.summary.lock().ok().map(|summary| summary.clone()))
    }

    pub async fn start(
        self: &Arc<Self>,
        session_id: String,
        profile: PortForwardProfile,
    ) -> Result<TunnelSummary, String> {
        validate_dynamic_bind_address(&profile)?;
        if self.sessions.bookmark_id(&session_id)? != profile.bookmark_id {
            return Err("端口转发配置与当前连接不匹配".into());
        }
        if let Some(existing) = self
            .list(&session_id)
            .into_iter()
            .find(|item| item.profile_id == profile.id)
        {
            return Ok(existing);
        }
        let id = Uuid::new_v4().to_string();
        let summary = TunnelSummary {
            id: id.clone(),
            profile_id: profile.id,
            session_id: session_id.clone(),
            name: profile.name,
            forward_type: profile.forward_type.clone(),
            bind_address: profile.bind_address.clone(),
            bind_port: profile.bind_port,
            target_host: profile.target_host.clone(),
            target_port: profile.target_port,
            status: TunnelStatus::Starting,
            error: None,
            removed: false,
        };
        let (sender, receiver) = oneshot::channel();
        let (active, task) = match profile.forward_type {
            PortForwardType::Remote => {
                let (assigned, channels) = self
                    .sessions
                    .request_remote_forward(
                        &session_id,
                        profile.bind_address.clone(),
                        profile.bind_port,
                    )
                    .await?;
                let mut running = summary.clone();
                running.bind_port = assigned;
                running.status = TunnelStatus::Running;
                let active = Arc::new(ActiveTunnel {
                    summary: Mutex::new(running.clone()),
                    cancel: Mutex::new(Some(sender)),
                    remote_binding: Some((profile.bind_address, assigned)),
                });
                let manager = self.clone();
                let active_task = active.clone();
                let task = async move { manager.run_remote(active_task, channels, receiver).await }
                    .boxed();
                (active, task)
            }
            PortForwardType::Local | PortForwardType::Dynamic => {
                let listener =
                    TcpListener::bind((profile.bind_address.as_str(), profile.bind_port))
                        .await
                        .map_err(|e| e.to_string())?;
                let assigned = listener.local_addr().map_err(|e| e.to_string())?.port();
                let mut running = summary.clone();
                running.bind_port = assigned;
                running.status = TunnelStatus::Running;
                let active = Arc::new(ActiveTunnel {
                    summary: Mutex::new(running.clone()),
                    cancel: Mutex::new(Some(sender)),
                    remote_binding: None,
                });
                let manager = self.clone();
                let active_task = active.clone();
                let dynamic = profile.forward_type == PortForwardType::Dynamic;
                let task = async move {
                    manager
                        .run_local(active_task, listener, receiver, dynamic)
                        .await
                }
                .boxed();
                (active, task)
            }
        };
        let running = active
            .summary
            .lock()
            .map_err(|_| "端口转发状态锁已损坏")?
            .clone();
        self.tunnels
            .lock()
            .map_err(|_| "端口转发列表锁已损坏")?
            .insert(id, active.clone());
        self.emit(running.clone());
        tauri::async_runtime::spawn(task);
        let manager = self.clone();
        let monitor_session = session_id;
        let monitor_id = running.id.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                if !manager
                    .tunnels
                    .lock()
                    .map(|items| items.contains_key(&monitor_id))
                    .unwrap_or(false)
                {
                    break;
                }
                if manager.sessions.get(&monitor_session).is_err() {
                    let _ = manager.stop(&monitor_session, &monitor_id).await;
                    break;
                }
            }
        });
        Ok(running)
    }

    pub async fn stop(&self, session_id: &str, tunnel_id: &str) -> Result<(), String> {
        let active = self
            .tunnels
            .lock()
            .map_err(|_| "端口转发列表锁已损坏")?
            .get(tunnel_id)
            .cloned();
        let Some(active) = active else {
            return Ok(());
        };
        let summary = active
            .summary
            .lock()
            .map_err(|_| "端口转发状态锁已损坏")?
            .clone();
        if summary.session_id != session_id {
            return Err("端口转发不属于当前会话".into());
        }
        self.tunnels
            .lock()
            .map_err(|_| "端口转发列表锁已损坏")?
            .remove(tunnel_id);
        if let Ok(mut cancel) = active.cancel.lock() {
            if let Some(sender) = cancel.take() {
                let _ = sender.send(());
            }
        }
        let remote_result = if let Some((address, port)) = &active.remote_binding {
            self.sessions
                .cancel_remote_forward(session_id, address.clone(), *port)
                .await
        } else {
            Ok(())
        };
        let mut removed = summary;
        removed.removed = true;
        self.emit(removed);
        remote_result
    }

    pub async fn stop_session(&self, session_id: &str) {
        let ids = self
            .list(session_id)
            .into_iter()
            .map(|item| item.id)
            .collect::<Vec<_>>();
        for id in ids {
            let _ = self.stop(session_id, &id).await;
        }
    }

    async fn run_local(
        self: Arc<Self>,
        active: Arc<ActiveTunnel>,
        listener: TcpListener,
        mut cancel: oneshot::Receiver<()>,
        dynamic: bool,
    ) {
        loop {
            tokio::select! {
                _ = &mut cancel => break,
                accepted = listener.accept() => match accepted {
                    Ok((socket, peer)) => {
                        let manager = self.clone();
                        let summary = active
                            .summary
                            .lock()
                            .expect("tunnel summary lock")
                            .clone();
                        tauri::async_runtime::spawn(async move {
                            let result = if dynamic {
                                manager
                                    .proxy_dynamic(
                                        &summary,
                                        socket,
                                        peer.ip().to_string(),
                                        peer.port(),
                                    )
                                    .await
                            } else {
                                manager
                                    .proxy_direct(
                                        &summary,
                                        socket,
                                        peer.ip().to_string(),
                                        peer.port(),
                                    )
                                    .await
                            };
                            if let Err(error) = result {
                                manager.runtime_error(&summary.id, error);
                            }
                        });
                    }
                    Err(error) => {
                        let id = active
                            .summary
                            .lock()
                            .expect("tunnel summary lock")
                            .id
                            .clone();
                        self.runtime_error(&id, error.to_string());
                        break;
                    }
                }
            }
        }
    }

    async fn proxy_direct(
        &self,
        summary: &TunnelSummary,
        mut socket: TcpStream,
        origin_host: String,
        origin_port: u16,
    ) -> Result<(), String> {
        let channel = self
            .sessions
            .direct_tcpip(
                &summary.session_id,
                summary.target_host.clone(),
                summary.target_port,
                origin_host,
                origin_port,
            )
            .await?;
        let mut stream = channel.into_stream();
        copy_bidirectional(&mut socket, &mut stream)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn proxy_dynamic(
        &self,
        summary: &TunnelSummary,
        socket: TcpStream,
        origin_host: String,
        origin_port: u16,
    ) -> Result<(), String> {
        let mut config: Config<DenyAuthentication> = Config::default();
        config
            .set_allow_no_auth(true)
            .set_execute_command(false)
            .set_dns_resolve(false);
        let config = Arc::new(config.with_authentication(AcceptAuthentication::default()));
        let mut socks = Socks5Socket::new(socket, config)
            .upgrade_to_socks5()
            .await
            .map_err(|e| e.to_string())?;
        let (target, port) = match socks
            .target_addr()
            .ok_or_else(|| "SOCKS5 请求没有目标地址".to_string())?
        {
            TargetAddr::Ip(address) => (address.ip().to_string(), address.port()),
            TargetAddr::Domain(host, port) => (host.clone(), *port),
        };
        let channel = self
            .sessions
            .direct_tcpip(&summary.session_id, target, port, origin_host, origin_port)
            .await?;
        socks
            .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
            .await
            .map_err(|e| e.to_string())?;
        let mut socket = socks.into_inner();
        let mut stream = channel.into_stream();
        copy_bidirectional(&mut socket, &mut stream)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    async fn run_remote(
        self: Arc<Self>,
        active: Arc<ActiveTunnel>,
        mut channels: tokio::sync::mpsc::UnboundedReceiver<russh::Channel<russh::client::Msg>>,
        mut cancel: oneshot::Receiver<()>,
    ) {
        loop {
            tokio::select! {
                _ = &mut cancel => break,
                channel = channels.recv() => {
                    let Some(channel) = channel else { break };
                    let summary = active
                        .summary
                        .lock()
                        .expect("tunnel summary lock")
                        .clone();
                    let manager = self.clone();
                    tauri::async_runtime::spawn(async move {
                        let result = async {
                            let mut socket = TcpStream::connect((
                                summary.target_host.as_str(),
                                summary.target_port,
                            ))
                            .await
                            .map_err(|error| error.to_string())?;
                            let mut stream = channel.into_stream();
                            copy_bidirectional(&mut socket, &mut stream)
                                .await
                                .map_err(|error| error.to_string())?;
                            Ok::<(), String>(())
                        }
                        .await;
                        if let Err(error) = result {
                            manager.runtime_error(&summary.id, error);
                        }
                    });
                }
            }
        }
    }

    fn runtime_error(&self, id: &str, error: String) {
        let active = self
            .tunnels
            .lock()
            .ok()
            .and_then(|items| items.get(id).cloned());
        if let Some(active) = active {
            let summary = {
                let mut summary = active.summary.lock().expect("tunnel summary lock");
                summary.status = TunnelStatus::Error;
                summary.error = Some(error);
                summary.clone()
            };
            self.emit(summary);
        }
    }
    fn emit(&self, summary: TunnelSummary) {
        let _ = self.app.emit("app:event", AppEvent::Tunnel(summary));
    }
}

pub(crate) fn validate_dynamic_bind_address(profile: &PortForwardProfile) -> Result<(), String> {
    if profile.forward_type != PortForwardType::Dynamic {
        return Ok(());
    }
    let address = profile
        .bind_address
        .trim()
        .parse::<IpAddr>()
        .map_err(|_| "动态 SOCKS5 监听地址必须是回环 IP（127.0.0.1 或 ::1）".to_string())?;
    if !address.is_loopback() {
        return Err("动态 SOCKS5 只能监听回环地址，避免暴露未认证代理".into());
    }
    Ok(())
}

use futures::FutureExt;

#[cfg(test)]
mod tests {
    use super::validate_dynamic_bind_address;
    use crate::models::{PortForwardProfile, PortForwardType};

    fn profile(bind_address: &str, forward_type: PortForwardType) -> PortForwardProfile {
        PortForwardProfile {
            id: "profile".into(),
            bookmark_id: "bookmark".into(),
            name: "test".into(),
            forward_type,
            bind_address: bind_address.into(),
            bind_port: 0,
            target_host: String::new(),
            target_port: 0,
        }
    }

    #[test]
    fn dynamic_forwarding_is_loopback_only() {
        assert!(validate_dynamic_bind_address(&profile("127.0.0.1", PortForwardType::Dynamic)).is_ok());
        assert!(validate_dynamic_bind_address(&profile("::1", PortForwardType::Dynamic)).is_ok());
        assert!(validate_dynamic_bind_address(&profile("0.0.0.0", PortForwardType::Dynamic)).is_err());
        assert!(validate_dynamic_bind_address(&profile("example.com", PortForwardType::Dynamic)).is_err());
        assert!(validate_dynamic_bind_address(&profile("0.0.0.0", PortForwardType::Local)).is_ok());
    }
}
