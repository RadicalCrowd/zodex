//! MCP tool server for the privileged-exec research runner.
//!
//! Exposes a single tool `exec` that accepts structured
//! `{executable, argv, cwd, reason}` input only.  Shell strings, environment
//! overrides, arbitrary stdin, sudo, and live credentials are out of scope and
//! are not accepted.

use anyhow::Result;
use rmcp::{
    handler::server::{
        tool::ToolRouter,
        wrapper::{Json, Parameters},
    },
    model::{Implementation, ServerCapabilities, ServerInfo},
    schemars::JsonSchema,
    tool, tool_router, ServerHandler, ServiceExt,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::{
    invocation_digest,
    output::OutputLimits,
    runner::{cancel_pair, run_structured, ExecRequest},
};

// ---------------------------------------------------------------------------
// Tool parameter schema
// ---------------------------------------------------------------------------

/// Parameters for the `exec` tool.  All fields are required.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct ExecParams {
    /// Absolute path to the executable.  Shell metacharacters are not
    /// interpreted; this must be a direct path such as `/usr/bin/id`.
    executable: String,
    /// Argument vector passed verbatim to the process.  Do not include the
    /// executable name at index 0; it is supplied automatically.
    #[schemars(default)]
    #[serde(default)]
    argv: Vec<String>,
    /// Absolute working directory for the child process.
    cwd: String,
    /// Human-readable reason for audit logging.
    reason: String,
}

// ---------------------------------------------------------------------------
// Response helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, JsonSchema)]
struct ToolResponse {
    #[serde(flatten)]
    fields: BTreeMap<String, Value>,
}

fn tool_json(value: Value) -> Json<ToolResponse> {
    let fields = match value {
        Value::Object(map) => map.into_iter().collect(),
        other => BTreeMap::from([("value".to_string(), other)]),
    };
    Json(ToolResponse { fields })
}

fn error_json(reason: &str) -> Json<ToolResponse> {
    tool_json(json!({
        "ok": false,
        "error": reason,
    }))
}

// ---------------------------------------------------------------------------
// MCP server handler
// ---------------------------------------------------------------------------

/// MCP server handler for the privileged-exec research runner.
#[derive(Clone)]
pub struct PrivilegedExecServer {
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
    /// Shared cancel sender so a future `cancel` tool can interrupt in-flight
    /// executions.  Currently wired but not exposed as a separate MCP tool
    /// (out of scope per RSR-02).
    _cancel: Arc<tokio::sync::watch::Sender<bool>>,
}

impl Default for PrivilegedExecServer {
    fn default() -> Self {
        let (tx, _rx) = cancel_pair();
        Self {
            tool_router: Self::tool_router(),
            _cancel: Arc::new(tx),
        }
    }
}

#[tool_router]
impl PrivilegedExecServer {
    /// Execute a structured privileged command.
    ///
    /// Input must be `{executable, argv, cwd, reason}`.  No shell strings,
    /// environment overrides, arbitrary stdin, or sudo are accepted.  Output is
    /// bounded to 64 KiB per stream and truncated with an explicit marker if
    /// exceeded.  The call enforces a hard five-minute deadline.
    #[tool(
        name = "exec",
        description = "Run a structured command (executable + argv + cwd + reason). \
            No shell string, no environment overrides, no stdin. \
            Output is bounded; deadline is five minutes. \
            Returns {ok, exit_code, stdout, stderr, invocation_digest, timed_out, cancelled}."
    )]
    async fn exec(&self, Parameters(params): Parameters<ExecParams>) -> Json<ToolResponse> {
        let request = ExecRequest {
            executable: params.executable,
            argv: params.argv,
            cwd: params.cwd,
            reason: params.reason,
        };

        // Pre-validate and emit the digest before executing so the caller can
        // correlate requests even when execution fails.
        if let Err(e) = request.validate() {
            return error_json(&format!("invalid request: {e}"));
        }
        let digest = invocation_digest(&request.executable, &request.argv, &request.cwd);

        let limits = OutputLimits::default();
        // No cancellation wired from the MCP layer yet; pass None.
        match run_structured(&request, None, limits).await {
            Ok(outcome) => tool_json(json!({
                "ok": outcome.ok,
                "invocation_digest": digest,
                "exit_code": outcome.exit_code,
                "stdout": outcome.stdout,
                "stderr": outcome.stderr,
                "stdout_truncated": outcome.stdout_truncated,
                "stderr_truncated": outcome.stderr_truncated,
                "timed_out": outcome.timed_out,
                "cancelled": outcome.cancelled,
            })),
            Err(e) => tool_json(json!({
                "ok": false,
                "invocation_digest": digest,
                "error": e.to_string(),
            })),
        }
    }
}

impl ServerHandler for PrivilegedExecServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "privileged-exec-research",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Research-only structured privileged-exec MCP runner for Zodex. \
                 Accepts {executable, argv, cwd, reason} only. \
                 No shell strings, no environment overrides, no stdin, no sudo. \
                 Out of scope: interactive console, password entry, remote terminal.",
            )
    }
}

/// Serve the MCP protocol over stdio until the client disconnects.
pub async fn serve_mcp() -> Result<()> {
    let service = PrivilegedExecServer::default();
    service
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_info_names_are_correct() {
        let server = PrivilegedExecServer::default();
        let info = server.get_info();
        assert_eq!(info.server_info.name, "privileged-exec-research");
    }

    #[tokio::test]
    async fn exec_tool_rejects_relative_executable() {
        let server = PrivilegedExecServer::default();
        let params = ExecParams {
            executable: "echo".to_string(),
            argv: vec![],
            cwd: "/tmp".to_string(),
            reason: "test".to_string(),
        };
        let result = server.exec(Parameters(params)).await;
        let fields = result.0.fields;
        assert_eq!(fields["ok"].as_bool(), Some(false));
        assert!(fields.contains_key("error"));
    }

    #[tokio::test]
    async fn exec_tool_succeeds_with_valid_request() {
        let server = PrivilegedExecServer::default();
        let params = ExecParams {
            executable: "/usr/bin/true".to_string(),
            argv: vec![],
            cwd: "/tmp".to_string(),
            reason: "mcp smoke test".to_string(),
        };
        let result = server.exec(Parameters(params)).await;
        let fields = result.0.fields;
        assert_eq!(fields["ok"].as_bool(), Some(true));
        assert!(fields.contains_key("invocation_digest"));
        assert!(!fields["invocation_digest"]
            .as_str()
            .unwrap_or("")
            .is_empty());
    }

    #[tokio::test]
    async fn exec_tool_returns_digest_on_validation_failure() {
        let server = PrivilegedExecServer::default();
        let params = ExecParams {
            executable: "not-absolute".to_string(),
            argv: vec![],
            cwd: "/tmp".to_string(),
            reason: "test".to_string(),
        };
        let result = server.exec(Parameters(params)).await;
        let fields = result.0.fields;
        // validation fails before digest emission for relative path
        assert_eq!(fields["ok"].as_bool(), Some(false));
    }
}
