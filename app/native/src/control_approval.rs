use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::control_contract::{
    ControlApproval, ControlApprovalStatus, ControlCaller, ControlError, ControlErrorCode,
    ControlRequest, ControlResult,
};

const APPROVAL_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Clone)]
struct ApprovalRecord {
    approval: ControlApproval,
    arguments: serde_json::Value,
    expires_at: SystemTime,
    idempotency_key: Option<String>,
    decision: tokio::sync::watch::Sender<ControlApprovalStatus>,
    waiters: usize,
}

#[derive(Default)]
pub struct ControlApprovalPolicy {
    approvals: Mutex<HashMap<String, ApprovalRecord>>,
}

impl ControlApprovalPolicy {
    pub fn request(
        &self,
        caller: &ControlCaller,
        request: &ControlRequest,
    ) -> ControlResult<ControlApproval> {
        let now = SystemTime::now();
        let expires_at = now + APPROVAL_TTL;
        let approval = ControlApproval {
            approval_id: format!("approval_{}", Uuid::new_v4().simple()),
            caller_id: caller.caller_id.clone(),
            caller_kind: caller.kind.clone(),
            operation: request.operation.clone(),
            resource: request.resource.clone(),
            requested_at: timestamp(now),
            expires_at: timestamp(expires_at),
            status: ControlApprovalStatus::Pending,
            resolved_at: None,
        };
        let mut approvals = self
            .approvals
            .lock()
            .map_err(|_| internal_error("审批策略锁已损坏"))?;
        for record in approvals.values_mut() {
            expire(record);
        }
        approvals.retain(|_, record| record.waiters > 0 || record.expires_at > now);
        if let Some(existing) = matching_transfer(&approvals, caller, request)? {
            return Ok(existing);
        }
        if approvals.len() >= 1024 {
            return Err(ControlError {
                code: ControlErrorCode::Unavailable,
                message: "审批请求过多，请等待已有请求完成".into(),
                retryable: true,
                details: None,
            });
        }
        approvals.insert(
            approval.approval_id.clone(),
            ApprovalRecord {
                approval: approval.clone(),
                arguments: request.arguments.clone(),
                expires_at,
                idempotency_key: request.idempotency_key.clone(),
                decision: tokio::sync::watch::channel(ControlApprovalStatus::Pending).0,
                waiters: 0,
            },
        );
        Ok(approval)
    }

    pub fn matching_transfer(
        &self,
        caller: &ControlCaller,
        request: &ControlRequest,
    ) -> ControlResult<Option<ControlApproval>> {
        let mut approvals = self
            .approvals
            .lock()
            .map_err(|_| internal_error("审批策略锁已损坏"))?;
        for record in approvals.values_mut() {
            expire(record);
        }
        matching_transfer(&approvals, caller, request)
    }

    /// Only the trusted desktop IPC projects file paths; public events carry IDs only.
    pub fn pending_transfers(&self) -> ControlResult<Vec<(ControlApproval, serde_json::Value)>> {
        let mut approvals = self
            .approvals
            .lock()
            .map_err(|_| internal_error("审批策略锁已损坏"))?;
        let mut pending = Vec::new();
        for record in approvals.values_mut() {
            expire(record);
            if record.approval.caller_kind == crate::control_contract::ControlCallerKind::Agent
                && record.approval.operation == "transfer.enqueue"
                && record.approval.status == ControlApprovalStatus::Pending
            {
                pending.push((record.approval.clone(), record.arguments.clone()));
            }
        }
        pending.sort_by(|left, right| {
            left.0
                .requested_at
                .cmp(&right.0.requested_at)
                .then_with(|| left.0.approval_id.cmp(&right.0.approval_id))
        });
        Ok(pending)
    }

    pub async fn wait_for_transfer(
        self: &Arc<Self>,
        caller: &ControlCaller,
        request: &ControlRequest,
        approval_id: &str,
    ) -> ControlResult<()> {
        let (mut decision, remaining) = {
            let mut approvals = self
                .approvals
                .lock()
                .map_err(|_| internal_error("审批策略锁已损坏"))?;
            let record = approvals
                .get_mut(approval_id)
                .ok_or_else(|| denied("审批请求不存在", None))?;
            if request.operation != "transfer.enqueue"
                || record.approval.caller_id != caller.caller_id
                || record.approval.caller_kind != caller.kind
                || record.approval.operation != request.operation
                || record.approval.resource != request.resource
                || record.arguments != request.arguments
                || record.idempotency_key != request.idempotency_key
            {
                return Err(denied("审批凭据与控制请求不匹配", None));
            }
            expire(record);
            record.waiters += 1;
            (
                record.decision.subscribe(),
                record
                    .expires_at
                    .duration_since(SystemTime::now())
                    .unwrap_or_default(),
            )
        };
        let _waiter = ApprovalWaiter {
            policy: self.clone(),
            approval_id: approval_id.into(),
        };
        let wait = async {
            loop {
                let status = decision.borrow_and_update().clone();
                match status {
                    ControlApprovalStatus::Approved | ControlApprovalStatus::Consumed => {
                        return Ok(());
                    }
                    ControlApprovalStatus::Pending => {}
                    _ => return Err(denied("SFTP 传输审批已拒绝、取消或过期", Some(status))),
                }
                decision
                    .changed()
                    .await
                    .map_err(|_| internal_error("审批通知已关闭"))?;
            }
        };
        match tokio::time::timeout(remaining, wait).await {
            Ok(result) => result,
            Err(_) => {
                let mut approvals = self
                    .approvals
                    .lock()
                    .map_err(|_| internal_error("审批策略锁已损坏"))?;
                if let Some(record) = approvals.get_mut(approval_id) {
                    if matches!(
                        record.approval.status,
                        ControlApprovalStatus::Pending | ControlApprovalStatus::Approved
                    ) {
                        record.approval.status = ControlApprovalStatus::Expired;
                        record.approval.resolved_at = Some(timestamp(SystemTime::now()));
                        record.decision.send_replace(ControlApprovalStatus::Expired);
                    }
                }
                Err(denied(
                    "SFTP 传输审批已过期",
                    Some(ControlApprovalStatus::Expired),
                ))
            }
        }
    }

    pub fn resolve(&self, approval_id: &str, approved: bool) -> ControlResult<ControlApproval> {
        let mut approvals = self
            .approvals
            .lock()
            .map_err(|_| internal_error("审批策略锁已损坏"))?;
        let record = approvals.get_mut(approval_id).ok_or_else(|| ControlError {
            code: ControlErrorCode::NotFound,
            message: "审批请求不存在".into(),
            retryable: false,
            details: None,
        })?;
        expire(record);
        if record.approval.status != ControlApprovalStatus::Pending {
            return Err(ControlError {
                code: ControlErrorCode::Conflict,
                message: "审批请求已处理或已过期".into(),
                retryable: false,
                details: Some(serde_json::json!({ "status": record.approval.status })),
            });
        }
        record.approval.status = if approved {
            ControlApprovalStatus::Approved
        } else {
            ControlApprovalStatus::Denied
        };
        record.approval.resolved_at = Some(timestamp(SystemTime::now()));
        record.decision.send_replace(record.approval.status.clone());
        Ok(record.approval.clone())
    }

    pub fn consume(
        &self,
        caller: &ControlCaller,
        request: &ControlRequest,
        approval_id: &str,
    ) -> ControlResult<ControlApproval> {
        let mut approvals = self
            .approvals
            .lock()
            .map_err(|_| internal_error("审批策略锁已损坏"))?;
        let record = approvals.get_mut(approval_id).ok_or_else(|| ControlError {
            code: ControlErrorCode::ApprovalDenied,
            message: "审批凭据不存在".into(),
            retryable: false,
            details: None,
        })?;
        expire(record);
        if record.approval.caller_id != caller.caller_id
            || record.approval.caller_kind != caller.kind
            || record.approval.operation != request.operation
            || record.approval.resource != request.resource
            || record.arguments != request.arguments
        {
            return Err(ControlError {
                code: ControlErrorCode::ApprovalDenied,
                message: "审批凭据与控制请求不匹配".into(),
                retryable: false,
                details: None,
            });
        }
        if record.approval.status != ControlApprovalStatus::Approved {
            return Err(ControlError {
                code: ControlErrorCode::ApprovalDenied,
                message: "审批请求未获批准或已失效".into(),
                retryable: false,
                details: Some(serde_json::json!({ "status": record.approval.status })),
            });
        }
        record.approval.status = ControlApprovalStatus::Consumed;
        record
            .decision
            .send_replace(ControlApprovalStatus::Consumed);
        Ok(record.approval.clone())
    }
}

fn matching_transfer(
    approvals: &HashMap<String, ApprovalRecord>,
    caller: &ControlCaller,
    request: &ControlRequest,
) -> ControlResult<Option<ControlApproval>> {
    if caller.kind != crate::control_contract::ControlCallerKind::Agent
        || request.operation != "transfer.enqueue"
    {
        return Ok(None);
    }
    let Some(key) = request
        .idempotency_key
        .as_deref()
        .filter(|key| !key.is_empty())
    else {
        return Ok(None);
    };
    for record in approvals.values() {
        if record.approval.caller_id == caller.caller_id
            && record.approval.caller_kind == caller.kind
            && record.approval.operation == request.operation
            && record.idempotency_key.as_deref() == Some(key)
            && matches!(
                record.approval.status,
                ControlApprovalStatus::Pending | ControlApprovalStatus::Approved
            )
        {
            if record.approval.resource != request.resource || record.arguments != request.arguments
            {
                return Err(ControlError {
                    code: ControlErrorCode::Conflict,
                    message: "幂等键已用于不同的待审批传输".into(),
                    retryable: false,
                    details: None,
                });
            }
            return Ok(Some(record.approval.clone()));
        }
    }
    Ok(None)
}

fn expire(record: &mut ApprovalRecord) {
    if record.approval.status == ControlApprovalStatus::Pending
        || record.approval.status == ControlApprovalStatus::Approved
    {
        if SystemTime::now() > record.expires_at {
            record.approval.status = ControlApprovalStatus::Expired;
            record.approval.resolved_at = Some(timestamp(SystemTime::now()));
            record.decision.send_replace(ControlApprovalStatus::Expired);
        }
    }
}

// Dropping a cancelled MCP call withdraws consent UI, while shared retries retain it.
struct ApprovalWaiter {
    policy: Arc<ControlApprovalPolicy>,
    approval_id: String,
}

impl Drop for ApprovalWaiter {
    fn drop(&mut self) {
        if let Ok(mut approvals) = self.policy.approvals.lock() {
            if let Some(record) = approvals.get_mut(&self.approval_id) {
                record.waiters = record.waiters.saturating_sub(1);
                if record.waiters == 0 && record.approval.status == ControlApprovalStatus::Pending {
                    record.approval.status = ControlApprovalStatus::Denied;
                    record.approval.resolved_at = Some(timestamp(SystemTime::now()));
                    record.decision.send_replace(ControlApprovalStatus::Denied);
                }
            }
        }
    }
}

fn denied(message: &str, status: Option<ControlApprovalStatus>) -> ControlError {
    ControlError {
        code: ControlErrorCode::ApprovalDenied,
        message: message.into(),
        retryable: false,
        details: status.map(|status| serde_json::json!({ "status": status })),
    }
}

fn timestamp(value: SystemTime) -> String {
    DateTime::<Utc>::from(value).to_rfc3339()
}

fn internal_error(message: &str) -> ControlError {
    ControlError {
        code: ControlErrorCode::Internal,
        message: message.into(),
        retryable: false,
        details: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control_contract::{
        CONTROL_CONTRACT_VERSION, ControlCallerKind, ControlRequest, ControlResourceKind,
        ControlResourceRef,
    };

    fn caller(id: &str) -> ControlCaller {
        ControlCaller {
            caller_id: id.into(),
            kind: ControlCallerKind::Agent,
            grants: vec![],
        }
    }

    fn request(approval_id: Option<String>) -> ControlRequest {
        ControlRequest {
            contract_version: CONTROL_CONTRACT_VERSION,
            request_id: "request-1".into(),
            operation: "terminal.runtime.close".into(),
            resource: Some(ControlResourceRef {
                kind: ControlResourceKind::TerminalRuntime,
                id: "runtime-1".into(),
            }),
            arguments: serde_json::json!({}),
            idempotency_key: None,
            approval_id,
        }
    }

    #[test]
    fn approval_is_bound_to_caller_operation_resource_and_arguments() {
        let policy = ControlApprovalPolicy::default();
        let agent = caller("agent-1");
        let approval = policy.request(&agent, &request(None)).unwrap();
        policy.resolve(&approval.approval_id, true).unwrap();
        let mut approved_request = request(Some(approval.approval_id.clone()));
        policy
            .consume(&agent, &approved_request, &approval.approval_id)
            .unwrap();
        let error = policy
            .consume(&agent, &approved_request, &approval.approval_id)
            .unwrap_err();
        assert_eq!(error.code, ControlErrorCode::ApprovalDenied);

        let other = policy.request(&agent, &request(None)).unwrap();
        policy.resolve(&other.approval_id, true).unwrap();
        approved_request.arguments = serde_json::json!({ "changed": true });
        assert_eq!(
            policy
                .consume(&agent, &approved_request, &other.approval_id)
                .unwrap_err()
                .code,
            ControlErrorCode::ApprovalDenied
        );
    }

    #[test]
    fn denied_approval_cannot_be_consumed() {
        let policy = ControlApprovalPolicy::default();
        let agent = caller("agent-1");
        let approval = policy.request(&agent, &request(None)).unwrap();
        policy.resolve(&approval.approval_id, false).unwrap();
        assert_eq!(
            policy
                .consume(&agent, &request(None), &approval.approval_id)
                .unwrap_err()
                .code,
            ControlErrorCode::ApprovalDenied
        );
    }
    fn transfer_request(key: &str) -> ControlRequest {
        let mut request = request(None);
        request.operation = "transfer.enqueue".into();
        request.arguments = serde_json::json!({ "direction": "upload", "sourcePaths": ["file.txt"], "destinationDirectory": "/tmp" });
        request.idempotency_key = Some(key.into());
        request
    }

    async fn await_waiters(policy: &ControlApprovalPolicy, id: &str, count: usize) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if policy.approvals.lock().unwrap()[id].waiters == count {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn transfer_waiters_share_one_ticket_and_last_cancellation_withdraws_it() {
        let policy = Arc::new(ControlApprovalPolicy::default());
        let agent = caller("agent-1");
        let request = transfer_request("stable");
        let ticket = policy.request(&agent, &request).unwrap();
        assert_eq!(
            policy.request(&agent, &request).unwrap().approval_id,
            ticket.approval_id
        );
        let mut changed = request.clone();
        changed.arguments["destinationDirectory"] = serde_json::json!("/different");
        assert_eq!(
            policy.request(&agent, &changed).unwrap_err().code,
            ControlErrorCode::Conflict
        );
        let start = || {
            let policy = policy.clone();
            let agent = agent.clone();
            let request = request.clone();
            let id = ticket.approval_id.clone();
            tokio::spawn(async move { policy.wait_for_transfer(&agent, &request, &id).await })
        };
        let first = start();
        let second = start();
        await_waiters(&policy, &ticket.approval_id, 2).await;
        first.abort();
        assert!(first.await.unwrap_err().is_cancelled());
        assert_eq!(policy.pending_transfers().unwrap().len(), 1);
        second.abort();
        assert!(second.await.unwrap_err().is_cancelled());
        assert!(policy.pending_transfers().unwrap().is_empty());
        assert_eq!(
            policy.resolve(&ticket.approval_id, true).unwrap_err().code,
            ControlErrorCode::Conflict
        );
    }

    #[tokio::test]
    async fn transfer_wait_is_bound_and_wakes_on_approval_denial_and_expiry() {
        let policy = Arc::new(ControlApprovalPolicy::default());
        let agent = caller("agent-1");
        let request = transfer_request("bound");
        for approve in [true, false] {
            let ticket = policy.request(&agent, &request).unwrap();
            assert_eq!(
                policy
                    .wait_for_transfer(&caller("other"), &request, &ticket.approval_id)
                    .await
                    .unwrap_err()
                    .code,
                ControlErrorCode::ApprovalDenied
            );
            let waiting = {
                let policy = policy.clone();
                let agent = agent.clone();
                let request = request.clone();
                let id = ticket.approval_id.clone();
                tokio::spawn(async move { policy.wait_for_transfer(&agent, &request, &id).await })
            };
            await_waiters(&policy, &ticket.approval_id, 1).await;
            policy.resolve(&ticket.approval_id, approve).unwrap();
            let result = waiting.await.unwrap();
            assert_eq!(result.is_ok(), approve);
            if approve {
                policy
                    .consume(&agent, &request, &ticket.approval_id)
                    .unwrap();
            }
            assert!(policy.pending_transfers().unwrap().is_empty());
        }
        let ticket = policy.request(&agent, &request).unwrap();
        policy
            .approvals
            .lock()
            .unwrap()
            .get_mut(&ticket.approval_id)
            .unwrap()
            .expires_at = SystemTime::now() + Duration::from_millis(20);
        let error = policy
            .wait_for_transfer(&agent, &request, &ticket.approval_id)
            .await
            .unwrap_err();
        assert_eq!(error.details.unwrap()["status"], "expired");
        assert!(policy.pending_transfers().unwrap().is_empty());
        assert_eq!(
            policy.resolve(&ticket.approval_id, true).unwrap_err().code,
            ControlErrorCode::Conflict
        );
    }
}
