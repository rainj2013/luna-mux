//! A request bridge to mounted database UI controls. This module never accesses
//! database connections, credentials, SQL drivers, or arbitrary webview scripts.
use std::{collections::HashMap, sync::Mutex, time::Duration};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::oneshot;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
pub enum PaneUiAction {
    Snapshot,
    Fill { r#ref: String, value: String },
    Click { r#ref: String },
}

impl PaneUiAction {
    pub fn validate(&self) -> Result<(), String> {
        let reference = match self {
            Self::Snapshot => return Ok(()),
            Self::Fill { r#ref, value } => {
                if value.len() > 65536 { return Err("Control value exceeds 65536 bytes".into()); }
                r#ref
            }
            Self::Click { r#ref } => r#ref,
        };
        if reference.is_empty() || reference.len() > 1024 {
            return Err("Invalid control ref".into());
        }
        Ok(())
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaneUiRequest {
    pub request_id: String,
    pub pane_id: String,
    pub mount_id: String,
    pub action: PaneUiAction,
}

struct Pending {
    pane_id: String,
    mount_id: String,
    sender: oneshot::Sender<Result<Value, String>>,
}

#[derive(Default)]
struct Inner {
    mounted: HashMap<String, String>,
    pending: HashMap<String, Pending>,
}

#[derive(Default)]
pub struct DatabasePaneUiBridge { inner: Mutex<Inner> }

// Cancellation and timeout release the reservation, including dropped callers.
struct Reservation<'a> { bridge: &'a DatabasePaneUiBridge, id: String }
impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if let Ok(mut inner) = self.bridge.inner.lock() { inner.pending.remove(&self.id); }
    }
}

impl DatabasePaneUiBridge {
    pub fn mount(&self, pane_id: &str, mount_id: &str, mounted: bool) -> Result<(), String> {
        if pane_id.is_empty() || mount_id.is_empty() || mount_id.len() > 128 {
            return Err("Invalid database pane mount".into());
        }
        let mut inner = self.inner.lock().map_err(|_| "Database UI bridge lock poisoned")?;
        if mounted {
            inner.mounted.insert(pane_id.into(), mount_id.into());
            inner.pending.retain(|_, pending| pending.pane_id != pane_id);
        } else if inner.mounted.get(pane_id).map(String::as_str) == Some(mount_id) {
            inner.mounted.remove(pane_id);
            inner.pending.retain(|_, pending| pending.pane_id != pane_id);
        }
        Ok(())
    }

    pub async fn request(
        &self,
        pane_id: &str,
        action: PaneUiAction,
        emit: impl FnOnce(&PaneUiRequest) -> Result<(), String>,
    ) -> Result<Value, String> {
        self.request_with_timeout(pane_id, action, Duration::from_secs(10), emit).await
    }

    async fn request_with_timeout(
        &self, pane_id: &str, action: PaneUiAction, timeout: Duration,
        emit: impl FnOnce(&PaneUiRequest) -> Result<(), String>,
    ) -> Result<Value, String> {
        action.validate()?;
        let (sender, receiver) = oneshot::channel();
        let request = {
            let mut inner = self.inner.lock().map_err(|_| "Database UI bridge lock poisoned")?;
            let mount_id = inner.mounted.get(pane_id).cloned()
                .ok_or("Database pane UI is not mounted; open its Session first")?;
            if inner.pending.len() >= 64 || inner.pending.values().any(|p| p.pane_id == pane_id) {
                return Err("Database pane UI already has a pending request".into());
            }
            let request_id = Uuid::new_v4().to_string();
            inner.pending.insert(request_id.clone(), Pending { pane_id: pane_id.into(), mount_id: mount_id.clone(), sender });
            PaneUiRequest { request_id, pane_id: pane_id.into(), mount_id, action }
        };
        let _reservation = Reservation { bridge: self, id: request.request_id.clone() };
        emit(&request)?;
        tokio::time::timeout(timeout, receiver).await
            .map_err(|_| "Database pane UI did not respond; inspect before retrying a click".to_string())?
            .map_err(|_| "Database pane UI was unmounted".to_string())?
    }

    pub fn respond(&self, request_id: &str, pane_id: &str, mount_id: &str, result: Result<Value, String>) -> Result<(), String> {
        if serde_json::to_vec(&result).map_err(|_| "Invalid UI response")?.len() > 512 * 1024 {
            return Err("Database UI response exceeds 512 KiB".into());
        }
        let mut inner = self.inner.lock().map_err(|_| "Database UI bridge lock poisoned")?;
        let pending = inner.pending.get(request_id).ok_or("Database UI request expired")?;
        if pending.pane_id != pane_id || pending.mount_id != mount_id {
            return Err("Database UI response has the wrong pane or mount".into());
        }
        let pending = inner.pending.remove(request_id).expect("validated pending request");
        pending.sender.send(result).map_err(|_| "Database UI caller disconnected".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn reply_is_bound_to_mounted_pane_and_request() {
        let bridge = DatabasePaneUiBridge::default();
        bridge.mount("pane", "mount", true).unwrap();
        let result = bridge.request("pane", PaneUiAction::Snapshot, |r| {
            assert!(bridge.respond(&r.request_id, "other", "mount", Ok(json!({}))).is_err());
            bridge.respond(&r.request_id, "pane", "mount", Ok(json!({"ready": true})))
        }).await.unwrap();
        assert_eq!(result["ready"], true);
        assert!(bridge.inner.lock().unwrap().pending.is_empty());
    }

    #[tokio::test]
    async fn timeout_emit_failure_and_unmount_release_requests() {
        let bridge = DatabasePaneUiBridge::default();
        assert!(bridge.request("pane", PaneUiAction::Snapshot, |_| Ok(())).await.is_err());
        bridge.mount("pane", "mount", true).unwrap();
        assert!(bridge.request_with_timeout("pane", PaneUiAction::Snapshot, Duration::from_millis(1), |_| Ok(())).await.is_err());
        assert!(bridge.request("pane", PaneUiAction::Snapshot, |_| Err("emit failed".into())).await.is_err());
        assert!(bridge.request("pane", PaneUiAction::Snapshot, |_| bridge.mount("pane", "mount", false)).await.is_err());
        assert!(bridge.inner.lock().unwrap().pending.is_empty());
    }

    #[tokio::test]
    async fn cancellation_cleans_up_and_old_unmount_cannot_remove_new_mount() {
        let bridge = DatabasePaneUiBridge::default();
        bridge.mount("pane", "new", true).unwrap();
        bridge.mount("pane", "old", false).unwrap();
        let mut request = Box::pin(bridge.request("pane", PaneUiAction::Snapshot, |_| Ok(())));
        tokio::select! { _ = &mut request => panic!("request should wait"), _ = tokio::task::yield_now() => {} }
        assert_eq!(bridge.inner.lock().unwrap().pending.len(), 1);
        drop(request);
        assert!(bridge.inner.lock().unwrap().pending.is_empty());
        assert_eq!(bridge.inner.lock().unwrap().mounted["pane"], "new");
    }
}
