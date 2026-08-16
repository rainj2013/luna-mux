use std::{
    collections::HashMap,
    sync::Mutex,
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
        self.approvals
            .lock()
            .map_err(|_| internal_error("审批策略锁已损坏"))?
            .insert(
                approval.approval_id.clone(),
                ApprovalRecord {
                    approval: approval.clone(),
                    arguments: request.arguments.clone(),
                    expires_at,
                },
            );
        Ok(approval)
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
        Ok(record.approval.clone())
    }
}

fn expire(record: &mut ApprovalRecord) {
    if record.approval.status == ControlApprovalStatus::Pending
        || record.approval.status == ControlApprovalStatus::Approved
    {
        if SystemTime::now() > record.expires_at {
            record.approval.status = ControlApprovalStatus::Expired;
            record.approval.resolved_at = Some(timestamp(SystemTime::now()));
        }
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
}
