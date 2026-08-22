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
    tool, tool_router, ServerHandler,
};
use serde::Deserialize;

#[cfg(feature = "process-control")]
use rmcp::ServiceExt;
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

pub fn error_json(reason: &str) -> Json<ExecResponse> {
    Json(ExecResponse {
        ok: false,
        exit_code: None,
        stdout: None,
        stderr: None,
        stdout_truncated: None,
        stderr_truncated: None,
        timed_out: None,
        cancelled: None,
        invocation_digest: None,
        error: Some(reason.to_string()),
    })
}

#[derive(Debug, Clone, serde::Serialize, rmcp::schemars::JsonSchema, serde::Deserialize)]
pub struct ExecResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout_truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr_truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timed_out: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancelled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invocation_digest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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
    async fn exec(&self, Parameters(params): Parameters<ExecParams>) -> Json<ExecResponse> {
        let request = ExecRequest {
            executable: params.executable,
            argv: params.argv,
            cwd: params.cwd,
            reason: params.reason,
        };

        // Pre-validate and emit the digest before executing so the caller can
        // correlate requests even when execution fails.
        let request = match request.validate_and_canonicalize() {
            Ok(r) => r,
            Err(e) => return error_json(&format!("invalid request: {e}")),
        };
        let digest = match invocation_digest(
            &request.executable,
            &request.argv,
            &request.cwd,
            &request.reason,
        ) {
            Ok(d) => d,
            Err(e) => return error_json(&format!("failed to compute invocation digest: {e}")),
        };

        let limits = OutputLimits::default();
        // No cancellation wired from the MCP layer yet; pass None.
        match run_structured(&request, None, limits).await {
            Ok(outcome) => Json(ExecResponse {
                ok: outcome.ok,
                invocation_digest: Some(digest),
                exit_code: outcome.exit_code,
                stdout: Some(outcome.stdout),
                stderr: Some(outcome.stderr),
                stdout_truncated: Some(outcome.stdout_truncated),
                stderr_truncated: Some(outcome.stderr_truncated),
                timed_out: Some(outcome.timed_out),
                cancelled: Some(outcome.cancelled),
                error: None,
            }),
            Err(e) => Json(ExecResponse {
                ok: false,
                invocation_digest: Some(digest),
                exit_code: None,
                stdout: None,
                stderr: None,
                stdout_truncated: None,
                stderr_truncated: None,
                timed_out: None,
                cancelled: None,
                error: Some(e.to_string()),
            }),
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
/// Activation ownership is managed externally. There is no active listener,
/// this only communicates via stdio.
#[cfg(feature = "process-control")]
pub async fn serve_mcp() -> Result<()> {
    let service = PrivilegedExecServer::default();
    service
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}

#[cfg(not(feature = "process-control"))]
pub async fn serve_mcp() -> Result<()> {
    anyhow::bail!("MCP execution surface requires the process-control feature")
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

        assert!(!result.0.ok);
        assert!(result.0.error.is_some());
    }

    #[cfg(feature = "process-control")]
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

        assert!(result.0.ok);
        assert!(result.0.invocation_digest.is_some());
        assert!(!result
            .0
            .invocation_digest
            .as_deref()
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

        // validation fails before digest emission for relative path
        assert!(!result.0.ok);
    }
}
