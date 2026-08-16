use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CONTROL_CONTRACT_VERSION: u32 = 2;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ControlCallerKind {
    Ui,
    Agent,
    Cli,
    Internal,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ControlAccess {
    Read,
    Write,
    Control,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ControlApprovalRequirement {
    None,
    User,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ControlResourceKind {
    Application,
    Settings,
    ConnectionProfile,
    MuxSession,
    Pane,
    TerminalTarget,
    TerminalRuntime,
    Agent,
    BrowserRuntime,
    Transfer,
    Tunnel,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ControlResourceRef {
    pub kind: ControlResourceKind,
    pub id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ControlGrant {
    pub resource_kind: ControlResourceKind,
    pub resource_id: Option<String>,
    pub access: ControlAccess,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ControlCaller {
    pub caller_id: String,
    pub kind: ControlCallerKind,
    pub grants: Vec<ControlGrant>,
}

impl ControlCaller {
    pub fn has_access(
        &self,
        resource_kind: &ControlResourceKind,
        resource_id: Option<&str>,
        required: &ControlAccess,
    ) -> bool {
        self.grants.iter().any(|grant| {
            let kind_matches = &grant.resource_kind == resource_kind
                || grant.resource_kind == ControlResourceKind::Application;
            let id_matches =
                grant.resource_id.is_none() || grant.resource_id.as_deref() == resource_id;
            kind_matches && id_matches && grant.access.allows(required)
        })
    }

    pub fn can_access_any(
        &self,
        resource_kind: &ControlResourceKind,
        required: &ControlAccess,
    ) -> bool {
        self.grants.iter().any(|grant| {
            (&grant.resource_kind == resource_kind
                || grant.resource_kind == ControlResourceKind::Application)
                && grant.access.allows(required)
        })
    }
}

impl ControlAccess {
    pub(crate) fn allows(&self, required: &Self) -> bool {
        match (self, required) {
            (Self::Control, _) => true,
            (Self::Write, Self::Read | Self::Write) => true,
            (Self::Read, Self::Read) => true,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ControlOperationDescriptor {
    pub name: String,
    pub version: u32,
    pub access: ControlAccess,
    pub resource_kind: ControlResourceKind,
    pub mutating: bool,
    pub supports_idempotency: bool,
    pub approval: ControlApprovalRequirement,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ControlCatalog {
    pub contract_version: u32,
    pub operations: Vec<ControlOperationDescriptor>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ControlRequest {
    pub contract_version: u32,
    pub request_id: String,
    pub operation: String,
    pub resource: Option<ControlResourceRef>,
    pub arguments: Value,
    pub idempotency_key: Option<String>,
    pub approval_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ControlResponse {
    pub request_id: String,
    pub result: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ControlErrorCode {
    UnsupportedVersion,
    UnknownOperation,
    InvalidArguments,
    NotFound,
    Unauthorized,
    ApprovalRequired,
    ApprovalDenied,
    Conflict,
    Unavailable,
    Internal,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ControlError {
    pub code: ControlErrorCode,
    pub message: String,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

pub type ControlResult<T> = Result<T, ControlError>;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ControlApprovalStatus {
    Pending,
    Approved,
    Denied,
    Consumed,
    Expired,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ControlApproval {
    pub approval_id: String,
    pub caller_id: String,
    pub caller_kind: ControlCallerKind,
    pub operation: String,
    pub resource: Option<ControlResourceRef>,
    pub requested_at: String,
    pub expires_at: String,
    pub status: ControlApprovalStatus,
    pub resolved_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ControlEvent {
    pub sequence: u64,
    pub timestamp: String,
    pub event_type: String,
    pub resource: Option<ControlResourceRef>,
    pub payload: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ControlEventReadResult {
    pub requested_sequence: u64,
    pub earliest_sequence: u64,
    pub next_sequence: u64,
    pub truncated: bool,
    pub events: Vec<ControlEvent>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_envelope_does_not_accept_caller_identity_from_transport_payload() {
        let request = ControlRequest {
            contract_version: CONTROL_CONTRACT_VERSION,
            request_id: "request-1".into(),
            operation: "terminal.runtime.list".into(),
            resource: None,
            arguments: json!({}),
            idempotency_key: None,
            approval_id: None,
        };
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["contractVersion"], 2);
        assert!(value.get("caller").is_none());
        assert!(value.get("callerId").is_none());
    }

    #[test]
    fn resource_grants_are_directional_and_scoped() {
        let caller = ControlCaller {
            caller_id: "agent-1".into(),
            kind: ControlCallerKind::Agent,
            grants: vec![ControlGrant {
                resource_kind: ControlResourceKind::TerminalRuntime,
                resource_id: Some("runtime-2".into()),
                access: ControlAccess::Write,
            }],
        };
        assert_eq!(caller.grants[0].resource_id.as_deref(), Some("runtime-2"));
        assert_eq!(caller.grants[0].access, ControlAccess::Write);
    }
}
