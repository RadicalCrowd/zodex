//! Bounded, redacted output collection.
//!
//! Output is collected up to a hard byte ceiling per stream (stdout, stderr).
//! Bytes beyond the ceiling are dropped and a redaction marker is appended so
//! the model always receives a clear signal that output was truncated rather
//! than silently incomplete data.

/// Marker appended when output is truncated at the byte ceiling.
pub const REDACTED_MARKER: &str = "\n[output truncated by privileged-exec runner]\n";

/// Maximum bytes to capture per stream when no explicit limit is provided.
pub const DEFAULT_MAX_BYTES: usize = 64 * 1024; // 64 KiB

/// Limits applied to a single stream.
#[derive(Debug, Clone, Copy)]
pub struct OutputLimits {
    /// Hard ceiling in bytes; bytes beyond this are dropped.
    pub max_bytes: usize,
}

impl Default for OutputLimits {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }
}

/// A collected output stream with truncation status.
#[derive(Debug, Clone, Default)]
pub struct BoundedOutput {
    /// Collected bytes (lossy UTF-8).
    pub text: Vec<u8>,
    /// True if output was truncated at the ceiling.
    pub truncated: bool,
    /// Total bytes received before truncation.
    pub total_bytes: usize,
}

impl BoundedOutput {
    /// Produce the final text: raw collected text, with the redaction marker
    /// appended when truncation occurred.
    pub fn into_display(self) -> String {
        let mut display = String::from_utf8_lossy(&self.text).into_owned();
        if self.truncated {
            display.push_str(REDACTED_MARKER);
        }
        display
    }
}

/// Collect raw bytes up to `limits`, updating `out` in place.
///
/// Call repeatedly as data arrives; returns `true` when the ceiling is reached
/// and further data should be discarded.
pub fn collect_output(buf: &[u8], out: &mut BoundedOutput, limits: OutputLimits) -> bool {
    out.total_bytes += buf.len();
    if out.truncated {
        return true;
    }
    let available = limits.max_bytes.saturating_sub(out.text.len());
    if available == 0 {
        out.truncated = true;
        return true;
    }
    let chunk = if buf.len() > available {
        out.truncated = true;
        &buf[..available]
    } else {
        buf
    };
    out.text.extend_from_slice(chunk);
    out.truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_within_limit_no_truncation() {
        let mut out = BoundedOutput::default();
        let limits = OutputLimits { max_bytes: 1024 };
        let at_limit = collect_output(b"hello", &mut out, limits);
        assert!(!at_limit);
        assert!(!out.truncated);
        assert_eq!(out.text, b"hello");
        assert_eq!(out.total_bytes, 5);
    }

    #[test]
    fn collect_exceeding_limit_truncates() {
        let mut out = BoundedOutput::default();
        let limits = OutputLimits { max_bytes: 4 };
        let at_limit = collect_output(b"hello world", &mut out, limits);
        assert!(at_limit);
        assert!(out.truncated);
        assert_eq!(out.text.len(), 4);
        assert_eq!(out.total_bytes, 11);
    }

    #[test]
    fn collect_second_call_after_truncation_discards() {
        let mut out = BoundedOutput::default();
        let limits = OutputLimits { max_bytes: 4 };
        collect_output(b"XXXX", &mut out, limits);
        // at exact ceiling, not yet truncated
        collect_output(b"Y", &mut out, limits);
        assert!(out.truncated);
        // text stays at 4 bytes
        assert_eq!(out.text.len(), 4);
    }

    #[test]
    fn into_display_appends_marker_when_truncated() {
        let out = BoundedOutput {
            text: b"partial".to_vec(),
            truncated: true,
            total_bytes: 100,
        };
        let display = out.into_display();
        assert!(display.contains(REDACTED_MARKER));
        assert!(display.starts_with("partial"));
    }

    #[test]
    fn into_display_no_marker_when_not_truncated() {
        let out = BoundedOutput {
            text: b"complete".to_vec(),
            truncated: false,
            total_bytes: 8,
        };
        let display = out.into_display();
        assert!(!display.contains(REDACTED_MARKER));
        assert_eq!(display, "complete");
    }
}
