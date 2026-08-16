use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use uuid::Uuid;

use crate::{
    control_contract::{
        ControlCaller, ControlCatalog, ControlError, ControlErrorCode, ControlEventReadResult,
        ControlRequest, ControlResponse, ControlResult,
    },
    control_service::LunaControlService,
};

/// Authentication boundary shared by future loopback HTTP, MCP, and CLI
/// transports. Transports provide only an opaque token and request envelope;
/// caller identity and grants always come from this registry.
pub struct AuthenticatedControlAdapter {
    service: Arc<dyn LunaControlService>,
    callers: RwLock<HashMap<String, ControlCaller>>,
}

impl std::fmt::Debug for AuthenticatedControlAdapter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthenticatedControlAdapter")
            .field(
                "registered_callers",
                &self
                    .callers
                    .read()
                    .map(|callers| callers.len())
                    .unwrap_or(0),
            )
            .finish_non_exhaustive()
    }
}

#[allow(dead_code)]
impl AuthenticatedControlAdapter {
    pub fn new(service: Arc<dyn LunaControlService>) -> Arc<Self> {
        Arc::new(Self {
            service,
            callers: RwLock::new(HashMap::new()),
        })
    }

    pub fn issue_token(&self, caller: ControlCaller) -> ControlResult<String> {
        if caller.caller_id.trim().is_empty() {
            return Err(invalid_token("callerId 不能为空"));
        }
        let token = format!("lmx_{}", Uuid::new_v4().simple());
        self.callers
            .write()
            .map_err(|_| internal_error("控制适配器授权锁已损坏"))?
            .insert(token.clone(), caller);
        Ok(token)
    }

    pub fn revoke_token(&self, token: &str) -> ControlResult<bool> {
        Ok(self
            .callers
            .write()
            .map_err(|_| internal_error("控制适配器授权锁已损坏"))?
            .remove(token)
            .is_some())
    }

    pub fn update_caller(&self, token: &str, caller: ControlCaller) -> ControlResult<bool> {
        let mut callers = self
            .callers
            .write()
            .map_err(|_| internal_error("控制适配器授权锁已损坏"))?;
        let Some(current) = callers.get_mut(token) else {
            return Ok(false);
        };
        *current = caller;
        Ok(true)
    }

    pub async fn invoke(
        &self,
        bearer_token: &str,
        request: ControlRequest,
    ) -> ControlResult<ControlResponse> {
        let caller = self.authenticate(bearer_token)?;
        self.service.invoke(&caller, request).await
    }

    pub fn catalog(&self, bearer_token: &str) -> ControlResult<ControlCatalog> {
        let caller = self.authenticate(bearer_token)?;
        Ok(self.service.catalog(&caller))
    }

    pub fn read_events(
        &self,
        bearer_token: &str,
        from_sequence: u64,
        limit: usize,
    ) -> ControlResult<ControlEventReadResult> {
        let caller = self.authenticate(bearer_token)?;
        self.service.read_events(&caller, from_sequence, limit)
    }

    fn authenticate(&self, bearer_token: &str) -> ControlResult<ControlCaller> {
        if bearer_token.trim().is_empty() {
            return Err(invalid_token("缺少控制适配器令牌"));
        }
        self.callers
            .read()
            .map_err(|_| internal_error("控制适配器授权锁已损坏"))?
            .get(bearer_token)
            .cloned()
            .ok_or_else(|| invalid_token("控制适配器令牌无效或已撤销"))
    }
}

fn invalid_token(message: &str) -> ControlError {
    ControlError {
        code: ControlErrorCode::Unauthorized,
        message: message.into(),
        retryable: false,
        details: None,
    }
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
    use async_trait::async_trait;
    use serde_json::json;

    use super::*;
    use crate::control_contract::{
        CONTROL_CONTRACT_VERSION, ControlAccess, ControlCallerKind, ControlCatalog, ControlGrant,
        ControlResourceKind,
    };
    use crate::control_service::LunaControlService;

    struct CallerEchoService;

    #[async_trait]
    impl LunaControlService for CallerEchoService {
        fn catalog(&self, _caller: &ControlCaller) -> ControlCatalog {
            ControlCatalog {
                contract_version: CONTROL_CONTRACT_VERSION,
                operations: vec![crate::control_contract::ControlOperationDescriptor {
                    name: "echo".into(),
                    version: 1,
                    access: ControlAccess::Read,
                    resource_kind: ControlResourceKind::MuxSession,
                    mutating: false,
                    supports_idempotency: true,
                    approval: crate::control_contract::ControlApprovalRequirement::None,
                }],
            }
        }

        async fn invoke(
            &self,
            caller: &ControlCaller,
            request: ControlRequest,
        ) -> ControlResult<ControlResponse> {
            Ok(ControlResponse {
                request_id: request.request_id,
                result: json!({ "callerId": caller.caller_id, "kind": caller.kind }),
            })
        }

        fn read_events(
            &self,
            caller: &ControlCaller,
            from_sequence: u64,
            _limit: usize,
        ) -> ControlResult<crate::control_contract::ControlEventReadResult> {
            Ok(crate::control_contract::ControlEventReadResult {
                requested_sequence: from_sequence,
                earliest_sequence: from_sequence,
                next_sequence: from_sequence + 1,
                truncated: false,
                events: vec![crate::control_contract::ControlEvent {
                    sequence: from_sequence,
                    timestamp: "2026-08-12T00:00:00Z".into(),
                    event_type: "echo".into(),
                    resource: None,
                    payload: json!({ "callerId": caller.caller_id }),
                }],
            })
        }
    }

    fn request() -> ControlRequest {
        ControlRequest {
            contract_version: CONTROL_CONTRACT_VERSION,
            request_id: "request-1".into(),
            operation: "test".into(),
            resource: None,
            arguments: json!({ "callerId": "forged" }),
            idempotency_key: None,
            approval_id: None,
        }
    }

    #[tokio::test]
    async fn injects_registered_caller_and_ignores_forged_arguments() {
        let adapter = AuthenticatedControlAdapter::new(Arc::new(CallerEchoService));
        let token = adapter
            .issue_token(ControlCaller {
                caller_id: "agent-real".into(),
                kind: ControlCallerKind::Agent,
                grants: vec![ControlGrant {
                    resource_kind: ControlResourceKind::MuxSession,
                    resource_id: None,
                    access: ControlAccess::Read,
                }],
            })
            .unwrap();
        let response = adapter.invoke(&token, request()).await.unwrap();
        assert_eq!(response.result["callerId"], "agent-real");
        assert_eq!(response.result["kind"], "agent");
    }

    #[tokio::test]
    async fn rejects_missing_unknown_and_revoked_tokens() {
        let adapter = AuthenticatedControlAdapter::new(Arc::new(CallerEchoService));
        assert_eq!(
            adapter.invoke("", request()).await.unwrap_err().code,
            ControlErrorCode::Unauthorized
        );
        let token = adapter
            .issue_token(ControlCaller {
                caller_id: "agent".into(),
                kind: ControlCallerKind::Agent,
                grants: vec![],
            })
            .unwrap();
        assert!(adapter.revoke_token(&token).unwrap());
        assert_eq!(
            adapter.invoke(&token, request()).await.unwrap_err().code,
            ControlErrorCode::Unauthorized
        );
    }

    #[test]
    fn authenticates_catalog_and_event_cursor_reads() {
        let adapter = AuthenticatedControlAdapter::new(Arc::new(CallerEchoService));
        let token = adapter
            .issue_token(ControlCaller {
                caller_id: "agent-events".into(),
                kind: ControlCallerKind::Agent,
                grants: vec![],
            })
            .unwrap();
        assert_eq!(adapter.catalog(&token).unwrap().operations[0].name, "echo");
        let events = adapter.read_events(&token, 7, 10).unwrap();
        assert_eq!(events.events[0].payload["callerId"], "agent-events");
        assert_eq!(events.next_sequence, 8);
    }
}
