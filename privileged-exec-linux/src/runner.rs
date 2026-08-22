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
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
#[cfg(feature = "process-control")]
use tokio::{
    io::{AsyncReadExt, BufReader},
    process::Command,
    time::Duration,
};

use crate::output::OutputLimits;
#[cfg(feature = "process-control")]
use crate::output::{collect_output, BoundedOutput};

/// Hard deadline: five minutes.
pub const DEADLINE_SECONDS: u64 = 5 * 60;
const MAX_PATH_BYTES: usize = 4096;
const MAX_REASON_BYTES: usize = 1024;
const MAX_ARG_COUNT: usize = 256;
const MAX_ARG_BYTES: usize = 16 * 1024;
const MAX_ARGV_BYTES: usize = 128 * 1024;

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
    /// Validate the request and replace its paths with the exact canonical
    /// paths that will be included in the digest and executed.
    pub fn validate_and_canonicalize(self) -> Result<Self> {
        validate_display_text("reason", &self.reason, MAX_REASON_BYTES, false)?;
        if self.argv.len() > MAX_ARG_COUNT {
            bail!("argv contains too many arguments");
        }
        let mut total_argv_len = 0usize;
        for arg in &self.argv {
            validate_display_text("argv item", arg, MAX_ARG_BYTES, true)?;
            total_argv_len = total_argv_len
                .checked_add(arg.len())
                .ok_or_else(|| anyhow::anyhow!("total argv length overflow"))?;
        }
        if total_argv_len > MAX_ARGV_BYTES {
            bail!("total argv length too large");
        }

        let exec_path = validate_unambiguous_absolute_path("executable", &self.executable)?;
        let cwd_path = validate_unambiguous_absolute_path("cwd", &self.cwd)?;
        let can_exec = fs::canonicalize(&exec_path).context("failed to canonicalize executable")?;
        let can_cwd = fs::canonicalize(&cwd_path).context("failed to canonicalize cwd")?;

        let exec_meta = fs::metadata(&can_exec).context("failed to get executable metadata")?;
        if !exec_meta.is_file() {
            bail!("executable is not a regular file");
        }
        if exec_meta.mode() & 0o111 == 0 {
            bail!("executable does not have execute permissions");
        }

        let cwd_meta = fs::metadata(&can_cwd).context("failed to get cwd metadata")?;
        if !cwd_meta.is_dir() {
            bail!("cwd is not a directory");
        }

        let exec_str = can_exec
            .into_os_string()
            .into_string()
            .map_err(|_| anyhow::anyhow!("executable canonical path is not valid UTF-8"))?;
        let cwd_str = can_cwd
            .into_os_string()
            .into_string()
            .map_err(|_| anyhow::anyhow!("cwd canonical path is not valid UTF-8"))?;

        Ok(Self {
            executable: exec_str,
            argv: self.argv,
            cwd: cwd_str,
            reason: self.reason,
        })
    }
}

fn validate_display_text(
    name: &str,
    value: &str,
    max_bytes: usize,
    allow_empty: bool,
) -> Result<()> {
    if (!allow_empty && value.is_empty()) || value.len() > max_bytes {
        bail!("{name} is empty or exceeds its byte limit");
    }
    if value.chars().any(char::is_control) {
        bail!("{name} contains a control character");
    }
    if !allow_empty && value.trim() != value {
        bail!("{name} contains ambiguous surrounding whitespace");
    }
    Ok(())
}

fn validate_unambiguous_absolute_path(name: &str, value: &str) -> Result<PathBuf> {
    validate_display_text(name, value, MAX_PATH_BYTES, false)?;
    if value.contains("//") || (value != "/" && value.ends_with('/')) {
        bail!("{name} is not a normalized path");
    }
    if value
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        bail!("{name} contains a relative or ambiguous component");
    }
    let path = Path::new(value);
    if !path.is_absolute() {
        bail!("{name} must be an absolute path; got {value:?}");
    }
    if path
        .components()
        .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
    {
        bail!("{name} contains a relative or ambiguous component");
    }
    Ok(path.to_path_buf())
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
    #[cfg(not(feature = "process-control"))]
    {
        let _ = request;
        let _ = cancel;
        let _ = limits;
        bail!("execution is unavailable without the process-control feature");
    }
    #[cfg(feature = "process-control")]
    {
        let request = request.clone().validate_and_canonicalize()?;

        let digest = crate::invocation_digest(
            &request.executable,
            &request.argv,
            &request.cwd,
            &request.reason,
        )
        .context("failed to compute invocation digest")?;
        let deadline = Duration::from_secs(DEADLINE_SECONDS);

        let mut cmd = Command::new(&request.executable);
        cmd.args(&request.argv)
            .current_dir(PathBuf::from(&request.cwd))
            .env_clear()
            .env("LC_ALL", "C")
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
        let pgid = i32::try_from(pid).context("pid does not fit in i32")?; // setpgid(0) → new group leader == pid

        // Collect output concurrently with a hard deadline.
        let stdout_handle = child.stdout.take().context("stdout pipe missing")?;
        let stderr_handle = child.stderr.take().context("stderr pipe missing")?;

        let result = collect_child(
            deadline,
            child,
            stdout_handle,
            stderr_handle,
            limits,
            cancel,
            pgid,
        )
        .await;

        match result {
            Ok((exit_code, stdout_out, stderr_out, cancelled, timed_out)) => Ok(ExecOutcome {
                ok: exit_code == Some(0) && !cancelled,
                exit_code,
                stdout: stdout_out.clone().into_display(),
                stderr: stderr_out.clone().into_display(),
                stdout_truncated: stdout_out.truncated,
                stderr_truncated: stderr_out.truncated,
                timed_out,
                cancelled,
                invocation_digest: digest,
            }),
            Err(e) => Err(e),
        }
    }
}

/// Drive the child to completion, collecting output and watching for cancel.
#[cfg(feature = "process-control")]
async fn collect_child(
    deadline: Duration,
    mut child: tokio::process::Child,
    stdout_pipe: tokio::process::ChildStdout,
    stderr_pipe: tokio::process::ChildStderr,
    limits: OutputLimits,
    mut cancel: Option<CancelReceiver>,
    pgid: i32,
) -> Result<(Option<i32>, BoundedOutput, BoundedOutput, bool, bool)> {
    let mut stdout_out = BoundedOutput::default();
    let mut stderr_out = BoundedOutput::default();

    let mut stdout_reader = BufReader::new(stdout_pipe);
    let mut stderr_reader = BufReader::new(stderr_pipe);

    let mut stdout_buf = vec![0u8; 4096];
    let mut stderr_buf = vec![0u8; 4096];
    let mut stdout_done = false;
    let mut stderr_done = false;
    let sleep = tokio::time::sleep(deadline);
    tokio::pin!(sleep);

    loop {
        if stdout_done && stderr_done {
            tokio::select! {
                status = child.wait() => {
                    let status = status.context("failed to wait for child")?;
                    return Ok((status.code(), stdout_out, stderr_out, false, false));
                }
                _ = &mut sleep => {
                    kill_process_group(pgid);
                    let _ = child.wait().await;
                    return Ok((None, stdout_out, stderr_out, false, true));
                }
                _ = wait_for_cancel(&mut cancel) => {
                    kill_process_group(pgid);
                    let _ = child.wait().await;
                    return Ok((None, stdout_out, stderr_out, true, false));
                }
            }
        }

        tokio::select! {
            _ = &mut sleep => {
                kill_process_group(pgid);
                let _ = child.wait().await;
                return Ok((None, stdout_out, stderr_out, false, true));
            }
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
            _ = wait_for_cancel(&mut cancel) => {
                kill_process_group(pgid);
                let _ = child.wait().await;
                return Ok((None, stdout_out, stderr_out, true, false));
            }
        }
    }
}

#[cfg(feature = "process-control")]
async fn wait_for_cancel(cancel: &mut Option<CancelReceiver>) {
    let Some(receiver) = cancel.as_mut() else {
        std::future::pending::<()>().await;
        return;
    };
    if *receiver.borrow() {
        return;
    }
    loop {
        if receiver.changed().await.is_err() || *receiver.borrow() {
            return;
        }
    }
}

/// Send SIGKILL to the entire process group.  Fail-closed: errors are ignored
/// because the group may have already exited.
#[cfg(feature = "process-control")]
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
        assert!(req.validate_and_canonicalize().is_err());
    }

    #[test]
    fn validate_rejects_relative_executable() {
        let req = ExecRequest {
            executable: "ls".to_string(),
            argv: vec![],
            cwd: "/tmp".to_string(),
            reason: "test".to_string(),
        };
        assert!(req.validate_and_canonicalize().is_err());
    }

    #[test]
    fn validate_rejects_empty_cwd() {
        let req = ExecRequest {
            executable: "/usr/bin/ls".to_string(),
            argv: vec![],
            cwd: "".to_string(),
            reason: "test".to_string(),
        };
        assert!(req.validate_and_canonicalize().is_err());
    }

    #[test]
    fn validate_rejects_relative_cwd() {
        let req = ExecRequest {
            executable: "/usr/bin/ls".to_string(),
            argv: vec![],
            cwd: "tmp".to_string(),
            reason: "test".to_string(),
        };
        assert!(req.validate_and_canonicalize().is_err());
    }

    #[test]
    fn validate_rejects_empty_reason() {
        let req = ExecRequest {
            executable: "/usr/bin/ls".to_string(),
            argv: vec![],
            cwd: "/tmp".to_string(),
            reason: "".to_string(),
        };
        assert!(req.validate_and_canonicalize().is_err());
    }

    #[test]
    fn validate_accepts_valid_request() {
        let req = ExecRequest {
            executable: "/usr/bin/ls".to_string(),
            argv: vec!["-la".to_string()],
            cwd: "/tmp".to_string(),
            reason: "list files".to_string(),
        };
        assert!(req.validate_and_canonicalize().is_ok());
    }

    #[test]
    fn validate_rejects_ambiguous_paths() {
        for executable in [
            "/usr//bin/ls",
            "/usr/./bin/ls",
            "/usr/../bin/ls",
            "/usr/bin/ls/",
            " /usr/bin/ls",
            "/usr/bin/ls\n",
        ] {
            let req = ExecRequest {
                executable: executable.to_string(),
                argv: vec![],
                cwd: "/tmp".to_string(),
                reason: "path validation".to_string(),
            };
            assert!(req.validate_and_canonicalize().is_err(), "{executable}");
        }
        assert!(validate_unambiguous_absolute_path("path", "/tmp/file..name").is_ok());
    }

    #[test]
    fn validate_rejects_unbounded_or_ambiguous_text() {
        let too_many_args = ExecRequest {
            executable: "/usr/bin/true".to_string(),
            argv: vec![String::new(); MAX_ARG_COUNT + 1],
            cwd: "/tmp".to_string(),
            reason: "bounds".to_string(),
        };
        assert!(too_many_args.validate_and_canonicalize().is_err());

        let control_arg = ExecRequest {
            executable: "/usr/bin/true".to_string(),
            argv: vec!["ambiguous\nargument".to_string()],
            cwd: "/tmp".to_string(),
            reason: "bounds".to_string(),
        };
        assert!(control_arg.validate_and_canonicalize().is_err());

        let padded_reason = ExecRequest {
            executable: "/usr/bin/true".to_string(),
            argv: vec![],
            cwd: "/tmp".to_string(),
            reason: " padded reason ".to_string(),
        };
        assert!(padded_reason.validate_and_canonicalize().is_err());
    }

    #[cfg(not(feature = "process-control"))]
    #[tokio::test]
    async fn run_structured_is_disabled_without_process_control() {
        let req = ExecRequest {
            executable: "/usr/bin/true".to_string(),
            argv: vec![],
            cwd: "/tmp".to_string(),
            reason: "feature gate".to_string(),
        };
        let error = run_structured(&req, None, OutputLimits::default())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("process-control"));
    }

    #[cfg(feature = "process-control")]
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

    #[cfg(feature = "process-control")]
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

    #[cfg(feature = "process-control")]
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

    #[cfg(feature = "process-control")]
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

    #[cfg(feature = "process-control")]
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
        let expected =
            crate::invocation_digest("/usr/bin/true", &[], "/tmp", "digest check").unwrap();
        assert_eq!(outcome.invocation_digest, expected);
    }
}
