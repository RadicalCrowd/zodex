//! Integration tests for the privileged-exec research runner.
//!
//! These tests exercise the full `run_structured` path including process group
//! ownership, output bounds, cancellation, and deadline semantics.

#[cfg(feature = "process-control")]
use codex_privileged_exec_linux::runner::{cancel_pair, run_structured, ExecRequest};
use codex_privileged_exec_linux::{
    collect_output, invocation_digest,
    output::{BoundedOutput, OutputLimits, REDACTED_MARKER},
    runner::DEADLINE_SECONDS,
};

// ---------------------------------------------------------------------------
// Digest tests
// ---------------------------------------------------------------------------

#[test]
fn digest_is_stable_across_calls() {
    let d1 = invocation_digest(
        "/bin/sh",
        &["-c".to_string(), "echo hi".to_string()],
        "/tmp",
        "reason",
    )
    .unwrap();
    let d2 = invocation_digest(
        "/bin/sh",
        &["-c".to_string(), "echo hi".to_string()],
        "/tmp",
        "reason",
    )
    .unwrap();
    assert_eq!(d1, d2);
}

#[test]
fn digest_differs_on_distinct_argv() {
    let d1 = invocation_digest("/usr/bin/ls", &[], "/home", "test").unwrap();
    let d2 = invocation_digest("/usr/bin/ls", &["-la".to_string()], "/home", "test").unwrap();
    assert_ne!(d1, d2);
}

#[test]
fn digest_differs_on_distinct_cwd() {
    let d1 = invocation_digest("/usr/bin/ls", &[], "/tmp", "test").unwrap();
    let d2 = invocation_digest("/usr/bin/ls", &[], "/var", "test").unwrap();
    assert_ne!(d1, d2);
}

#[test]
fn digest_hex_length_is_64() {
    let d = invocation_digest("/usr/bin/ls", &[], "/tmp", "test").unwrap();
    assert_eq!(d.len(), 64);
}

// ---------------------------------------------------------------------------
// Output bound tests
// ---------------------------------------------------------------------------

#[test]
fn output_collect_no_truncation_under_limit() {
    let mut out = BoundedOutput::default();
    let limits = OutputLimits { max_bytes: 128 };
    collect_output(b"short output", &mut out, limits);
    assert!(!out.truncated);
    assert_eq!(out.text, b"short output");
}

#[test]
fn output_collect_truncation_at_limit() {
    let mut out = BoundedOutput::default();
    let limits = OutputLimits { max_bytes: 3 };
    collect_output(b"longer than three bytes", &mut out, limits);
    assert!(out.truncated);
    assert_eq!(out.text.len(), 3);
}

#[test]
fn output_display_has_redacted_marker_when_truncated() {
    let out = BoundedOutput {
        text: b"xyz".to_vec(),
        truncated: true,
        total_bytes: 100,
    };
    let display = out.into_display();
    assert!(display.contains(REDACTED_MARKER));
}

#[test]
fn output_display_no_marker_when_complete() {
    let out = BoundedOutput {
        text: b"xyz".to_vec(),
        truncated: false,
        total_bytes: 3,
    };
    let display = out.into_display();
    assert!(!display.contains(REDACTED_MARKER));
}

// ---------------------------------------------------------------------------
// Runner integration tests
// ---------------------------------------------------------------------------

#[cfg(feature = "process-control")]
#[tokio::test]
async fn runner_executes_true_and_reports_ok() {
    let req = ExecRequest {
        executable: "/usr/bin/true".to_string(),
        argv: vec![],
        cwd: "/tmp".to_string(),
        reason: "integration smoke test".to_string(),
    };
    let outcome = run_structured(&req, None, OutputLimits::default())
        .await
        .expect("run_structured should not error");
    assert!(outcome.ok);
    assert_eq!(outcome.exit_code, Some(0));
    assert!(!outcome.timed_out);
    assert!(!outcome.cancelled);
}

#[cfg(feature = "process-control")]
#[tokio::test]
async fn runner_captures_stdout_correctly() {
    let req = ExecRequest {
        executable: "/bin/echo".to_string(),
        argv: vec!["integration-marker".to_string()],
        cwd: "/tmp".to_string(),
        reason: "stdout capture test".to_string(),
    };
    let outcome = run_structured(&req, None, OutputLimits::default())
        .await
        .expect("run_structured should not error");
    assert!(outcome.ok);
    assert!(
        outcome.stdout.contains("integration-marker"),
        "stdout: {:?}",
        outcome.stdout
    );
}

#[cfg(feature = "process-control")]
#[tokio::test]
async fn runner_non_zero_exit_is_not_ok() {
    let req = ExecRequest {
        executable: "/usr/bin/false".to_string(),
        argv: vec![],
        cwd: "/tmp".to_string(),
        reason: "non-zero exit test".to_string(),
    };
    let outcome = run_structured(&req, None, OutputLimits::default())
        .await
        .unwrap();
    assert!(!outcome.ok);
    assert_ne!(outcome.exit_code, Some(0));
}

#[cfg(feature = "process-control")]
#[tokio::test]
async fn runner_fails_on_nonexistent_executable() {
    let req = ExecRequest {
        executable: "/this/does/not/exist/at/all".to_string(),
        argv: vec![],
        cwd: "/tmp".to_string(),
        reason: "nonexistent binary test".to_string(),
    };
    let result = run_structured(&req, None, OutputLimits::default()).await;
    // Should return Err because spawn fails
    assert!(result.is_err(), "expected Err for nonexistent binary");
}

#[cfg(feature = "process-control")]
#[tokio::test]
async fn runner_cancellation_stops_long_running_process() {
    let (tx, rx) = cancel_pair();
    // Pre-cancel before spawn
    let _ = tx.send(true);

    let req = ExecRequest {
        executable: "/usr/bin/sleep".to_string(),
        argv: vec!["300".to_string()],
        cwd: "/tmp".to_string(),
        reason: "cancel integration test".to_string(),
    };
    let outcome = run_structured(&req, Some(rx), OutputLimits::default())
        .await
        .unwrap();
    assert!(!outcome.ok);
    assert!(outcome.cancelled);
    assert!(!outcome.timed_out);
}

#[cfg(feature = "process-control")]
#[tokio::test]
async fn runner_output_truncated_with_marker() {
    let limits = OutputLimits { max_bytes: 8 };
    // /bin/echo always produces more than 8 bytes when given a long string.
    let req = ExecRequest {
        executable: "/bin/echo".to_string(),
        argv: vec!["this is more than eight bytes of output".to_string()],
        cwd: "/tmp".to_string(),
        reason: "truncation integration test".to_string(),
    };
    let outcome = run_structured(&req, None, limits).await.unwrap();
    assert!(outcome.stdout_truncated, "stdout should be truncated");
    assert!(
        outcome.stdout.contains(REDACTED_MARKER),
        "stdout should contain redaction marker"
    );
}

#[cfg(feature = "process-control")]
#[tokio::test]
async fn runner_invocation_digest_matches_standalone() {
    let exe = "/usr/bin/true";
    let argv: Vec<String> = vec![];
    let cwd = "/tmp";
    let reason = "digest integration test";
    let req = ExecRequest {
        executable: exe.to_string(),
        argv: argv.clone(),
        cwd: cwd.to_string(),
        reason: reason.to_string(),
    };
    let outcome = run_structured(&req, None, OutputLimits::default())
        .await
        .unwrap();

    // Test requires matching canonicalized form exactly as the runner would prepare it
    let expected = invocation_digest(
        &std::fs::canonicalize(exe)
            .unwrap()
            .into_os_string()
            .into_string()
            .unwrap(),
        &argv,
        &std::fs::canonicalize(cwd)
            .unwrap()
            .into_os_string()
            .into_string()
            .unwrap(),
        reason,
    )
    .unwrap();
    assert_eq!(outcome.invocation_digest, expected);
}

#[cfg(feature = "process-control")]
#[tokio::test]
async fn runner_rejects_shell_string_as_executable() {
    // Shell injection attempt: passing a command string where an absolute path
    // is expected must be rejected by validation before spawn.
    let req = ExecRequest {
        executable: "echo hello; rm -rf /".to_string(),
        argv: vec![],
        cwd: "/tmp".to_string(),
        reason: "shell injection test".to_string(),
    };
    let result = run_structured(&req, None, OutputLimits::default()).await;
    assert!(result.is_err(), "shell string must be rejected");
}

#[test]
fn deadline_constant_is_five_minutes() {
    assert_eq!(DEADLINE_SECONDS, 300);
}
