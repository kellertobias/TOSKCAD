//! MCP protocol binding (rmcp) over the typed tool catalog. Transport is
//! stdio only: the client launches the server, so no port is ever opened.
use crate::{
    host::Host,
    tools::{self, Call},
};
use multicad_core::commands::{Actor, Channel};
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool, ToolAnnotations,
    },
    service::RequestContext,
};
use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

const INSTRUCTIONS: &str = "MultiCAD controls one canonical parametric document. Lengths are explicit unit-bearing values; coordinates are millimeters. Read the current revision with document_info and pass it as expected_revision to every mutating tool; a stale revision is refused and nothing changes. Each mutation commits one undoable transaction attributed to this client. Formulas start with '=' and reference parameters with '$'. Region indices from inspect_sketch are valid only at the revision they were reported for.";

#[derive(Clone)]
pub struct Server {
    host: Arc<dyn Host>,
}
impl Server {
    pub fn new(host: Arc<dyn Host>) -> Self {
        Self { host }
    }
    fn tools() -> Vec<Tool> {
        tools::catalog()
            .into_iter()
            .map(|t| {
                let Value::Object(schema) = t.schema else {
                    unreachable!("schemars produces object schemas");
                };
                Tool::new(t.name, t.description, schema)
                    .with_title(t.title)
                    .with_annotations(
                        ToolAnnotations::with_title(t.title)
                            .read_only(t.read_only)
                            .destructive(false)
                            .open_world(false),
                    )
            })
            .collect()
    }
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        let mut implementation = Implementation::new("multicad", env!("CARGO_PKG_VERSION"));
        implementation.title = Some("MultiCAD".into());
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(implementation)
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(Self::tools()))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        Self::tools().into_iter().find(|t| t.name == name)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let client = context
            .peer
            .peer_info()
            .map(|info| info.client_info.name.clone())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "unnamed".into());
        let call = Call {
            actor: Actor {
                id: format!("mcp:{}", client.chars().take(64).collect::<String>()),
                channel: Channel::Mcp,
            },
            cancel: Arc::new(AtomicBool::new(false)),
        };
        let cancel = Arc::clone(&call.cancel);
        let token = context.ct.clone();
        let watcher = tokio::spawn(async move {
            token.cancelled().await;
            cancel.store(true, Ordering::Relaxed);
        });
        let host = Arc::clone(&self.host);
        let name = request.name.to_string();
        let arguments = request.arguments.map(Value::Object).unwrap_or(Value::Null);
        let result = tokio::task::spawn_blocking(move || {
            tools::dispatch(host.as_ref(), &call, &name, arguments)
        })
        .await;
        watcher.abort();
        let result = result.map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(match result {
            Ok(value) => CallToolResult::structured(value),
            Err(error) => CallToolResult::structured_error(
                serde_json::to_value(&error).expect("tool errors serialize"),
            ),
        }
        .into())
    }
}

/// Serve MCP on this process's stdin/stdout until the client disconnects.
pub fn run_stdio(host: Arc<dyn Host>) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(async move {
        let service = Server::new(host)
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|e| e.to_string())?;
        service.waiting().await.map_err(|e| e.to_string())?;
        Ok(())
    })
}
