use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http,
    middleware::{self, Next},
    response::Response,
};
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorCode,
        ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
        ToolAnnotations,
    },
    service::RequestContext,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    browser_runtime::{BrowserRuntime, BrowserRuntimeManager},
    control_adapter::AuthenticatedControlAdapter,
    control_contract::{
        CONTROL_CONTRACT_VERSION, ControlAccess, ControlApprovalRequirement, ControlCaller,
        ControlCallerKind, ControlError, ControlGrant, ControlOperationDescriptor, ControlRequest,
        ControlResourceKind, ControlResourceRef,
    },
    terminal_runtime_contract::TerminalManagedAgentContext,
};

pub trait BrowserRuntimeCatalog: Send + Sync {
    fn list_browser_runtimes(&self) -> Result<Vec<BrowserRuntime>, String>;
}

impl BrowserRuntimeCatalog for BrowserRuntimeManager {
    fn list_browser_runtimes(&self) -> Result<Vec<BrowserRuntime>, String> {
        self.list()
    }
}

pub const MCP_AUTHORIZATION_ENV: &str = "LUNA_MUX_MCP_AUTHORIZATION";

pub struct LunaMcpService {
    adapter: Arc<AuthenticatedControlAdapter>,
    database: Arc<crate::database::Database>,
    agent_hooks: Arc<crate::agent_hooks::AgentHookService>,
    browser_runtimes: Option<Arc<dyn BrowserRuntimeCatalog>>,
    endpoint: RwLock<String>,
    runtime_tokens: RwLock<HashMap<String, Vec<String>>>,
    cancellation: CancellationToken,
}

impl LunaMcpService {
    pub fn new(
        adapter: Arc<AuthenticatedControlAdapter>,
        database: Arc<crate::database::Database>,
        agent_hooks: Arc<crate::agent_hooks::AgentHookService>,
        browser_runtimes: Option<Arc<dyn BrowserRuntimeCatalog>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            adapter,
            database,
            agent_hooks,
            browser_runtimes,
            endpoint: RwLock::new(String::new()),
            runtime_tokens: RwLock::new(HashMap::new()),
            cancellation: CancellationToken::new(),
        })
    }

    pub fn start(self: &Arc<Self>) -> Result<(), String> {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .map_err(|error| error.to_string())?;
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        *self.endpoint.write().map_err(|_| "MCP 地址锁已损坏")? =
            format!("http://127.0.0.1:{}/mcp", address.port());

        let adapter = self.adapter.clone();
        let cancellation = self.cancellation.clone();
        tauri::async_runtime::spawn(async move {
            let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
                return;
            };
            let handler_adapter = adapter.clone();
            let service = StreamableHttpService::new(
                move || Ok(LunaMcpHandler::new(handler_adapter.clone())),
                Arc::new(LocalSessionManager::default()),
                StreamableHttpServerConfig::default()
                    .with_json_response(true)
                    .with_cancellation_token(cancellation.clone()),
            );
            let app =
                Router::new()
                    .nest_service("/mcp", service)
                    .layer(middleware::from_fn_with_state(
                        adapter.clone(),
                        authenticate_http,
                    ));
            let _ = axum::serve(listener, app).await;
        });
        Ok(())
    }

    pub fn endpoint(&self) -> Result<String, String> {
        let endpoint = self
            .endpoint
            .read()
            .map_err(|_| "MCP 地址锁已损坏")?
            .clone();
        if endpoint.is_empty() {
            Err("MCP 服务尚未启动".into())
        } else {
            Ok(endpoint)
        }
    }

    pub fn issue_runtime_token(
        &self,
        context: &TerminalManagedAgentContext,
    ) -> Result<String, String> {
        let caller = self.runtime_caller(context)?;
        let token = self
            .adapter
            .issue_token(caller)
            .map_err(|error| error.message)?;
        let mut tokens = self
            .runtime_tokens
            .write()
            .map_err(|_| "MCP Runtime 授权锁已损坏".to_string())?;
        tokens
            .entry(context.runtime_id.clone())
            .or_default()
            .push(token.clone());
        drop(tokens);
        if let Err(error) = self.refresh_target_resource("pane", &context.pane_id) {
            self.revoke_token(&token);
            return Err(error);
        }
        Ok(token)
    }

    pub fn issue_runtime_context_token(
        &self,
        context: &crate::terminal_runtime_contract::TerminalRuntimeContext,
    ) -> Result<String, String> {
        let managed = TerminalManagedAgentContext {
            mux_session_id: context.mux_session_id.clone(),
            pane_id: context.pane_id.clone(),
            runtime_id: context.runtime_id.clone(),
            agent_id: format!("runtime:{}", context.runtime_id),
            launch_profile_id: "codex.auto".into(),
        };
        let caller = self.runtime_caller(&managed)?;
        let token = self
            .adapter
            .issue_token(caller)
            .map_err(|error| error.message)?;
        self.runtime_tokens
            .write()
            .map_err(|_| "MCP Runtime 授权锁已损坏".to_string())?
            .entry(context.runtime_id.clone())
            .or_default()
            .push(token.clone());
        Ok(token)
    }

    pub fn refresh_target_resource(
        &self,
        target_resource_kind: &str,
        target_resource_id: &str,
    ) -> Result<(), String> {
        let source_panes = self
            .database
            .list_agent_control_grants(None)?
            .into_iter()
            .filter(|grant| {
                grant.target_resource_kind == target_resource_kind
                    && grant.target_resource_id == target_resource_id
            })
            .map(|grant| grant.source_pane_id)
            .collect::<std::collections::HashSet<_>>();
        for source_pane_id in source_panes {
            self.refresh_source_pane(&source_pane_id)?;
        }
        Ok(())
    }

    pub fn refresh_source_pane(&self, source_pane_id: &str) -> Result<(), String> {
        let contexts = self
            .agent_hooks
            .snapshots()
            .into_iter()
            .filter(|agent| agent.context.pane_id == source_pane_id)
            .map(|agent| agent.context)
            .collect::<Vec<_>>();
        let runtimes = self
            .runtime_tokens
            .read()
            .map_err(|_| "MCP Runtime 授权锁已损坏".to_string())?;
        for context in contexts {
            let caller = self.runtime_caller(&context)?;
            if let Some(tokens) = runtimes.get(&context.runtime_id) {
                for token in tokens {
                    self.adapter
                        .update_caller(token, caller.clone())
                        .map_err(|error| error.message)?;
                }
            }
        }
        Ok(())
    }

    fn runtime_caller(
        &self,
        context: &TerminalManagedAgentContext,
    ) -> Result<ControlCaller, String> {
        let mut caller = base_runtime_caller(context);
        let snapshots = self.agent_hooks.snapshots();
        let browser_runtimes = self
            .browser_runtimes
            .as_ref()
            .map(|catalog| catalog.list_browser_runtimes())
            .transpose()?
            .unwrap_or_default();
        for grant in self
            .database
            .list_agent_control_grants(Some(&context.pane_id))?
        {
            let access = match grant.access.as_str() {
                "control" => ControlAccess::Control,
                "write" => ControlAccess::Write,
                _ => ControlAccess::Read,
            };
            if grant.target_resource_kind == "pane"
                && let Some(target_agent) = snapshots
                    .iter()
                    .find(|agent| agent.context.pane_id == grant.target_resource_id)
            {
                caller.grants.push(ControlGrant {
                    resource_kind: ControlResourceKind::Agent,
                    resource_id: Some(target_agent.context.agent_id.clone()),
                    access: access.clone(),
                });
                caller.grants.push(ControlGrant {
                    resource_kind: ControlResourceKind::TerminalRuntime,
                    resource_id: Some(target_agent.context.runtime_id.clone()),
                    access: access.clone(),
                });
            }
            if grant.target_resource_kind == "browser"
                && self
                    .database
                    .list_browser_resources(None)?
                    .iter()
                    .any(|resource| resource.id == grant.target_resource_id)
            {
                for runtime in browser_runtimes
                    .iter()
                    .filter(|runtime| runtime.browser_resource_id == grant.target_resource_id)
                {
                    caller.grants.push(ControlGrant {
                        resource_kind: ControlResourceKind::BrowserRuntime,
                        resource_id: Some(runtime.id.clone()),
                        access: access.clone(),
                    });
                }
            }
        }
        Ok(caller)
    }

    pub fn revoke_token(&self, token: &str) {
        let _ = self.adapter.revoke_token(token);
        if let Ok(mut runtimes) = self.runtime_tokens.write() {
            runtimes.retain(|_, tokens| {
                tokens.retain(|current| current != token);
                !tokens.is_empty()
            });
        }
    }

    pub fn revoke_runtime(&self, runtime_id: &str) {
        let tokens = self
            .runtime_tokens
            .write()
            .ok()
            .and_then(|mut runtimes| runtimes.remove(runtime_id))
            .unwrap_or_default();
        for token in tokens {
            let _ = self.adapter.revoke_token(&token);
        }
        if let Ok(grants) = self.database.list_agent_control_grants(None) {
            for source_pane_id in grants
                .into_iter()
                .map(|grant| grant.source_pane_id)
                .collect::<std::collections::HashSet<_>>()
            {
                let _ = self.refresh_source_pane(&source_pane_id);
            }
        }
    }

    pub fn shutdown(&self) {
        self.cancellation.cancel();
    }
}

async fn authenticate_http(
    State(adapter): State<Arc<AuthenticatedControlAdapter>>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, http::StatusCode> {
    let token = bearer_token(request.headers()).ok_or(http::StatusCode::UNAUTHORIZED)?;
    adapter
        .catalog(token)
        .map_err(|_| http::StatusCode::UNAUTHORIZED)?;
    Ok(next.run(request).await)
}

fn bearer_token(headers: &http::HeaderMap) -> Option<&str> {
    let value = headers.get(http::header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty()).then(|| token.trim())
}

fn base_runtime_caller(context: &TerminalManagedAgentContext) -> ControlCaller {
    ControlCaller {
        caller_id: format!("agent-runtime:{}", context.runtime_id),
        kind: ControlCallerKind::Agent,
        grants: vec![
            ControlGrant {
                resource_kind: ControlResourceKind::MuxSession,
                resource_id: Some(context.mux_session_id.clone()),
                access: ControlAccess::Read,
            },
            ControlGrant {
                resource_kind: ControlResourceKind::Pane,
                resource_id: Some(context.pane_id.clone()),
                access: ControlAccess::Read,
            },
            ControlGrant {
                resource_kind: ControlResourceKind::TerminalTarget,
                resource_id: None,
                access: ControlAccess::Read,
            },
            ControlGrant {
                resource_kind: ControlResourceKind::TerminalRuntime,
                resource_id: Some(context.runtime_id.clone()),
                access: ControlAccess::Control,
            },
            ControlGrant {
                resource_kind: ControlResourceKind::Agent,
                resource_id: Some(context.agent_id.clone()),
                access: ControlAccess::Read,
            },
        ],
    }
}

#[derive(Clone)]
struct LunaMcpHandler {
    adapter: Arc<AuthenticatedControlAdapter>,
}

impl LunaMcpHandler {
    fn new(adapter: Arc<AuthenticatedControlAdapter>) -> Self {
        Self { adapter }
    }

    fn token(context: &RequestContext<RoleServer>) -> Result<String, McpError> {
        let parts = context
            .extensions
            .get::<http::request::Parts>()
            .ok_or_else(|| McpError::invalid_request("缺少 MCP HTTP 请求上下文", None))?;
        bearer_token(&parts.headers)
            .map(str::to_string)
            .ok_or_else(|| McpError::invalid_request("缺少有效的 MCP Bearer 授权", None))
    }

    fn authorized_catalog(
        &self,
        context: &RequestContext<RoleServer>,
    ) -> Result<(String, crate::control_contract::ControlCatalog), McpError> {
        let token = Self::token(context)?;
        let catalog = self.adapter.catalog(&token).map_err(authentication_error)?;
        Ok((token, catalog))
    }
}

impl ServerHandler for LunaMcpHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "This server controls Luna Mux terminals, Sessions, panes, and other application resources. Browser automation is provided by the separate native agent-browser server; use that server for page navigation, snapshots, interaction, tabs, screenshots, console, and network inspection. Tools are filtered to the current Agent Runtime grants.",
        )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let (_, catalog) = self.authorized_catalog(&context)?;
        Ok(ListToolsResult::with_all_items(
            catalog
                .operations
                .into_iter()
                .filter(|operation| !operation.name.starts_with("browser."))
                .map(tool_for)
                .collect(),
        ))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let (token, catalog) = self.authorized_catalog(&context)?;
        let descriptor = catalog
            .operations
            .into_iter()
            .filter(|operation| !operation.name.starts_with("browser."))
            .find(|operation| operation.name == request.name)
            .ok_or_else(|| {
                McpError::new(
                    ErrorCode::METHOD_NOT_FOUND,
                    format!("MCP tool 不存在或当前 Runtime 无权调用：{}", request.name),
                    None,
                )
            })?;
        let input = serde_json::from_value::<McpControlInput>(Value::Object(
            request.arguments.unwrap_or_default(),
        ))
        .map_err(|error| McpError::invalid_params(error.to_string(), None))?;
        if !input.arguments.is_object() {
            return Err(McpError::invalid_params("arguments 必须是 JSON 对象", None));
        }
        let control_request = ControlRequest {
            contract_version: CONTROL_CONTRACT_VERSION,
            request_id: format!("mcp_{}", Uuid::new_v4().simple()),
            operation: descriptor.name,
            resource: input.resource_id.map(|id| ControlResourceRef {
                kind: descriptor.resource_kind,
                id,
            }),
            arguments: input.arguments,
            idempotency_key: input.idempotency_key,
            approval_id: input.approval_id,
        };
        let result = match self.adapter.invoke(&token, control_request).await {
            Ok(response) => CallToolResult::structured(
                serde_json::to_value(response)
                    .map_err(|error| McpError::internal_error(error.to_string(), None))?,
            ),
            Err(error) => control_tool_error(error),
        };
        Ok(result.into())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpControlInput {
    #[serde(default)]
    resource_id: Option<String>,
    #[serde(default = "empty_arguments")]
    arguments: Value,
    #[serde(default)]
    idempotency_key: Option<String>,
    #[serde(default)]
    approval_id: Option<String>,
}

fn empty_arguments() -> Value {
    json!({})
}

fn tool_for(descriptor: ControlOperationDescriptor) -> Tool {
    let approval = descriptor.approval == ControlApprovalRequirement::User;
    let description = format!(
        "Luna Mux control operation `{}`. Resource kind: {:?}. Access: {:?}.{} Put operation-specific fields inside `arguments`.",
        descriptor.name,
        descriptor.resource_kind,
        descriptor.access,
        if approval {
            " This operation requires approval in the trusted desktop UI; retry with the returned approvalId."
        } else {
            ""
        },
    );
    let arguments_schema = control_arguments_schema(&descriptor.name);
    let resource_required = !matches!(
        descriptor.name.as_str(),
        "agents.list"
            | "mux.sessions.list"
            | "mux.panes.list"
            | "terminal.targets.list"
            | "terminal.runtimes.list"
            | "browser.runtimes.list"
    );
    let schema = json!({
        "type": "object",
        "properties": {
            "resourceId": {
                "type": "string",
                "description": "Optional target resource ID. Required by operations on one Runtime, transfer, or tunnel."
            },
            "arguments": arguments_schema,
            "idempotencyKey": {
                "type": "string",
                "description": "Optional stable key for safely retrying the exact same operation, resource, and arguments."
            },
            "approvalId": {
                "type": "string",
                "description": "Approval ID returned by a prior approval-required result after the desktop UI approves it."
            }
        },
        "required": if resource_required { vec!["resourceId", "arguments"] } else { vec!["arguments"] },
        "additionalProperties": false
    });
    Tool::new(
        descriptor.name,
        description,
        schema.as_object().expect("tool schema object").clone(),
    )
    .with_annotations(
        ToolAnnotations::new()
            .read_only(!descriptor.mutating)
            .destructive(approval)
            .idempotent(descriptor.supports_idempotency)
            .open_world(false),
    )
}

fn control_arguments_schema(operation: &str) -> Value {
    match operation {
        "browser.navigate" => json!({
            "type": "object",
            "properties": { "url": { "type": "string", "description": "URL to navigate to." } },
            "required": ["url"],
            "additionalProperties": false
        }),
        "browser.click" => browser_selector_schema("CSS selector of the element to click."),
        "browser.type" => json!({
            "type": "object",
            "properties": {
                "selector": { "type": "string", "description": "CSS selector of the editable element." },
                "text": { "type": "string", "description": "Text to insert into the focused element." },
                "clear": { "type": "boolean", "default": false, "description": "Clear the current value before inserting text." }
            },
            "required": ["selector", "text"],
            "additionalProperties": false
        }),
        "browser.press" => json!({
            "type": "object",
            "properties": { "key": { "type": "string", "description": "Key or shortcut, for example Enter, Escape, Ctrl+A, or Meta+L." } },
            "required": ["key"],
            "additionalProperties": false
        }),
        "browser.scroll" => json!({
            "type": "object",
            "properties": {
                "deltaX": { "type": "number", "default": 0 },
                "deltaY": { "type": "number", "description": "Vertical scroll distance in CSS pixels." },
                "x": { "type": "number", "minimum": 0, "default": 0 },
                "y": { "type": "number", "minimum": 0, "default": 0 }
            },
            "required": ["deltaY"],
            "additionalProperties": false
        }),
        "browser.evaluate" => json!({
            "type": "object",
            "properties": { "expression": { "type": "string", "description": "JavaScript expression evaluated in the current page." } },
            "required": ["expression"],
            "additionalProperties": false
        }),
        "browser.wait" => json!({
            "type": "object",
            "properties": {
                "selector": { "type": "string", "description": "CSS selector to wait for." },
                "timeoutMs": { "type": "integer", "minimum": 100, "maximum": 30000, "default": 5000 }
            },
            "required": ["selector"],
            "additionalProperties": false
        }),
        "browser.resize" => json!({
            "type": "object",
            "properties": {
                "width": { "type": "integer", "minimum": 1, "maximum": 10000 },
                "height": { "type": "integer", "minimum": 1, "maximum": 10000 }
            },
            "required": ["width", "height"],
            "additionalProperties": false
        }),
        "browser.snapshot" | "browser.screenshot" | "browser.close" => empty_object_schema(),
        _ => json!({
            "type": "object",
            "description": "Operation-specific arguments. Use an empty object when the operation takes no arguments."
        }),
    }
}

fn browser_selector_schema(description: &str) -> Value {
    json!({
        "type": "object",
        "properties": { "selector": { "type": "string", "description": description } },
        "required": ["selector"],
        "additionalProperties": false
    })
}

fn empty_object_schema() -> Value {
    json!({ "type": "object", "maxProperties": 0, "additionalProperties": false })
}

fn authentication_error(error: ControlError) -> McpError {
    McpError::invalid_request(error.message.clone(), serde_json::to_value(error).ok())
}

fn control_tool_error(error: ControlError) -> CallToolResult {
    let value = serde_json::to_value(&error).unwrap_or_else(|_| {
        json!({
            "code": "internal",
            "message": "Luna Mux 控制操作失败",
            "retryable": false
        })
    });
    let mut result = CallToolResult::error(vec![ContentBlock::text(error.message)]);
    result.structured_content = Some(value);
    result
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;
    use crate::{
        control_contract::{ControlCatalog, ControlResponse, ControlResult},
        control_service::LunaControlService,
    };

    fn test_database() -> (Arc<crate::database::Database>, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "{}-mcp-{}.db",
            crate::product::PRODUCT_KEY,
            Uuid::new_v4()
        ));
        let database = Arc::new(
            crate::database::Database::open(
                &path,
                &format!("{}.mcp-test", crate::product::CREDENTIAL_SERVICE),
            )
            .unwrap(),
        );
        (database, path)
    }

    fn test_mcp(
        adapter: Arc<AuthenticatedControlAdapter>,
    ) -> (Arc<LunaMcpService>, std::path::PathBuf) {
        let (database, path) = test_database();
        (
            LunaMcpService::new(
                adapter,
                database,
                crate::agent_hooks::AgentHookService::new(),
                None,
            ),
            path,
        )
    }

    struct RecordingService {
        seen: RwLock<Vec<(String, String)>>,
    }

    struct StaticBrowserCatalog {
        runtimes: Vec<BrowserRuntime>,
    }

    impl BrowserRuntimeCatalog for StaticBrowserCatalog {
        fn list_browser_runtimes(&self) -> Result<Vec<BrowserRuntime>, String> {
            Ok(self.runtimes.clone())
        }
    }

    #[async_trait]
    impl LunaControlService for RecordingService {
        fn catalog(&self, caller: &ControlCaller) -> ControlCatalog {
            ControlCatalog {
                contract_version: CONTROL_CONTRACT_VERSION,
                operations: vec![ControlOperationDescriptor {
                    name: "terminal.runtimes.list".into(),
                    version: 1,
                    access: ControlAccess::Read,
                    resource_kind: ControlResourceKind::TerminalRuntime,
                    mutating: false,
                    supports_idempotency: true,
                    approval: ControlApprovalRequirement::None,
                }]
                .into_iter()
                .filter(|operation| {
                    caller.can_access_any(&operation.resource_kind, &operation.access)
                })
                .collect(),
            }
        }

        async fn invoke(
            &self,
            caller: &ControlCaller,
            request: ControlRequest,
        ) -> ControlResult<ControlResponse> {
            self.seen
                .write()
                .unwrap()
                .push((caller.caller_id.clone(), request.operation.clone()));
            Ok(ControlResponse {
                request_id: request.request_id,
                result: json!({ "routed": true, "grants": caller.grants }),
            })
        }

        fn read_events(
            &self,
            _caller: &ControlCaller,
            _from_sequence: u64,
            _limit: usize,
        ) -> ControlResult<crate::control_contract::ControlEventReadResult> {
            unreachable!()
        }
    }

    fn context(runtime_id: &str) -> TerminalManagedAgentContext {
        TerminalManagedAgentContext {
            mux_session_id: "session-1".into(),
            pane_id: "pane-1".into(),
            runtime_id: runtime_id.into(),
            agent_id: format!("agent-{runtime_id}"),
            launch_profile_id: "codex.default".into(),
        }
    }

    #[test]
    fn runtime_caller_is_scoped_without_application_access() {
        let caller = base_runtime_caller(&context("runtime-1"));
        assert!(caller.has_access(
            &ControlResourceKind::TerminalRuntime,
            Some("runtime-1"),
            &ControlAccess::Control
        ));
        assert!(!caller.has_access(
            &ControlResourceKind::TerminalRuntime,
            Some("runtime-2"),
            &ControlAccess::Read
        ));
        assert!(!caller.can_access_any(&ControlResourceKind::Application, &ControlAccess::Read));
    }

    #[test]
    fn browser_tool_schema_declares_operation_specific_arguments() {
        let tool = tool_for(ControlOperationDescriptor {
            name: "browser.type".into(),
            version: 1,
            access: ControlAccess::Write,
            resource_kind: ControlResourceKind::BrowserRuntime,
            mutating: true,
            supports_idempotency: true,
            approval: ControlApprovalRequirement::User,
        });
        let schema = Value::Object((*tool.input_schema).clone());
        assert_eq!(schema["required"], json!(["resourceId", "arguments"]));
        assert_eq!(
            schema["properties"]["arguments"]["required"],
            json!(["selector", "text"])
        );
        assert_eq!(
            schema["properties"]["arguments"]["properties"]["clear"]["type"],
            "boolean"
        );
    }

    #[tokio::test]
    async fn runtime_tokens_are_isolated_and_revoked_with_runtime() {
        let service = Arc::new(RecordingService {
            seen: RwLock::new(Vec::new()),
        });
        let adapter = AuthenticatedControlAdapter::new(service.clone());
        let (mcp, path) = test_mcp(adapter.clone());
        let first = mcp.issue_runtime_token(&context("runtime-1")).unwrap();
        let second = mcp.issue_runtime_token(&context("runtime-2")).unwrap();
        assert_eq!(adapter.catalog(&first).unwrap().operations.len(), 1);
        assert_eq!(adapter.catalog(&second).unwrap().operations.len(), 1);
        mcp.revoke_runtime("runtime-1");
        assert!(adapter.catalog(&first).is_err());
        assert!(adapter.catalog(&second).is_ok());
        drop(mcp);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn http_entry_rejects_missing_unknown_and_revoked_tokens() {
        let service = Arc::new(RecordingService {
            seen: RwLock::new(Vec::new()),
        });
        let adapter = AuthenticatedControlAdapter::new(service.clone());
        let (mcp, path) = test_mcp(adapter);
        mcp.start().unwrap();
        let endpoint = mcp.endpoint().unwrap();
        let client = reqwest::Client::new();
        let initialize = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "luna-mux-test", "version": "1" }
            }
        });

        let missing = client
            .post(&endpoint)
            .json(&initialize)
            .send()
            .await
            .unwrap();
        assert_eq!(missing.status(), http::StatusCode::UNAUTHORIZED);
        let unknown = client
            .post(&endpoint)
            .bearer_auth("unknown")
            .json(&initialize)
            .send()
            .await
            .unwrap();
        assert_eq!(unknown.status(), http::StatusCode::UNAUTHORIZED);

        let token = mcp.issue_runtime_token(&context("runtime-1")).unwrap();
        let accepted = client
            .post(&endpoint)
            .bearer_auth(&token)
            .header("Accept", "application/json, text/event-stream")
            .json(&initialize)
            .send()
            .await
            .unwrap();
        assert_eq!(accepted.status(), http::StatusCode::OK);
        let session_id = accepted
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .expect("legacy MCP session id")
            .to_string();
        let initialized = client
            .post(&endpoint)
            .bearer_auth(&token)
            .header("Accept", "application/json, text/event-stream")
            .header("mcp-session-id", &session_id)
            .json(&json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(initialized.status(), http::StatusCode::ACCEPTED);

        let tools = client
            .post(&endpoint)
            .bearer_auth(&token)
            .header("Accept", "application/json, text/event-stream")
            .header("mcp-session-id", &session_id)
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list",
                "params": {}
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(tools.status(), http::StatusCode::OK);
        let tools_body = tools.text().await.unwrap();
        assert!(tools_body.contains("terminal.runtimes.list"));

        let called = client
            .post(&endpoint)
            .bearer_auth(&token)
            .header("Accept", "application/json, text/event-stream")
            .header("mcp-session-id", &session_id)
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "terminal.runtimes.list",
                    "arguments": { "arguments": {} }
                }
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(called.status(), http::StatusCode::OK);
        let called_body = called.text().await.unwrap();
        assert!(called_body.contains("routed"), "{called_body}");
        assert_eq!(
            service.seen.read().unwrap().as_slice(),
            &[(
                "agent-runtime:runtime-1".to_string(),
                "terminal.runtimes.list".to_string()
            )]
        );

        mcp.revoke_runtime("runtime-1");
        let revoked = client
            .post(&endpoint)
            .bearer_auth(token)
            .json(&initialize)
            .send()
            .await
            .unwrap();
        assert_eq!(revoked.status(), http::StatusCode::UNAUTHORIZED);
        mcp.shutdown();
        drop(mcp);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn directional_grants_refresh_and_revoke_on_live_runtime_tokens() {
        let service = Arc::new(RecordingService {
            seen: RwLock::new(Vec::new()),
        });
        let adapter = AuthenticatedControlAdapter::new(service);
        let (database, path) = test_database();
        let hooks = crate::agent_hooks::AgentHookService::new();
        let session = database
            .save_mux_session(crate::models::MuxSessionInput {
                id: None,
                name: "MCP grants".into(),
                root_path: String::new(),
                layout: None,
            })
            .unwrap();
        let pane = |id: &str| crate::models::MuxPaneInput {
            id: Some(id.into()),
            mux_session_id: session.id.clone(),
            kind: crate::models::MuxPaneKind::Terminal,
            title: id.into(),
            target_id: "local:powershell".into(),
            bookmark_id: String::new(),
            cwd: String::new(),
            command: String::new(),
            launch_profile_id: "codex.default".into(),
        };
        database.save_mux_pane(pane("pane-source")).unwrap();
        database.save_mux_pane(pane("pane-target")).unwrap();
        let mut source = context("runtime-source");
        source.pane_id = "pane-source".into();
        source.agent_id = "agent-source".into();
        let mut target = context("runtime-target");
        target.pane_id = "pane-target".into();
        target.agent_id = "agent-target".into();
        hooks.issue_token(source.clone()).unwrap();
        hooks.issue_token(target.clone()).unwrap();
        let mcp = LunaMcpService::new(adapter.clone(), database.clone(), hooks, None);
        let token = mcp.issue_runtime_token(&source).unwrap();
        let probe = || ControlRequest {
            contract_version: CONTROL_CONTRACT_VERSION,
            request_id: Uuid::new_v4().to_string(),
            operation: "terminal.runtimes.list".into(),
            resource: None,
            arguments: json!({}),
            idempotency_key: None,
            approval_id: None,
        };
        let before = adapter.invoke(&token, probe()).await.unwrap();
        assert!(
            !before.result["grants"]
                .as_array()
                .unwrap()
                .iter()
                .any(|grant| {
                    grant["resourceKind"] == "agent" && grant["resourceId"] == "agent-target"
                })
        );
        let grant = database
            .save_agent_control_grant(crate::models::AgentControlGrantInput {
                source_pane_id: "pane-source".into(),
                target_resource_kind: "pane".into(),
                target_resource_id: "pane-target".into(),
                access: "control".into(),
            })
            .unwrap();
        mcp.refresh_source_pane("pane-source").unwrap();
        let granted = adapter.invoke(&token, probe()).await.unwrap();
        assert!(
            granted.result["grants"]
                .as_array()
                .unwrap()
                .iter()
                .any(|grant| {
                    grant["resourceKind"] == "agent"
                        && grant["resourceId"] == "agent-target"
                        && grant["access"] == "control"
                })
        );
        database.delete_agent_control_grant(&grant.id).unwrap();
        mcp.refresh_source_pane("pane-source").unwrap();
        let revoked = adapter.invoke(&token, probe()).await.unwrap();
        assert!(
            !revoked.result["grants"]
                .as_array()
                .unwrap()
                .iter()
                .any(|grant| {
                    grant["resourceKind"] == "agent" && grant["resourceId"] == "agent-target"
                })
        );
        drop(mcp);
        drop(database);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn browser_resource_grant_maps_to_the_live_browser_runtime_and_revokes() {
        let service = Arc::new(RecordingService {
            seen: RwLock::new(Vec::new()),
        });
        let adapter = AuthenticatedControlAdapter::new(service);
        let (database, path) = test_database();
        let hooks = crate::agent_hooks::AgentHookService::new();
        let session = database
            .save_mux_session(crate::models::MuxSessionInput {
                id: Some("session-browser-grant".into()),
                name: "Browser grant".into(),
                root_path: String::new(),
                layout: None,
            })
            .unwrap();
        database
            .save_mux_pane(crate::models::MuxPaneInput {
                id: Some("pane-source".into()),
                mux_session_id: session.id.clone(),
                kind: crate::models::MuxPaneKind::Terminal,
                title: "Source Agent".into(),
                target_id: "local:powershell".into(),
                bookmark_id: String::new(),
                cwd: String::new(),
                command: String::new(),
                launch_profile_id: "codex.default".into(),
            })
            .unwrap();
        database
            .save_browser_resource(crate::models::BrowserResourceInput {
                id: Some("browser-resource".into()),
                mux_session_id: session.id.clone(),
                name: "Browser".into(),
                source_pane_id: String::new(),
                bookmark_id: String::new(),
                url: "http://localhost:3000".into(),
                temporary_profile: false,
            })
            .unwrap();
        let mut source = context("runtime-source");
        source.mux_session_id = session.id.clone();
        source.pane_id = "pane-source".into();
        source.agent_id = "agent-source".into();
        hooks.issue_token(source.clone()).unwrap();
        let browser_catalog = Arc::new(StaticBrowserCatalog {
            runtimes: vec![BrowserRuntime {
                id: "browser-runtime-1".into(),
                mux_session_id: session.id,
                browser_resource_id: "browser-resource".into(),
                url: "http://localhost:3000/".into(),
                cdp_port: 43210,
                profile_path: "test-profile".into(),
                process_id: 1234,
                status: crate::browser_runtime::BrowserRuntimeStatus::Running,
                error: None,
            }],
        });
        let mcp = LunaMcpService::new(
            adapter.clone(),
            database.clone(),
            hooks,
            Some(browser_catalog),
        );
        let token = mcp.issue_runtime_token(&source).unwrap();
        let probe = || ControlRequest {
            contract_version: CONTROL_CONTRACT_VERSION,
            request_id: Uuid::new_v4().to_string(),
            operation: "terminal.runtimes.list".into(),
            resource: None,
            arguments: json!({}),
            idempotency_key: None,
            approval_id: None,
        };
        let has_browser = |response: &ControlResponse| {
            response.result["grants"]
                .as_array()
                .unwrap()
                .iter()
                .any(|grant| {
                    grant["resourceKind"] == "browserRuntime"
                        && grant["resourceId"] == "browser-runtime-1"
                        && grant["access"] == "write"
                })
        };
        assert!(!has_browser(
            &adapter.invoke(&token, probe()).await.unwrap()
        ));
        let grant = database
            .save_agent_control_grant(crate::models::AgentControlGrantInput {
                source_pane_id: "pane-source".into(),
                target_resource_kind: "browser".into(),
                target_resource_id: "browser-resource".into(),
                access: "write".into(),
            })
            .unwrap();
        mcp.refresh_target_resource("browser", "browser-resource")
            .unwrap();
        assert!(has_browser(&adapter.invoke(&token, probe()).await.unwrap()));
        database.delete_agent_control_grant(&grant.id).unwrap();
        mcp.refresh_source_pane("pane-source").unwrap();
        assert!(!has_browser(
            &adapter.invoke(&token, probe()).await.unwrap()
        ));

        drop(mcp);
        drop(database);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn tool_schema_never_contains_runtime_tokens() {
        let tool = tool_for(ControlOperationDescriptor {
            name: "terminal.runtimes.list".into(),
            version: 1,
            access: ControlAccess::Read,
            resource_kind: ControlResourceKind::TerminalRuntime,
            mutating: false,
            supports_idempotency: true,
            approval: ControlApprovalRequirement::None,
        });
        let encoded = serde_json::to_string(&tool).unwrap();
        assert!(!encoded.contains(MCP_AUTHORIZATION_ENV));
        assert!(!encoded.contains("lmx_"));
    }
}
