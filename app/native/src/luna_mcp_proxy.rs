use rmcp::{
    ErrorData as McpError, Peer, RoleClient, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, ListToolsResult, PaginatedRequestParams,
        ServerInfo,
    },
    service::RequestContext,
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};

use crate::luna_mcp::MCP_AUTHORIZATION_ENV;

#[derive(Clone)]
struct LunaMcpStdioProxy {
    upstream: Peer<RoleClient>,
    info: ServerInfo,
}

impl ServerHandler for LunaMcpStdioProxy {
    fn get_info(&self) -> ServerInfo {
        self.info.clone()
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<rmcp::RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        self.upstream.list_tools(request).await.map_err(proxy_error)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<rmcp::RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        self.upstream
            .call_tool(request)
            .await
            .map(Into::into)
            .map_err(proxy_error)
    }
}

fn proxy_error(error: impl std::fmt::Display) -> McpError {
    McpError::internal_error(format!("Luna MCP 代理失败：{error}"), None)
}

async fn run_proxy() -> Result<(), String> {
    let endpoint = std::env::var("LUNA_MUX_MCP_ENDPOINT")
        .map_err(|_| "缺少 LUNA_MUX_MCP_ENDPOINT".to_string())?;
    let token = std::env::var(MCP_AUTHORIZATION_ENV)
        .map_err(|_| format!("缺少 {MCP_AUTHORIZATION_ENV}"))?;
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(endpoint).auth_header(token),
    );
    let upstream = ().serve(transport).await.map_err(|error| error.to_string())?;
    let peer = upstream.peer().clone();
    let peer_info = peer
        .peer_info()
        .ok_or_else(|| "Luna MCP 未返回初始化信息".to_string())?;
    let mut info = ServerInfo::new(peer_info.capabilities.clone())
        .with_protocol_version(peer_info.protocol_version.clone());
    if let Some(server_info) = peer_info.server_info.clone() {
        info = info.with_server_info(server_info);
    }
    if let Some(instructions) = peer_info.instructions.clone() {
        info = info.with_instructions(instructions);
    }
    let upstream_task = tokio::spawn(async move { upstream.waiting().await });
    let downstream = LunaMcpStdioProxy {
        upstream: peer,
        info,
    }
    .serve(rmcp::transport::stdio())
    .await
    .map_err(|error| error.to_string())?;
    downstream
        .waiting()
        .await
        .map_err(|error| error.to_string())?;
    upstream_task.abort();
    Ok(())
}

pub fn try_run_luna_mcp_proxy(args: &[String]) -> Option<i32> {
    if args.get(1).map(String::as_str) != Some("mcp")
        || args.get(2).map(String::as_str) != Some("luna")
    {
        return None;
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("无法启动 Luna MCP 代理 Runtime：{error}");
            return Some(1);
        }
    };
    match runtime.block_on(run_proxy()) {
        Ok(()) => Some(0),
        Err(error) => {
            eprintln!("无法启动 Luna MCP 代理：{error}");
            Some(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_unrelated_child_process_modes() {
        assert_eq!(
            try_run_luna_mcp_proxy(&["luna-mux".into(), "mcp".into(), "browser".into()]),
            None
        );
        assert_eq!(try_run_luna_mcp_proxy(&["luna-mux".into()]), None);
    }
}
