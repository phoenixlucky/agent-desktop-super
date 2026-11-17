use axum::{Router, extract::State, http::StatusCode, middleware, routing::post};
use clap::ArgMatches;
use rmcp::{
    RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ListToolsResult,
        PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerConfig, Tool,
    },
    service::RequestContext,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, session::local::LocalSessionManager,
    },
};
use serde_json::{Map, Value, json};
use std::process::ExitCode;
use std::{borrow::Cow, sync::Arc};
use tokio::sync::{Semaphore, watch};

use super::{MAX_CONCURRENT_CALLS, call, calls::Calls, catalog, forwarded_globals};
#[path = "http_auth.rs"]
mod http_auth;
use self::http_auth::{authorize_request, generate_token};

pub(crate) fn as_rmcp_tool(tool: &catalog::Tool) -> Result<Tool, serde_json::Error> {
    serde_json::from_value(tool.definition())
}

pub(crate) fn call_result(mut result: Value) -> Result<CallToolResult, serde_json::Error> {
    if let Some(fields) = result.as_object_mut() {
        fields
            .entry("resultType")
            .or_insert_with(|| Value::from("complete"));
    }
    serde_json::from_value(result)
}

pub(crate) fn tool_concurrency_gate() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(MAX_CONCURRENT_CALLS))
}

fn http_protocol_versions() -> &'static [ProtocolVersion] {
    &ProtocolVersion::KNOWN_VERSIONS[1..]
}

pub(crate) fn command_arg(command: clap::Command) -> clap::Command {
    command.arg(
        clap::Arg::new("transport")
            .long("transport")
            .value_parser(["stdio", "streamable-http"])
            .default_value("stdio")
            .help("MCP transport to serve"),
    )
}

pub(crate) fn serve(matches: &ArgMatches) -> ExitCode {
    let result = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())
        .and_then(|runtime| {
            runtime
                .block_on(run_http(matches))
                .map_err(|error| error.to_string())
        });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("agent-desktop mcp: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run_http(matches: &ArgMatches) -> Result<(), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:12899").await?;
    let token = generate_token().map_err(|error| std::io::Error::other(error.to_string()))?;
    let (shutdown, mut shutdown_rx) = watch::channel(false);
    let app = router(forwarded_globals(matches), token.clone(), shutdown)?;
    emit_ready(&token)?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.changed().await;
        })
        .await?;
    Ok(())
}

fn router(
    globals: Vec<String>,
    token: String,
    shutdown: watch::Sender<bool>,
) -> Result<Router, std::io::Error> {
    let state = Arc::new(HttpState {
        tools: catalog::tools(),
        globals,
        calls: Calls::with_child_job()?,
        gate: tool_concurrency_gate(),
        shutdown,
    });
    let handler_state = Arc::clone(&state);
    let service = rmcp::transport::streamable_http_server::tower::StreamableHttpService::new(
        move || Ok(HttpServer(Arc::clone(&handler_state))),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    Ok(Router::new()
        .route("/__shutdown", post(request_shutdown))
        .nest_service("/desktop", service)
        .with_state(state)
        .route_layer(middleware::from_fn_with_state(token, authorize_request)))
}

async fn request_shutdown(State(state): State<Arc<HttpState>>) -> StatusCode {
    state.calls.cancel_all();
    state.shutdown.send_replace(true);
    StatusCode::ACCEPTED
}

fn emit_ready(token: &str) -> Result<(), Box<dyn std::error::Error>> {
    let tools = catalog::tools()
        .iter()
        .map(|tool| {
            let definition = tool.definition();
            json!({ "name": tool.name, "description": definition["description"] })
        })
        .collect::<Vec<_>>();
    let metadata = json!({
        "event": "ready",
        "url": "http://127.0.0.1:12899/desktop",
        "token": token,
        "tools": tools,
    });
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer(&mut stdout, &metadata)?;
    use std::io::Write;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

struct HttpState {
    tools: Vec<catalog::Tool>,
    globals: Vec<String>,
    calls: Calls,
    gate: Arc<Semaphore>,
    shutdown: watch::Sender<bool>,
}

struct HttpServer(Arc<HttpState>);

impl ServerHandler for HttpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(super::INSTRUCTIONS)
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(http_protocol_versions())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        let tools = self
            .0
            .tools
            .iter()
            .map(as_rmcp_tool)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| rmcp::ErrorData::internal_error(error.to_string(), None))?;
        Ok(ListToolsResult {
            tools,
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, rmcp::ErrorData> {
        let permit = match self.0.gate.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => return busy_result(),
        };
        let tool = self
            .0
            .tools
            .iter()
            .find(|tool| tool.name == request.name)
            .ok_or_else(|| {
                rmcp::ErrorData::invalid_params(format!("Unknown tool: {}", request.name), None)
            })?;
        let arguments = request
            .arguments
            .map(Value::Object)
            .unwrap_or_else(|| json!({}));
        let arguments = arguments.as_object().cloned().ok_or_else(|| {
            rmcp::ErrorData::invalid_params("Tool arguments must be an object", None)
        })?;
        let id = serde_json::to_value(&context.id).unwrap_or(Value::Null);
        if !self.0.calls.admit(&id) {
            return Err(rmcp::ErrorData::internal_error(
                "Unable to admit tool call",
                None,
            ));
        }
        let mut worker = execute_call(self.0.clone(), tool.name.clone(), arguments, id.clone());
        let result = tokio::select! {
            result = &mut worker => result.unwrap_or_else(|error| json!({"content":[{"type":"text","text":format!("Tool worker failed: {error}")}],"isError":true})),
            () = context.ct.cancelled() => {
                self.0.calls.cancel(&id);
                let _ = worker.await;
                json!({"content":[{"type":"text","text":"Cancelled"}],"isError":true})
            }
        };
        self.0.calls.finish(&id);
        drop(permit);
        call_result(result)
            .map(CallToolResponse::Complete)
            .map_err(|error| rmcp::ErrorData::internal_error(error.to_string(), None))
    }
}

fn busy_result() -> Result<CallToolResponse, rmcp::ErrorData> {
    let message =
        format!("{MAX_CONCURRENT_CALLS} tool calls are already running; retry when one finishes");
    let value =
        json!({"content":[{"type":"text","text":message}],"isError":true,"resultType":"complete"});
    call_result(value)
        .map(CallToolResponse::Complete)
        .map_err(|error| rmcp::ErrorData::internal_error(error.to_string(), None))
}

fn execute_call(
    state: Arc<HttpState>,
    name: String,
    arguments: Map<String, Value>,
    id: Value,
) -> tokio::task::JoinHandle<Value> {
    tokio::task::spawn_blocking(move || {
        let Some(tool) = state.tools.iter().find(|tool| tool.name == name) else {
            return json!({"content":[{"type":"text","text":"Tool disappeared during dispatch"}],"isError":true});
        };
        call::call(tool, &arguments, &state.globals, &state.calls, &id)
    })
}

#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;
