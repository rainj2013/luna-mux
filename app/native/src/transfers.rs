use std::{
    collections::{HashMap, VecDeque},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use async_recursion::async_recursion;
use chrono::Utc;
use russh_sftp::client::SftpSession;
use tauri::{AppHandle, Emitter};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{Semaphore, oneshot},
};
use uuid::Uuid;

use crate::{database::Database, models::*, sessions::SessionManager};

struct InternalTask {
    task: Mutex<TransferTask>,
    batch_id: String,
    cancelled: AtomicBool,
}

struct ConflictWaiter {
    task: Arc<InternalTask>,
    destination_path: String,
    sender: oneshot::Sender<ConflictResolution>,
}

#[derive(Default)]
struct ConflictState {
    waiters: HashMap<String, ConflictWaiter>,
    queue: VecDeque<String>,
    active: Option<String>,
    batch_resolutions: HashMap<String, ConflictResolution>,
}

pub struct TransferManager {
    app: AppHandle,
    db: Arc<Database>,
    sessions: Arc<SessionManager>,
    tasks: Mutex<HashMap<String, Arc<InternalTask>>>,
    conflicts: Mutex<ConflictState>,
    concurrency: Arc<Semaphore>,
}

impl TransferManager {
    pub fn new(app: AppHandle, db: Arc<Database>, sessions: Arc<SessionManager>) -> Arc<Self> {
        Arc::new(Self {
            app,
            db,
            sessions,
            tasks: Mutex::new(HashMap::new()),
            conflicts: Mutex::new(ConflictState::default()),
            concurrency: Arc::new(Semaphore::new(3)),
        })
    }

    pub fn list(&self) -> Result<Vec<TransferTask>, String> {
        self.db.list_transfers()
    }

    pub fn enqueue(
        self: &Arc<Self>,
        request: TransferRequest,
    ) -> Result<Vec<TransferTask>, String> {
        if request.source_paths.is_empty() {
            return Ok(vec![]);
        }
        let bookmark_id = self.sessions.bookmark_id(&request.session_id)?;
        let now = Utc::now().to_rfc3339();
        let batch_id = Uuid::new_v4().to_string();
        let mut result = Vec::new();
        for source in &request.source_paths {
            let name = if request.direction == TransferDirection::Upload {
                local_name(source)
            } else {
                remote_name(source)
            };
            if name.is_empty() {
                return Err(format!("无法确定传输项目名称：{source}"));
            }
            let destination = if request.direction == TransferDirection::Upload {
                remote_join(&request.destination_directory, &name)
            } else {
                Path::new(&request.destination_directory)
                    .join(&name)
                    .to_string_lossy()
                    .into_owned()
            };
            let task = TransferTask {
                id: Uuid::new_v4().to_string(),
                session_id: request.session_id.clone(),
                bookmark_id: bookmark_id.clone(),
                direction: request.direction.clone(),
                source_path: source.clone(),
                destination_path: destination,
                display_name: name,
                status: TransferStatus::Queued,
                bytes_total: 0,
                bytes_transferred: 0,
                speed: 0.0,
                error: None,
                created_at: now.clone(),
                updated_at: now.clone(),
            };
            let internal = Arc::new(InternalTask {
                task: Mutex::new(task.clone()),
                batch_id: batch_id.clone(),
                cancelled: AtomicBool::new(false),
            });
            self.tasks
                .lock()
                .map_err(|_| "传输队列锁已损坏")?
                .insert(task.id.clone(), internal.clone());
            self.persist(&task);
            result.push(task);
            let manager = self.clone();
            tauri::async_runtime::spawn(async move {
                manager.run(internal).await;
            });
        }
        Ok(result)
    }

    pub fn cancel(&self, id: &str) {
        let internal = self
            .tasks
            .lock()
            .ok()
            .and_then(|items| items.get(id).cloned());
        let Some(internal) = internal else {
            return;
        };
        internal.cancelled.store(true, Ordering::Release);
        self.resolve_conflict(id, ConflictResolution::Skip, false);
        self.mutate(&internal, |task| {
            task.status = TransferStatus::Cancelled;
            task.error = None;
            task.speed = 0.0;
        });
    }

    pub fn retry(self: &Arc<Self>, id: &str, session_id: String) -> Result<(), String> {
        let old = self
            .db
            .list_transfers()?
            .into_iter()
            .find(|task| task.id == id)
            .ok_or_else(|| "传输记录不存在".to_string())?;
        if !matches!(
            old.status,
            TransferStatus::Failed | TransferStatus::Interrupted | TransferStatus::Cancelled
        ) {
            return Ok(());
        }
        if self.sessions.bookmark_id(&session_id)? != old.bookmark_id {
            return Err("请先连接此传输任务对应的连接".into());
        }
        let request = TransferRequest {
            session_id,
            direction: old.direction.clone(),
            source_paths: vec![old.source_path],
            destination_directory: if old.direction == TransferDirection::Upload {
                remote_parent(&old.destination_path)
            } else {
                Path::new(&old.destination_path)
                    .parent()
                    .unwrap_or(Path::new(""))
                    .to_string_lossy()
                    .into_owned()
            },
        };
        let _ = self.enqueue(request)?;
        Ok(())
    }

    pub fn clear_completed(&self) -> Result<(), String> {
        self.db.clear_completed()
    }

    pub fn resolve_conflict(&self, id: &str, resolution: ConflictResolution, apply_to_batch: bool) {
        let (senders, prompt) = self.settle_conflict(id, resolution, apply_to_batch);
        for (sender, value) in senders {
            let _ = sender.send(value);
        }
        if let Some(prompt) = prompt {
            self.emit_conflict(prompt);
        }
    }

    pub async fn when_settled(&self, ids: &[String]) -> bool {
        loop {
            let Ok(tasks) = self.db.list_transfers() else {
                return false;
            };
            let selected: Vec<_> = tasks
                .into_iter()
                .filter(|task| ids.contains(&task.id))
                .collect();
            if selected.len() != ids.len() {
                return false;
            }
            if selected.iter().all(|task| {
                !matches!(
                    task.status,
                    TransferStatus::Queued
                        | TransferStatus::Scanning
                        | TransferStatus::Running
                        | TransferStatus::Conflict
                )
            }) {
                return selected
                    .iter()
                    .all(|task| task.status == TransferStatus::Completed);
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    async fn run(self: Arc<Self>, internal: Arc<InternalTask>) {
        let permit = match self.concurrency.clone().acquire_owned().await {
            Ok(value) => value,
            Err(_) => return,
        };
        if internal.cancelled.load(Ordering::Acquire) {
            drop(permit);
            self.finish(&internal);
            return;
        }
        let result = self.run_task(&internal).await;
        match result {
            _ if internal.cancelled.load(Ordering::Acquire) => self.mutate(&internal, |task| {
                task.status = TransferStatus::Cancelled;
                task.speed = 0.0;
                task.error = None;
            }),
            Ok(()) => self.mutate(&internal, |task| {
                task.status = TransferStatus::Completed;
                task.speed = 0.0;
                task.error = None;
            }),
            Err(error) => self.mutate(&internal, |task| {
                task.status = TransferStatus::Failed;
                task.speed = 0.0;
                task.error = Some(error);
            }),
        }
        drop(permit);
        self.finish(&internal);
    }

    async fn run_task(&self, internal: &Arc<InternalTask>) -> Result<(), String> {
        self.mutate(internal, |task| task.status = TransferStatus::Scanning);
        let snapshot = self.snapshot(internal);
        let size = if snapshot.direction == TransferDirection::Upload {
            self.local_size(&snapshot.source_path).await?
        } else {
            self.remote_size(&snapshot.session_id, snapshot.source_path.clone())
                .await?
        };
        self.mutate(internal, |task| {
            task.bytes_total = size;
            task.status = TransferStatus::Running;
        });
        self.assert_active(internal)?;
        if snapshot.direction == TransferDirection::Upload {
            self.upload_entry(internal, snapshot.source_path, snapshot.destination_path)
                .await
        } else {
            self.download_entry(internal, snapshot.source_path, snapshot.destination_path)
                .await
        }
    }

    #[async_recursion]
    async fn local_size(&self, path: &str) -> Result<u64, String> {
        let metadata = tokio::fs::symlink_metadata(path)
            .await
            .map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err(format!("暂不支持传输符号链接：{path}"));
        }
        if metadata.is_file() {
            return Ok(metadata.len());
        }
        if !metadata.is_dir() {
            return Ok(0);
        }
        let mut total = 0;
        let mut entries = tokio::fs::read_dir(path).await.map_err(|e| e.to_string())?;
        while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
            total += self.local_size(&entry.path().to_string_lossy()).await?;
        }
        Ok(total)
    }

    #[async_recursion]
    async fn remote_size(&self, session_id: &str, path: String) -> Result<u64, String> {
        let sftp = self.sessions.sftp(session_id).await?;
        let metadata = sftp
            .symlink_metadata(path.clone())
            .await
            .map_err(|e| e.to_string())?;
        if metadata.is_symlink() {
            return Err(format!("暂不支持传输符号链接：{path}"));
        }
        if !metadata.is_dir() {
            return Ok(metadata.len());
        }
        let mut total = 0;
        for entry in sftp.read_dir(path).await.map_err(|e| e.to_string())? {
            total += self.remote_size(session_id, entry.path()).await?;
        }
        Ok(total)
    }

    #[async_recursion]
    async fn upload_entry(
        &self,
        internal: &Arc<InternalTask>,
        source: String,
        destination: String,
    ) -> Result<(), String> {
        self.assert_active(internal)?;
        let metadata = tokio::fs::symlink_metadata(&source)
            .await
            .map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() {
            return Err(format!("暂不支持传输符号链接：{source}"));
        }
        let session_id = self.snapshot(internal).session_id;
        let sftp = self.sessions.sftp(&session_id).await?;
        if metadata.is_dir() {
            self.ensure_remote_directory(&sftp, &destination).await?;
            let mut entries = tokio::fs::read_dir(&source)
                .await
                .map_err(|e| e.to_string())?;
            while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
                let name = entry.file_name().to_string_lossy().into_owned();
                self.upload_entry(
                    internal,
                    entry.path().to_string_lossy().into_owned(),
                    remote_join(&destination, &name),
                )
                .await?;
            }
            return Ok(());
        }
        if !metadata.is_file() {
            return Ok(());
        }
        let Some((destination, overwrite)) = self
            .resolve_destination(internal, destination, true)
            .await?
        else {
            self.add_progress(internal, metadata.len(), 0.0);
            return Ok(());
        };
        self.ensure_remote_directory(&sftp, &remote_parent(&destination))
            .await?;
        let temporary = format!("{}.{}.part", destination, Uuid::new_v4());
        let mut reader = tokio::fs::File::open(&source)
            .await
            .map_err(|e| e.to_string())?;
        let mut writer = sftp
            .create(temporary.clone())
            .await
            .map_err(|e| e.to_string())?;
        let result = self.copy(internal, &mut reader, &mut writer).await;
        let _ = writer.shutdown().await;
        if let Err(error) = result {
            let _ = sftp.remove_file(temporary).await;
            return Err(error);
        }
        self.commit_remote(&sftp, temporary, destination, overwrite)
            .await
    }

    #[async_recursion]
    async fn download_entry(
        &self,
        internal: &Arc<InternalTask>,
        source: String,
        destination: String,
    ) -> Result<(), String> {
        self.assert_active(internal)?;
        let session_id = self.snapshot(internal).session_id;
        let sftp = self.sessions.sftp(&session_id).await?;
        let metadata = sftp
            .symlink_metadata(source.clone())
            .await
            .map_err(|e| e.to_string())?;
        if metadata.is_symlink() {
            return Err(format!("暂不支持传输符号链接：{source}"));
        }
        if metadata.is_dir() {
            tokio::fs::create_dir_all(&destination)
                .await
                .map_err(|e| e.to_string())?;
            for entry in sftp.read_dir(source).await.map_err(|e| e.to_string())? {
                let name = entry.file_name();
                self.download_entry(
                    internal,
                    entry.path(),
                    Path::new(&destination)
                        .join(name)
                        .to_string_lossy()
                        .into_owned(),
                )
                .await?;
            }
            return Ok(());
        }
        let Some((destination, overwrite)) = self
            .resolve_destination(internal, destination, false)
            .await?
        else {
            self.add_progress(internal, metadata.len(), 0.0);
            return Ok(());
        };
        if let Some(parent) = Path::new(&destination).parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| e.to_string())?;
        }
        let temporary = format!("{}.{}.part", destination, Uuid::new_v4());
        let mut reader = sftp.open(source).await.map_err(|e| e.to_string())?;
        let mut writer = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .await
            .map_err(|e| e.to_string())?;
        let result = self.copy(internal, &mut reader, &mut writer).await;
        let _ = writer.shutdown().await;
        if let Err(error) = result {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(error);
        }
        self.commit_local(temporary, destination, overwrite).await
    }

    async fn copy<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
        &self,
        internal: &Arc<InternalTask>,
        reader: &mut R,
        writer: &mut W,
    ) -> Result<(), String> {
        let mut buffer = vec![0u8; 64 * 1024];
        let mut last = Instant::now();
        let mut last_bytes = self.snapshot(internal).bytes_transferred;
        loop {
            self.assert_active(internal)?;
            let count = tokio::time::timeout(Duration::from_secs(60), reader.read(&mut buffer))
                .await
                .map_err(|_| "文件传输 60 秒没有进展，已中止".to_string())?
                .map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            tokio::time::timeout(Duration::from_secs(60), writer.write_all(&buffer[..count]))
                .await
                .map_err(|_| "文件传输 60 秒没有进展，已中止".to_string())?
                .map_err(|e| e.to_string())?;
            let elapsed = last.elapsed();
            let current = self.snapshot(internal).bytes_transferred + count as u64;
            let speed = if elapsed >= Duration::from_millis(250) {
                (current - last_bytes) as f64 / elapsed.as_secs_f64()
            } else {
                -1.0
            };
            self.add_progress(internal, count as u64, speed);
            if speed >= 0.0 {
                last = Instant::now();
                last_bytes = current;
            }
        }
        writer.flush().await.map_err(|e| e.to_string())
    }

    async fn resolve_destination(
        &self,
        internal: &Arc<InternalTask>,
        destination: String,
        remote: bool,
    ) -> Result<Option<(String, bool)>, String> {
        if !self
            .exists(&self.snapshot(internal).session_id, &destination, remote)
            .await?
        {
            return Ok(Some((destination, false)));
        }
        let resolution = self.ask_conflict(internal, destination.clone()).await?;
        match resolution {
            ConflictResolution::Skip => Ok(None),
            ConflictResolution::Overwrite => Ok(Some((destination, true))),
            ConflictResolution::Rename => {
                let mut attempt = 1;
                loop {
                    let candidate = renamed_path(&destination, attempt, remote);
                    if !self
                        .exists(&self.snapshot(internal).session_id, &candidate, remote)
                        .await?
                    {
                        return Ok(Some((candidate, false)));
                    }
                    attempt += 1;
                }
            }
        }
    }

    async fn ask_conflict(
        &self,
        internal: &Arc<InternalTask>,
        destination_path: String,
    ) -> Result<ConflictResolution, String> {
        self.mutate(internal, |task| task.status = TransferStatus::Conflict);
        let (sender, receiver) = oneshot::channel();
        let id = self.snapshot(internal).id;
        let prompt = {
            let mut state = self.conflicts.lock().map_err(|_| "传输冲突锁已损坏")?;
            if let Some(value) = state.batch_resolutions.get(&internal.batch_id).cloned() {
                return Ok(value);
            }
            state.waiters.insert(
                id.clone(),
                ConflictWaiter {
                    task: internal.clone(),
                    destination_path,
                    sender,
                },
            );
            state.queue.push_back(id);
            next_conflict(&mut state)
        };
        if let Some(prompt) = prompt {
            self.emit_conflict(prompt);
        }
        receiver.await.map_err(|_| "传输冲突处理已取消".to_string())
    }

    fn settle_conflict(
        &self,
        id: &str,
        resolution: ConflictResolution,
        apply_to_batch: bool,
    ) -> (
        Vec<(oneshot::Sender<ConflictResolution>, ConflictResolution)>,
        Option<TransferConflict>,
    ) {
        let mut senders = Vec::new();
        let mut prompt = None;
        if let Ok(mut state) = self.conflicts.lock() {
            if state.active.as_deref() != Some(id) {
                return (senders, prompt);
            }
            state.active = None;
            if let Some(waiter) = state.waiters.remove(id) {
                if apply_to_batch {
                    state
                        .batch_resolutions
                        .insert(waiter.task.batch_id.clone(), resolution.clone());
                }
                self.mutate(&waiter.task, |task| task.status = TransferStatus::Running);
                senders.push((waiter.sender, resolution));
            }
            loop {
                let Some(next) = state.queue.pop_front() else {
                    break;
                };
                let Some(waiter) = state.waiters.remove(&next) else {
                    continue;
                };
                if let Some(value) = state.batch_resolutions.get(&waiter.task.batch_id).cloned() {
                    self.mutate(&waiter.task, |task| task.status = TransferStatus::Running);
                    senders.push((waiter.sender, value));
                    continue;
                }
                prompt = Some(TransferConflict {
                    task_id: next.clone(),
                    source_path: self.snapshot(&waiter.task).source_path,
                    destination_path: waiter.destination_path.clone(),
                });
                state.waiters.insert(next.clone(), waiter);
                state.active = Some(next);
                break;
            }
        }
        (senders, prompt)
    }

    async fn exists(&self, session_id: &str, path: &str, remote: bool) -> Result<bool, String> {
        if remote {
            self.sessions
                .sftp(session_id)
                .await?
                .try_exists(path)
                .await
                .map_err(|e| e.to_string())
        } else {
            match tokio::fs::symlink_metadata(path).await {
                Ok(_) => Ok(true),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(error.to_string()),
            }
        }
    }

    async fn ensure_remote_directory(&self, sftp: &SftpSession, path: &str) -> Result<(), String> {
        if path.is_empty() || path == "/" || path == "." {
            return Ok(());
        }
        let mut current = if path.starts_with('/') {
            "/".to_string()
        } else {
            String::new()
        };
        for part in path.split('/').filter(|part| !part.is_empty()) {
            current = remote_join(&current, part);
            if sftp
                .try_exists(current.clone())
                .await
                .map_err(|e| e.to_string())?
            {
                if !sftp
                    .symlink_metadata(current.clone())
                    .await
                    .map_err(|e| e.to_string())?
                    .is_dir()
                {
                    return Err(format!("{current} 不是目录"));
                }
            } else {
                sftp.create_dir(current.clone())
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    async fn commit_local(
        &self,
        temporary: String,
        destination: String,
        overwrite: bool,
    ) -> Result<(), String> {
        if !overwrite
            && tokio::fs::try_exists(&destination)
                .await
                .map_err(|e| e.to_string())?
        {
            return Err(format!("目标文件在传输期间已出现：{destination}"));
        }
        if overwrite
            && tokio::fs::try_exists(&destination)
                .await
                .map_err(|e| e.to_string())?
        {
            if tokio::fs::symlink_metadata(&destination)
                .await
                .map_err(|e| e.to_string())?
                .is_dir()
            {
                return Err(format!("不能用文件覆盖目录：{destination}"));
            }
            tokio::fs::remove_file(&destination)
                .await
                .map_err(|e| e.to_string())?;
        }
        tokio::fs::rename(temporary, destination)
            .await
            .map_err(|e| e.to_string())
    }

    async fn commit_remote(
        &self,
        sftp: &SftpSession,
        temporary: String,
        destination: String,
        overwrite: bool,
    ) -> Result<(), String> {
        let exists = sftp
            .try_exists(destination.clone())
            .await
            .map_err(|e| e.to_string())?;
        if !overwrite && exists {
            return Err(format!("目标文件在传输期间已出现：{destination}"));
        }
        if exists {
            if sftp
                .symlink_metadata(destination.clone())
                .await
                .map_err(|e| e.to_string())?
                .is_dir()
            {
                return Err(format!("不能用文件覆盖目录：{destination}"));
            }
            sftp.remove_file(destination.clone())
                .await
                .map_err(|e| e.to_string())?;
        }
        sftp.rename(temporary, destination)
            .await
            .map_err(|e| e.to_string())
    }

    fn assert_active(&self, internal: &Arc<InternalTask>) -> Result<(), String> {
        if internal.cancelled.load(Ordering::Acquire) {
            Err("已取消".into())
        } else {
            Ok(())
        }
    }
    fn snapshot(&self, internal: &Arc<InternalTask>) -> TransferTask {
        internal.task.lock().expect("transfer task lock").clone()
    }
    fn mutate(&self, internal: &Arc<InternalTask>, operation: impl FnOnce(&mut TransferTask)) {
        let task = {
            let mut task = internal.task.lock().expect("transfer task lock");
            operation(&mut task);
            task.updated_at = Utc::now().to_rfc3339();
            task.clone()
        };
        self.persist(&task);
    }
    fn add_progress(&self, internal: &Arc<InternalTask>, bytes: u64, speed: f64) {
        self.mutate(internal, |task| {
            task.bytes_transferred = task
                .bytes_transferred
                .saturating_add(bytes)
                .min(task.bytes_total);
            if speed >= 0.0 {
                task.speed = speed;
            }
        });
    }
    fn persist(&self, task: &TransferTask) {
        let _ = self.db.save_transfer(task);
        let _ = self.app.emit("app:event", AppEvent::Transfer(task.clone()));
    }
    fn emit_conflict(&self, prompt: TransferConflict) {
        let _ = self
            .app
            .emit("app:event", AppEvent::TransferConflict(prompt));
    }
    fn finish(&self, internal: &Arc<InternalTask>) {
        let id = self.snapshot(internal).id;
        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.remove(&id);
            let batch_active = tasks
                .values()
                .any(|task| task.batch_id == internal.batch_id);
            if !batch_active {
                if let Ok(mut conflicts) = self.conflicts.lock() {
                    conflicts.batch_resolutions.remove(&internal.batch_id);
                }
            }
        }
    }
}

fn next_conflict(state: &mut ConflictState) -> Option<TransferConflict> {
    if state.active.is_some() {
        return None;
    }
    while let Some(id) = state.queue.pop_front() {
        let Some(waiter) = state.waiters.get(&id) else {
            continue;
        };
        let Some((source_path, destination_path)) = waiter
            .task
            .task
            .lock()
            .ok()
            .map(|task| (task.source_path.clone(), waiter.destination_path.clone()))
        else {
            state.waiters.remove(&id);
            continue;
        };
        state.active = Some(id.clone());
        return Some(TransferConflict {
            task_id: id,
            source_path,
            destination_path,
        });
    }
    None
}

fn local_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}
fn remote_name(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string()
}
fn remote_parent(path: &str) -> String {
    let path = path.trim_end_matches('/');
    match path.rfind('/') {
        Some(0) => "/".into(),
        Some(index) => path[..index].to_string(),
        None => ".".into(),
    }
}
fn remote_join(parent: &str, child: &str) -> String {
    if parent == "/" {
        format!("/{}", child.trim_matches('/'))
    } else if parent.is_empty() || parent == "." {
        child.trim_matches('/').to_string()
    } else {
        format!(
            "{}/{}",
            parent.trim_end_matches('/'),
            child.trim_matches('/')
        )
    }
}
fn renamed_path(path: &str, attempt: u32, remote: bool) -> String {
    if remote {
        let parent = remote_parent(path);
        let name = remote_name(path);
        let (stem, extension) = name
            .rsplit_once('.')
            .filter(|(stem, _)| !stem.is_empty())
            .map(|(a, b)| (a, format!(".{b}")))
            .unwrap_or((&name, String::new()));
        remote_join(&parent, &format!("{stem} ({attempt}){extension}"))
    } else {
        let value = Path::new(path);
        let parent = value.parent().unwrap_or(Path::new(""));
        let stem = value.file_stem().unwrap_or_default().to_string_lossy();
        let extension = value
            .extension()
            .map(|value| format!(".{}", value.to_string_lossy()))
            .unwrap_or_default();
        parent
            .join(format!("{stem} ({attempt}){extension}"))
            .to_string_lossy()
            .into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conflict_task(id: &str) -> Arc<InternalTask> {
        Arc::new(InternalTask {
            task: Mutex::new(TransferTask {
                id: id.into(),
                session_id: "session-1".into(),
                bookmark_id: "bookmark-1".into(),
                direction: TransferDirection::Upload,
                source_path: format!("/tmp/{id}.txt"),
                destination_path: format!("/remote/{id}.txt"),
                display_name: format!("{id}.txt"),
                status: TransferStatus::Conflict,
                bytes_total: 0,
                bytes_transferred: 0,
                speed: 0.0,
                error: None,
                created_at: String::new(),
                updated_at: String::new(),
            }),
            batch_id: "batch-1".into(),
            cancelled: AtomicBool::new(false),
        })
    }

    #[test]
    fn remote_paths_are_platform_independent() {
        assert_eq!(remote_join("/tmp", "a.txt"), "/tmp/a.txt");
        assert_eq!(remote_parent("/tmp/a.txt"), "/tmp");
        assert_eq!(renamed_path("/tmp/a.txt", 2, true), "/tmp/a (2).txt");
    }

    #[test]
    fn stale_conflict_queue_entries_do_not_block_the_next_prompt() {
        let mut state = ConflictState::default();
        state.queue.push_back("stale".into());
        state.queue.push_back("ready".into());
        let (sender, _receiver) = oneshot::channel();
        state.waiters.insert(
            "ready".into(),
            ConflictWaiter {
                task: conflict_task("ready"),
                destination_path: "/remote/ready.txt".into(),
                sender,
            },
        );

        let prompt = next_conflict(&mut state).expect("the valid conflict is selected");
        assert_eq!(prompt.task_id, "ready");
        assert_eq!(state.active.as_deref(), Some("ready"));
    }
}
