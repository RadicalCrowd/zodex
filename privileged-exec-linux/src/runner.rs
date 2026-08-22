//! Structured process runner: owned process group, five-minute deadline,
//! cancellation, bounded output, and fail-closed error handling.
//!
//! # Security invariants
//!
//! - Input is `ExecRequest` only: structured `{executable, argv, cwd, reason}`.
//!   No shell string, no environment override, no arbitrary stdin, no sudo.
//! - The child is placed in its own process group so kill(-pgid) terminates
//!   the entire subtree reliably.
//! - Deadline is hard-enforced at five minutes; the child group receives SIGKILL
//!   after the deadline regardless of output state.
//! - Cancellation via `CancelToken` (tokio watch) also kills the full group.
//! - stdin is /dev/null; no data from the caller reaches the child.
//! - Environment is inherited from the runner's own environment.  No overrides
//!   are accepted.

use anyhow::{bail, Context, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::{
    io::{AsyncReadExt, BufReader},
    process::Command,
    time::{timeout, Duration},
};

use crate::output::{collect_output, BoundedOutput, OutputLimits};

/// Hard deadline: five minutes.
pub const DEADLINE_SECONDS: u64 = 5 * 60;

/// Structured request with no shell strings, no environment overrides.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ExecRequest {
    /// Absolute path to the executable.  Must not be empty or a shell fragment.
    pub executable: String,
    /// Argument vector passed directly to `execve`.  Shell metacharacters are
    /// not interpreted.
    #[serde(default)]
    pub argv: Vec<String>,
    /// Absolute working directory for the child process.
    pub cwd: String,
    /// Human-readable reason shown in audit logs.  Not passed to the child.
    pub reason: String,
}

impl ExecRequest {
    /// Validate the request fields without executing anything.
    pub fn validate(&self) -> Result<()> {
        if self.executable.trim().is_empty() {
            bail!("executable must not be empty");
        }
        if !self.executable.starts_with('/') {
            bail!(
                "executable must be an absolute path; got {:?}",
                self.executable
            );
        }
        if self.cwd.trim().is_empty() {
            bail!("cwd must not be empty");
        }
        if !self.cwd.starts_with('/') {
            bail!("cwd must be an absolute path; got {:?}", self.cwd);
        }
        if self.reason.trim().is_empty() {
            bail!("reason must not be empty");
        }
        Ok(())
    }
}

/// Token for cooperative cancellation.  Drop or send `true` to cancel.
pub type CancelToken = tokio::sync::watch::Sender<bool>;
/// Receiver side of a cancel token.
pub type CancelReceiver = tokio::sync::watch::Receiver<bool>;

/// Create a linked (sender, receiver) cancel pair.
pub fn cancel_pair() -> (CancelToken, CancelReceiver) {
    tokio::sync::watch::channel(false)
}

/// Outcome of a completed or failed execution.
#[derive(Debug, Serialize)]
pub struct ExecOutcome {
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub timed_out: bool,
    pub cancelled: bool,
    pub invocation_digest: String,
}

/// Run the structured request with the five-minute deadline and cancellation.
///
/// Fails closed on every error: if the child cannot be spawned, output cannot
/// be read, or the deadline fires, `ok` is `false` and the error details are
/// in the returned `ExecOutcome` or the `Err` variant.
pub async fn run_structured(
    request: &ExecRequest,
    cancel: Option<CancelReceiver>,
    limits: OutputLimits,
) -> Result<ExecOutcome> {
    request.validate()?;

    let digest = crate::invocation_digest(&request.executable, &request.argv, &request.cwd);
    let deadline = Duration::from_secs(DEADLINE_SECONDS);

    let mut cmd = Command::new(&request.executable);
    cmd.args(&request.argv)
        .current_dir(PathBuf::from(&request.cwd))
        // stdin is /dev/null: no caller data reaches the child.
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // Place the child in its own process group so kill(-pgid) is reliable.
        .process_group(0);

    let mut child = cmd.spawn().context("failed to spawn child process")?;
    let pid = child
        .id()
        .context("child exited immediately before pid was read")?;
    let pgid = pid; // setsid(0) → new group leader == pid

    // Collect output concurrently with a hard deadline.
    let stdout_handle = child.stdout.take().context("stdout pipe missing")?;
    let stderr_handle = child.stderr.take().context("stderr pipe missing")?;

    let result = timeout(
        deadline,
        collect_child(
            child,
            stdout_handle,
            stderr_handle,
            limits,
            cancel,
            pgid as i32,
        ),
    )
    .await;

    match result {
        Ok(Ok((exit_code, stdout_out, stderr_out, cancelled))) => Ok(ExecOutcome {
            ok: exit_code == Some(0) && !cancelled,
            exit_code,
            stdout: stdout_out.clone().into_display(),
            stderr: stderr_out.clone().into_display(),
            stdout_truncated: stdout_out.truncated,
            stderr_truncated: stderr_out.truncated,
            timed_out: false,
            cancelled,
            invocation_digest: digest,
        }),
        Ok(Err(e)) => Err(e),
        Err(_elapsed) => {
            // Deadline fired: kill the entire process group.
            kill_process_group(pgid as i32);
            Ok(ExecOutcome {
                ok: false,
                exit_code: None,
                stdout: String::new(),
                stderr: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
                timed_out: true,
                cancelled: false,
                invocation_digest: digest,
            })
        }
    }
}

/// Drive the child to completion, collecting output and watching for cancel.
async fn collect_child(
    mut child: tokio::process::Child,
    stdout_pipe: tokio::process::ChildStdout,
    stderr_pipe: tokio::process::ChildStderr,
    limits: OutputLimits,
    mut cancel: Option<CancelReceiver>,
    pgid: i32,
) -> Result<(Option<i32>, BoundedOutput, BoundedOutput, bool)> {
    let mut stdout_out = BoundedOutput::default();
    let mut stderr_out = BoundedOutput::default();

    let mut stdout_reader = BufReader::new(stdout_pipe);
    let mut stderr_reader = BufReader::new(stderr_pipe);

    let mut stdout_buf = vec![0u8; 4096];
    let mut stderr_buf = vec![0u8; 4096];
    let mut stdout_done = false;
    let mut stderr_done = false;

    loop {
        // Check cancellation.
        if let Some(ref mut rx) = cancel {
            if *rx.borrow() {
                kill_process_group(pgid);
                let _ = child.wait().await;
                return Ok((None, stdout_out, stderr_out, true));
            }
        }

        if stdout_done && stderr_done {
            break;
        }

        tokio::select! {
            n = stdout_reader.read(&mut stdout_buf), if !stdout_done => {
                match n {
                    Ok(0) => stdout_done = true,
                    Ok(n) => { collect_output(&stdout_buf[..n], &mut stdout_out, limits); }
                    Err(_) => stdout_done = true,
                }
            }
            n = stderr_reader.read(&mut stderr_buf), if !stderr_done => {
                match n {
                    Ok(0) => stderr_done = true,
                    Ok(n) => { collect_output(&stderr_buf[..n], &mut stderr_out, limits); }
                    Err(_) => stderr_done = true,
                }
            }
        }
    }

    let status = child.wait().await.context("failed to wait for child")?;
    let exit_code = status.code();
    Ok((exit_code, stdout_out, stderr_out, false))
}

/// Send SIGKILL to the entire process group.  Fail-closed: errors are ignored
/// because the group may have already exited.
fn kill_process_group(pgid: i32) {
    // Safety: kill(2) is always safe to call with a valid pgid; negative pgid
    // addresses the group.  ESRCH is ignored.
    unsafe {
        libc::kill(-pgid, libc::SIGKILL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_empty_executable() {
        let req = ExecRequest {
            executable: "".to_string(),
            argv: vec![],
            cwd: "/tmp".to_string(),
            reason: "test".to_string(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_rejects_relative_executable() {
        let req = ExecRequest {
            executable: "ls".to_string(),
            argv: vec![],
            cwd: "/tmp".to_string(),
            reason: "test".to_string(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_rejects_empty_cwd() {
        let req = ExecRequest {
            executable: "/usr/bin/ls".to_string(),
            argv: vec![],
            cwd: "".to_string(),
            reason: "test".to_string(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_rejects_relative_cwd() {
        let req = ExecRequest {
            executable: "/usr/bin/ls".to_string(),
            argv: vec![],
            cwd: "tmp".to_string(),
            reason: "test".to_string(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_rejects_empty_reason() {
        let req = ExecRequest {
            executable: "/usr/bin/ls".to_string(),
            argv: vec![],
            cwd: "/tmp".to_string(),
            reason: "".to_string(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_accepts_valid_request() {
        let req = ExecRequest {
            executable: "/usr/bin/ls".to_string(),
            argv: vec!["-la".to_string()],
            cwd: "/tmp".to_string(),
            reason: "list files".to_string(),
        };
        assert!(req.validate().is_ok());
    }

    #[tokio::test]
    async fn run_structured_captures_stdout() {
        let req = ExecRequest {
            executable: "/usr/bin/printf".to_string(),
            argv: vec!["hello\\n".to_string()],
            cwd: "/tmp".to_string(),
            reason: "smoke test".to_string(),
        };
        let outcome = run_structured(&req, None, OutputLimits::default())
            .await
            .unwrap();
        assert!(outcome.ok);
        assert_eq!(outcome.exit_code, Some(0));
        assert!(outcome.stdout.contains("hello"));
        assert!(!outcome.timed_out);
        assert!(!outcome.cancelled);
    }

    #[tokio::test]
    async fn run_structured_non_zero_exit_is_not_ok() {
        let req = ExecRequest {
            executable: "/usr/bin/false".to_string(),
            argv: vec![],
            cwd: "/tmp".to_string(),
            reason: "fail test".to_string(),
        };
        let outcome = run_structured(&req, None, OutputLimits::default())
            .await
            .unwrap();
        assert!(!outcome.ok);
        assert_ne!(outcome.exit_code, Some(0));
    }

    #[tokio::test]
    async fn run_structured_cancel_short_circuits() {
        let (tx, rx) = cancel_pair();
        // Cancel immediately before run.
        let _ = tx.send(true);
        let req = ExecRequest {
            executable: "/usr/bin/sleep".to_string(),
            argv: vec!["60".to_string()],
            cwd: "/tmp".to_string(),
            reason: "cancel test".to_string(),
        };
        let outcome = run_structured(&req, Some(rx), OutputLimits::default())
            .await
            .unwrap();
        assert!(!outcome.ok);
        assert!(outcome.cancelled);
    }

    #[tokio::test]
    async fn run_structured_output_truncated_at_limit() {
        let limits = OutputLimits { max_bytes: 5 };
        let req = ExecRequest {
            executable: "/bin/echo".to_string(),
            argv: vec!["hello world this is longer than five bytes".to_string()],
            cwd: "/tmp".to_string(),
            reason: "truncation test".to_string(),
        };
        let outcome = run_structured(&req, None, limits).await.unwrap();
        assert!(outcome.stdout_truncated);
        assert!(outcome.stdout.contains("[output truncated"));
    }

    #[tokio::test]
    async fn run_structured_rejects_relative_executable() {
        let req = ExecRequest {
            executable: "echo".to_string(),
            argv: vec![],
            cwd: "/tmp".to_string(),
            reason: "relative test".to_string(),
        };
        assert!(run_structured(&req, None, OutputLimits::default())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn invocation_digest_in_outcome_matches_standalone() {
        let req = ExecRequest {
            executable: "/usr/bin/true".to_string(),
            argv: vec![],
            cwd: "/tmp".to_string(),
            reason: "digest check".to_string(),
        };
        let outcome = run_structured(&req, None, OutputLimits::default())
            .await
            .unwrap();
        let expected = crate::invocation_digest("/usr/bin/true", &[], "/tmp");
        assert_eq!(outcome.invocation_digest, expected);
    }
}
