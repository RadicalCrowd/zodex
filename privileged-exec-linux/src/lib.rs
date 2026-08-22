//! Fail-closed structured privileged-exec library for Zodex research.
//!
//! Provides:
//! - Structured input type (`ExecRequest`) with no shell strings, no env overrides.
//! - Canonical invocation digest (SHA-256 over deterministic JSON).
//! - Bounded, redacted output collection.
//! - Owned process-group management and five-minute deadline.
//! - Cancellation via tokio watch channel.
//! - MCP tool server (`mcp` module).

pub mod digest;
pub mod mcp;
pub mod output;
pub mod runner;

pub use digest::invocation_digest;
pub use output::{collect_output, BoundedOutput, OutputLimits, REDACTED_MARKER};
pub use runner::{run_structured, CancelToken, ExecOutcome, ExecRequest};
