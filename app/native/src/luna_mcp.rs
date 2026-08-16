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
    composite_terminal_backend::CompositeTerminalBackend,
    control_adapter::AuthenticatedControlAdapter,
    control_contract::{
        CONTROL_CONTRACT_VERSION, ControlAccess, ControlApprovalRequirement, ControlCaller,
        ControlCallerKind, ControlError, ControlGrant, ControlOperationDescriptor, ControlRequest,
        ControlResourceKind, ControlResourceRef,
    },
    terminal_backend::TerminalBackend,
    terminal_runtime_contract::{
        TerminalManagedAgentContext, TerminalRuntime, TerminalRuntimeStatus,
    },
};

pub trait TerminalRuntimeCatalog: Send + Sync {
    fn list_terminal_runtimes(&self) -> Result<Vec<TerminalRuntime>, String>;
}

impl TerminalRuntimeCatalog for CompositeTerminalBackend {
    fn list_terminal_runtimes(&self) -> Result<Vec<TerminalRuntime>, String> {
        self.list()
    }
}

pub const MCP_AUTHORIZATION_ENV: &str = "LUNA_MUX_MCP_AUTHORIZATION";

pub struct LunaMcpService {
    adapter: Arc<AuthenticatedControlAdapter>,
    database: Arc<crate::database::Database>,
    agent_hooks: Arc<crate::agent_hooks::AgentHookService>,
    terminal_runtimes: Arc<dyn TerminalRuntimeCatalog>,
    endpoint: RwLock<String>,
    runtime_tokens: RwLock<HashMap<String, Vec<String>>>,
    runtime_contexts: RwLock<HashMap<String, TerminalManagedAgentContext>>,
    cancellation: CancellationToken,
}

impl LunaMcpService {
    pub fn new(
        adapter: Arc<AuthenticatedControlAdapter>,
        database: Arc<crate::database::Database>,
        agent_hooks: Arc<crate::agent_hooks::AgentHookService>,
        terminal_runtimes: Arc<dyn TerminalRuntimeCatalog>,
    ) -> Arc<Self> {
        Arc::new(Self {
            adapter,
            database,
            agent_hooks,
            terminal_runtimes,
            endpoint: RwLock::new(String::new()),
            runtime_tokens: RwLock::new(HashMap::new()),
            runtime_contexts: RwLock::new(HashMap::new()),
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
        self.runtime_contexts
            .write()
            .map_err(|_| "MCP Runtime 上下文锁已损坏".to_string())?
            .insert(context.runtime_id.clone(), context.clone());
        if let Err(error) = self.refresh_session(&context.mux_session_id) {
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
        self.runtime_contexts
            .write()
            .map_err(|_| "MCP Runtime 上下文锁已损坏".to_string())?
            .insert(context.runtime_id.clone(), managed);
        if let Err(error) = self.refresh_session(&context.mux_session_id) {
            self.revoke_token(&token);
            return Err(error);
        }
        Ok(token)
    }

    pub fn refresh_target_resource(
        &self,
        target_resource_kind: &str,
        target_resource_id: &str,
    ) -> Result<(), String> {
        let mux_session_id = match target_resource_kind {
            "pane" => self
                .database
                .list_mux_panes(None)?
                .into_iter()
                .find(|pane| pane.id == target_resource_id)
                .map(|pane| pane.mux_session_id),
            "browser" => self
                .database
                .list_browser_resources(None)?
                .into_iter()
                .find(|resource| resource.id == target_resource_id)
                .map(|resource| resource.mux_session_id),
            _ => None,
        };
        mux_session_id
            .map(|mux_session_id| self.refresh_session(&mux_session_id))
            .unwrap_or(Ok(()))
    }

    pub fn refresh_source_pane(&self, source_pane_id: &str) -> Result<(), String> {
        let mux_session_id = self
            .database
            .list_mux_panes(None)?
            .into_iter()
            .find(|pane| pane.id == source_pane_id)
            .map(|pane| pane.mux_session_id)
            .or_else(|| {
                self.runtime_contexts
                    .read()
                    .ok()?
                    .values()
                    .find(|context| context.pane_id == source_pane_id)
                    .map(|context| context.mux_session_id.clone())
            });
        mux_session_id
            .map(|mux_session_id| self.refresh_session(&mux_session_id))
            .unwrap_or(Ok(()))
    }

    pub fn refresh_session(&self, mux_session_id: &str) -> Result<(), String> {
        let mut contexts = self
            .runtime_contexts
            .read()
            .map_err(|_| "MCP Runtime 上下文锁已损坏".to_string())?
            .values()
            .filter(|context| context.mux_session_id == mux_session_id)
            .cloned()
            .map(|context| (context.runtime_id.clone(), context))
            .collect::<HashMap<_, _>>();
        for snapshot in self
            .agent_hooks
            .snapshots()
            .into_iter()
            .filter(|snapshot| snapshot.context.mux_session_id == mux_session_id)
        {
            contexts.insert(snapshot.context.runtime_id.clone(), snapshot.context);
        }
        let runtime_tokens = self
            .runtime_tokens
            .read()
            .map_err(|_| "MCP Runtime 授权锁已损坏".to_string())?
            .clone();
        for context in contexts.into_values() {
            let caller = self.runtime_caller(&context)?;
            if let Some(tokens) = runtime_tokens.get(&context.runtime_id) {
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
        for pane in self
            .database
            .list_mux_panes(Some(&context.mux_session_id))?
        {
            caller.grants.push(ControlGrant {
                resource_kind: ControlResourceKind::Pane,
                resource_id: Some(pane.id),
                access: ControlAccess::Write,
            });
        }
        for runtime in self
            .terminal_runtimes
            .list_terminal_runtimes()?
            .into_iter()
            .filter(|runtime| {
                matches!(
                    runtime.status,
                    TerminalRuntimeStatus::Starting
                        | TerminalRuntimeStatus::Connecting
                        | TerminalRuntimeStatus::Running
                ) && runtime
                    .managed_agent
                    .as_ref()
                    .map(|managed| managed.mux_session_id.as_str())
                    .or_else(|| {
                        runtime
                            .context
                            .as_ref()
                            .map(|context| context.mux_session_id.as_str())
                    })
                    == Some(context.mux_session_id.as_str())
            })
        {
            caller.grants.push(ControlGrant {
                resource_kind: ControlResourceKind::TerminalRuntime,
                resource_id: Some(runtime.runtime_id),
                access: ControlAccess::Control,
            });
        }
        for snapshot in snapshots
            .into_iter()
            .filter(|snapshot| snapshot.context.mux_session_id == context.mux_session_id)
        {
            caller.grants.push(ControlGrant {
                resource_kind: ControlResourceKind::Agent,
                resource_id: Some(snapshot.context.agent_id),
                access: ControlAccess::Control,
            });
        }
        Ok(caller)
    }

    pub fn revoke_token(&self, token: &str) {
        let _ = self.adapter.revoke_token(token);
        let mut empty_runtime_ids = Vec::new();
        if let Ok(mut runtimes) = self.runtime_tokens.write() {
            runtimes.retain(|runtime_id, tokens| {
                tokens.retain(|current| current != token);
                let keep = !tokens.is_empty();
                if !keep {
                    empty_runtime_ids.push(runtime_id.clone());
                }
                keep
            });
        }
        if let Ok(mut contexts) = self.runtime_contexts.write() {
            for runtime_id in empty_runtime_ids {
                contexts.remove(&runtime_id);
            }
        }
    }

    pub fn revoke_runtime(&self, runtime_id: &str) {
        let mux_session_id = self
            .runtime_contexts
            .write()
            .ok()
            .and_then(|mut contexts| contexts.remove(runtime_id))
            .map(|context| context.mux_session_id);
        let tokens = self
            .runtime_tokens
            .write()
            .ok()
            .and_then(|mut runtimes| runtimes.remove(runtime_id))
            .unwrap_or_default();
        for token in tokens {
            let _ = self.adapter.revoke_token(&token);
        }
        if let Some(mux_session_id) = mux_session_id {
            let _ = self.refresh_session(&mux_session_id);
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
                access: ControlAccess::Write,
            },
            ControlGrant {
                resource_kind: ControlResourceKind::Pane,
                resource_id: Some(context.pane_id.clone()),
                access: ControlAccess::Write,
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
            ControlGrant {
                resource_kind: ControlResourceKind::Settings,
                resource_id: None,
                access: ControlAccess::Write,
            },
            ControlGrant {
                resource_kind: ControlResourceKind::ConnectionProfile,
                resource_id: None,
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
            "This server controls resources owned by the Luna Mux desktop application: app theme and terminal appearance, saved connection summaries and terminal targets, Mux Sessions, panes and split layouts, Luna-owned terminal runtimes, managed Agents, SFTP transfers, and SSH tunnels. Use it only when the object being inspected or changed belongs to Luna Mux. Do not use it for ordinary source-code, filesystem, shell, Git, operating-system, or external-service work merely because the caller runs inside Luna Mux. Route any unqualified request about 窗格, Pane, 新建窗格, 分屏, split, 布局, or layout here, not to agent_browser. Read settings.appearance.get before changing Luna Mux theme or terminal appearance. Use terminal.targets.list followed by mux.pane.create to create and optionally start a terminal Pane; use mux.layout.set for a complete validated split layout. Use agents.* for managed Agent processes shown by Luna Mux, not as a substitute for the caller's own subagent/delegation features. Use terminal.runtime.* when the user wants to control a Luna-owned Pane runtime, especially another Pane; use the caller's normal shell for commands that are simply part of the current coding task. Session, Pane, and layout edits are limited to the caller's current Mux Session. Connection tools never expose saved credentials or private-key contents. Mutating terminal, transfer, and tunnel operations may require approval in the trusted desktop UI. Browser automation is provided by the separate native agent-browser server; use that server only for content rendered by a webpage: navigation, snapshots, interaction, browser tabs/windows, page screenshots, browser console, and page network inspection. A Luna Mux Pane is never a browser tab or window.",
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
    let routing_guidance = operation_routing_guidance(&descriptor.name);
    let description = format!(
        "Luna Mux control operation `{}`. Resource kind: {:?}. Access: {:?}.{}{} Put operation-specific fields inside `arguments`.",
        descriptor.name,
        descriptor.resource_kind,
        descriptor.access,
        if approval {
            " This operation requires approval in the trusted desktop UI; retry with the returned approvalId."
        } else {
            ""
        },
        routing_guidance,
    );
    let arguments_schema = control_arguments_schema(&descriptor.name);
    let resource_required = !matches!(
        descriptor.name.as_str(),
        "settings.appearance.get"
            | "settings.theme.set"
            | "settings.terminal.set"
            | "connections.list"
            | "agents.list"
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

fn operation_routing_guidance(operation: &str) -> &'static str {
    match operation {
        "settings.appearance.get" | "settings.theme.set" | "settings.terminal.set" => {
            " Use only for the Luna Mux desktop application's own theme or terminal rendering preferences. Do not use for webpage CSS/theme, repository configuration, or operating-system appearance."
        }
        "connections.list" => {
            " Lists credential-free summaries of connections saved in Luna Mux. This is not browser network traffic, an HTTP connection list, or a general host/network scan."
        }
        "agents.list" | "agents.get_status" | "agents.send_task" | "agents.interrupt" => {
            " Controls managed Agent processes detected in Luna Mux terminal Panes. Do not use as a substitute for this coding agent's own subagent/delegation tools or for website chatbots."
        }
        "mux.sessions.list" | "mux.session.update" => {
            " Operates on Luna Mux project Sessions shown in the application sidebar. This is not a browser session, login session, shell environment, or named agent_browser session."
        }
        "mux.pane.create" => {
            " Preferred tool whenever the user asks to create, add, or open a Luna Mux 窗格/Pane, terminal pane, or one side of a split. An unqualified 窗格/Pane is an application Pane, not a browser tab or window. Call terminal.targets.list first when targetId is unknown."
        }
        "mux.layout.set" => {
            " Use for Luna Mux 分屏/split and 布局/layout changes after the required Panes exist. This arranges application Panes, not browser tabs, browser windows, webpage layout, or source-code layout."
        }
        "mux.panes.list" | "mux.pane.update" => {
            " Inspects or updates Luna Mux terminal 窗格/Panes in the caller's current Mux Session. Do not use browser tab/window tools for these application resources."
        }
        "terminal.targets.list" => {
            " Lists local, WSL, and saved SSH launch targets available to Luna Mux when creating a Pane. This is not a list of shell build targets, browser targets, or network hosts discovered from the web."
        }
        name if name.starts_with("terminal.runtime.") || name == "terminal.runtimes.list" => {
            " Operates on terminal Runtimes owned by Luna Mux Panes, including another Pane's bounded output/input, size, flow, interrupt, or close state. Use the normal shell tool instead when a command is merely part of the current coding task. This does not control a browser viewport or webpage console."
        }
        "transfer.enqueue" | "transfers.list" | "transfer.cancel" => {
            " Operates on Luna Mux's SFTP transfer queue attached to a terminal Runtime. Do not use for webpage uploads/downloads, HTTP requests, package downloads, or ordinary local file copies."
        }
        "tunnel.start" | "tunnels.list" | "tunnel.profiles.list" | "tunnel.stop" => {
            " Operates on SSH port-forward tunnels managed by Luna Mux. Do not use for browser proxies, webpage network inspection, application-layer HTTP calls, or unrelated VPN/OS networking."
        }
        _ => {
            " Use only when the target resource is owned and displayed by the Luna Mux desktop application; otherwise choose the shell, filesystem, browser, or external-service tool that owns the requested state."
        }
    }
}

fn control_arguments_schema(operation: &str) -> Value {
    match operation {
        "settings.appearance.get"
        | "connections.list"
        | "agents.list"
        | "agents.get_status"
        | "agents.interrupt"
        | "mux.sessions.list"
        | "terminal.targets.list"
        | "terminal.runtimes.list"
        | "terminal.runtime.interrupt"
        | "terminal.runtime.close"
        | "transfers.list"
        | "tunnels.list" => empty_object_schema(),
        "settings.theme.set" => json!({
            "type": "object",
            "properties": {
                "theme": {
                    "type": "string",
                    "enum": ["system", "light", "dark"],
                    "description": "Application color theme. System follows the operating-system appearance."
                }
            },
            "required": ["theme"],
            "additionalProperties": false
        }),
        "settings.terminal.set" => json!({
            "type": "object",
            "properties": {
                "fontFamily": { "type": "string", "description": "Terminal font-family CSS value." },
                "fontSize": { "type": "integer", "minimum": 10, "maximum": 32 },
                "foregroundColor": { "type": "string", "description": "Terminal foreground CSS color." },
                "backgroundColor": { "type": "string", "description": "Terminal background CSS color." },
                "backgroundOpacity": { "type": "number", "minimum": 0.1, "maximum": 1.0 },
                "backgroundImagePath": { "type": "string", "description": "Existing local background image path, or empty to disable." },
                "backgroundImageFit": { "type": "string", "enum": ["cover", "contain", "stretch", "tile"] }
            },
            "required": ["fontFamily", "fontSize", "foregroundColor", "backgroundColor", "backgroundOpacity", "backgroundImagePath", "backgroundImageFit"],
            "additionalProperties": false
        }),
        "agents.send_task" => json!({
            "type": "object",
            "properties": {
                "task": { "type": "string", "minLength": 1, "description": "Task text to submit to the target Agent." }
            },
            "required": ["task"],
            "additionalProperties": false
        }),
        "mux.panes.list" => json!({
            "type": "object",
            "properties": {
                "muxSessionId": { "type": "string", "description": "Optional Session filter. Agent callers can only see their own Session." }
            },
            "additionalProperties": false
        }),
        "mux.session.update" => json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "minLength": 1, "description": "New Session display name." },
                "rootPath": { "type": "string", "description": "Default working directory for the Session." }
            },
            "minProperties": 1,
            "additionalProperties": false
        }),
        "mux.layout.set" => json!({
            "type": "object",
            "properties": {
                "layout": {
                    "type": ["object", "null"],
                    "description": "Complete validated split tree. A leaf is {type:'pane',paneId}; a split is {type:'split',direction:'horizontal'|'vertical',ratio:0.1..0.9,first:<node>,second:<node>}. Every Pane in the Session must appear exactly once. Use null only when the Session has no Panes."
                }
            },
            "required": ["layout"],
            "additionalProperties": false
        }),
        "mux.pane.update" => json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "minLength": 1, "description": "New pane title." },
                "cwd": { "type": "string", "description": "Working directory used when the pane restarts." },
                "command": { "type": "string", "description": "Command used when the pane starts or restarts." },
                "launchProfileId": { "type": "string", "description": "Managed Agent launch profile, or empty for a plain terminal." }
            },
            "minProperties": 1,
            "additionalProperties": false
        }),
        "mux.pane.create" => json!({
            "type": "object",
            "properties": {
                "targetId": { "type": "string", "minLength": 1, "description": "Terminal target from terminal.targets.list." },
                "title": { "type": "string", "description": "Optional pane title; defaults to the target label." },
                "cwd": { "type": "string", "description": "Optional working directory; defaults to the Session root path." },
                "command": { "type": "string", "description": "Optional initial shell command." },
                "launchProfileId": { "type": "string", "description": "Optional managed Agent launch profile ID." },
                "anchorPaneId": { "type": "string", "description": "Optional existing Pane beside which the new Pane is inserted." },
                "splitDirection": { "type": "string", "enum": ["horizontal", "vertical"], "default": "horizontal" },
                "splitRatio": { "type": "number", "minimum": 0.1, "maximum": 0.9, "default": 0.5, "description": "Fraction assigned to the existing/first side." },
                "start": { "type": "boolean", "default": true, "description": "Start the new terminal Runtime immediately in the desktop UI." }
            },
            "required": ["targetId"],
            "additionalProperties": false
        }),
        "terminal.runtime.output.read" => json!({
            "type": "object",
            "properties": {
                "fromCursor": { "type": "integer", "minimum": 0, "default": 0, "description": "UTF-8 byte cursor returned by the previous read." },
                "maxBytes": { "type": "integer", "minimum": 1, "maximum": 1048576, "default": 65536 }
            },
            "additionalProperties": false
        }),
        "terminal.runtime.write" => json!({
            "type": "object",
            "properties": {
                "data": { "type": "string", "description": "Exact bytes to write to the target PTY. End a shell command with a carriage return." }
            },
            "required": ["data"],
            "additionalProperties": false
        }),
        "terminal.runtime.resize" => json!({
            "type": "object",
            "properties": {
                "cols": { "type": "integer", "minimum": 1, "maximum": 10000 },
                "rows": { "type": "integer", "minimum": 1, "maximum": 10000 }
            },
            "required": ["cols", "rows"],
            "additionalProperties": false
        }),
        "terminal.runtime.flow.set" => json!({
            "type": "object",
            "properties": { "paused": { "type": "boolean" } },
            "required": ["paused"],
            "additionalProperties": false
        }),
        "tunnel.profiles.list" => json!({
            "type": "object",
            "properties": {
                "bookmarkId": { "type": "string", "description": "Optional connection-profile filter." }
            },
            "additionalProperties": false
        }),
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
                Arc::new(StaticTerminalCatalog::default()),
            ),
            path,
        )
    }

    struct RecordingService {
        seen: RwLock<Vec<(String, String)>>,
    }

    #[derive(Default)]
    struct StaticTerminalCatalog {
        runtimes: RwLock<Vec<TerminalRuntime>>,
    }

    impl TerminalRuntimeCatalog for StaticTerminalCatalog {
        fn list_terminal_runtimes(&self) -> Result<Vec<TerminalRuntime>, String> {
            Ok(self.runtimes.read().unwrap().clone())
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
        assert!(caller.can_access_any(&ControlResourceKind::Settings, &ControlAccess::Write));
    }

    #[test]
    fn theme_tool_schema_is_global_and_enumerates_supported_values() {
        let tool = tool_for(ControlOperationDescriptor {
            name: "settings.theme.set".into(),
            version: 1,
            access: ControlAccess::Write,
            resource_kind: ControlResourceKind::Settings,
            mutating: true,
            supports_idempotency: true,
            approval: ControlApprovalRequirement::None,
        });
        let schema = Value::Object((*tool.input_schema).clone());
        assert_eq!(schema["required"], json!(["arguments"]));
        assert_eq!(
            schema["properties"]["arguments"]["properties"]["theme"]["enum"],
            json!(["system", "light", "dark"])
        );
    }

    #[test]
    fn operation_descriptions_disambiguate_luna_mux_resource_domains() {
        let cases = [
            ("settings.theme.set", "webpage CSS/theme"),
            ("connections.list", "browser network traffic"),
            ("agents.send_task", "subagent/delegation"),
            ("mux.sessions.list", "browser session"),
            ("mux.pane.create", "not a browser tab or window"),
            ("mux.layout.set", "webpage layout"),
            ("terminal.targets.list", "shell build targets"),
            ("terminal.runtime.resize", "browser viewport"),
            ("transfer.enqueue", "webpage uploads/downloads"),
            ("tunnel.start", "browser proxies"),
        ];
        for (operation, expected) in cases {
            assert!(
                operation_routing_guidance(operation).contains(expected),
                "{operation} should distinguish itself from {expected}"
            );
        }
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

    #[test]
    fn collaboration_tool_schemas_declare_required_arguments() {
        let descriptor =
            |name: &str, access: ControlAccess, resource_kind| ControlOperationDescriptor {
                name: name.into(),
                version: 1,
                access,
                resource_kind,
                mutating: true,
                supports_idempotency: true,
                approval: ControlApprovalRequirement::None,
            };
        let write = tool_for(descriptor(
            "terminal.runtime.write",
            ControlAccess::Write,
            ControlResourceKind::TerminalRuntime,
        ));
        let write_schema = Value::Object((*write.input_schema).clone());
        assert_eq!(
            write_schema["properties"]["arguments"]["required"],
            json!(["data"])
        );
        let send_task = tool_for(descriptor(
            "agents.send_task",
            ControlAccess::Write,
            ControlResourceKind::Agent,
        ));
        let task_schema = Value::Object((*send_task.input_schema).clone());
        assert_eq!(
            task_schema["properties"]["arguments"]["required"],
            json!(["task"])
        );
        assert_eq!(
            control_arguments_schema("terminal.runtime.output.read")["properties"]["fromCursor"]["minimum"],
            0
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
    async fn session_members_share_live_panes_and_terminal_runtimes_by_default() {
        let service = Arc::new(RecordingService {
            seen: RwLock::new(Vec::new()),
        });
        let adapter = AuthenticatedControlAdapter::new(service);
        let (database, path) = test_database();
        let hooks = crate::agent_hooks::AgentHookService::new();
        let session = database
            .save_mux_session(crate::models::MuxSessionInput {
                id: Some("session-shared".into()),
                name: "Shared session".into(),
                root_path: String::new(),
                layout: None,
            })
            .unwrap();
        let other_session = database
            .save_mux_session(crate::models::MuxSessionInput {
                id: Some("session-private".into()),
                name: "Private session".into(),
                root_path: String::new(),
                layout: None,
            })
            .unwrap();
        let pane = |id: &str, mux_session_id: &str| crate::models::MuxPaneInput {
            id: Some(id.into()),
            mux_session_id: mux_session_id.into(),
            kind: crate::models::MuxPaneKind::Terminal,
            title: id.into(),
            target_id: "local:powershell".into(),
            bookmark_id: String::new(),
            cwd: String::new(),
            command: String::new(),
            launch_profile_id: String::new(),
        };
        database
            .save_mux_pane(pane("pane-source", &session.id))
            .unwrap();
        database
            .save_mux_pane(pane("pane-bash", &session.id))
            .unwrap();
        database
            .save_mux_pane(pane("pane-private", &other_session.id))
            .unwrap();
        let mut source = context("runtime-source");
        source.mux_session_id = session.id.clone();
        source.pane_id = "pane-source".into();
        source.agent_id = "agent-source".into();
        let runtime = |runtime_id: &str, pane_id: &str, mux_session_id: &str| TerminalRuntime {
            runtime_id: runtime_id.into(),
            target_id: "local:shell".into(),
            title: pane_id.into(),
            status: TerminalRuntimeStatus::Running,
            capabilities: crate::terminal_backend::standard_terminal_capabilities(false),
            context: Some(crate::terminal_runtime_contract::TerminalRuntimeContext {
                mux_session_id: mux_session_id.into(),
                pane_id: pane_id.into(),
                runtime_id: runtime_id.into(),
            }),
            managed_agent: None,
            error: None,
        };
        let terminal_catalog = Arc::new(StaticTerminalCatalog {
            runtimes: RwLock::new(vec![
                runtime("runtime-source", "pane-source", &session.id),
                runtime("runtime-bash", "pane-bash", &session.id),
                runtime("runtime-private", "pane-private", &other_session.id),
            ]),
        });
        let mcp = LunaMcpService::new(
            adapter.clone(),
            database.clone(),
            hooks,
            terminal_catalog.clone(),
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
        let response = adapter.invoke(&token, probe()).await.unwrap();
        let grants = response.result["grants"].as_array().unwrap();
        let has_grant = |kind: &str, id: &str, access: &str| {
            grants.iter().any(|grant| {
                grant["resourceKind"] == kind
                    && grant["resourceId"] == id
                    && grant["access"] == access
            })
        };
        assert!(has_grant("pane", "pane-bash", "write"));
        assert!(has_grant("terminalRuntime", "runtime-bash", "control"));
        assert!(!grants.iter().any(|grant| {
            grant["resourceId"] == "pane-private" || grant["resourceId"] == "runtime-private"
        }));

        terminal_catalog.runtimes.write().unwrap().push(runtime(
            "runtime-later",
            "pane-bash",
            &session.id,
        ));
        mcp.refresh_session(&session.id).unwrap();
        let refreshed = adapter.invoke(&token, probe()).await.unwrap();
        assert!(
            refreshed.result["grants"]
                .as_array()
                .unwrap()
                .iter()
                .any(|grant| {
                    grant["resourceKind"] == "terminalRuntime"
                        && grant["resourceId"] == "runtime-later"
                        && grant["access"] == "control"
                })
        );

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
