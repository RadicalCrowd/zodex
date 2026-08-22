//! Canonical invocation digest: SHA-256 over a deterministic JSON representation
//! of the structured request fields that affect execution identity.
//!
//! The digest covers `{executable, argv, cwd, reason}` in sorted-key JSON so
//! the exact action and the human-readable justification shown for approval
//! cannot be changed independently.

use sha2::{Digest, Sha256};

/// Produce a lowercase hex SHA-256 digest for the given structured fields.
///
/// Inputs must already be validated (non-empty executable, absolute cwd).
/// The digest is computed over the canonical, sorted-key JSON object:
/// `{"argv":[...],"cwd":"...","executable":"...","reason":"..."}`.
pub fn invocation_digest(
    executable: &str,
    argv: &[String],
    cwd: &str,
    reason: &str,
) -> anyhow::Result<String> {
    // Build sorted-key JSON manually so serde_json field ordering matches
    // the alphabetically-sorted contract documented above.
    let argv_json: Vec<serde_json::Value> = argv
        .iter()
        .map(|s| serde_json::Value::String(s.clone()))
        .collect();
    let canonical = serde_json::json!({
        "argv": argv_json,
        "cwd": cwd,
        "executable": executable,
        "reason": reason,
    });
    // serde_json serializes object keys in insertion order for Value::Object
    // built from json!(), which uses IndexMap preserving sort.  To guarantee
    // determinism regardless of the internal map, we sort manually here.
    let sorted = sorted_value(canonical);
    let serialized = serde_json::to_string(&sorted)?;
    let hash = Sha256::digest(serialized.as_bytes());
    Ok(hex::encode(hash))
}

/// Recursively sort object keys so serialization is deterministic.
fn sorted_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<String> = map.keys().cloned().collect();
            keys.sort();
            let sorted_map: serde_json::Map<String, serde_json::Value> = keys
                .into_iter()
                .map(|k| {
                    let v = map[&k].clone();
                    (k, sorted_value(v))
                })
                .collect();
            serde_json::Value::Object(sorted_map)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.into_iter().map(sorted_value).collect())
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_deterministic() {
        let d1 = invocation_digest("/usr/bin/id", &[], "/tmp", "test").unwrap();
        let d2 = invocation_digest("/usr/bin/id", &[], "/tmp", "test").unwrap();
        assert_eq!(d1, d2);
    }

    #[test]
    fn digest_is_hex_sha256_length() {
        let d = invocation_digest("/usr/bin/id", &[], "/tmp", "test").unwrap();
        assert_eq!(d.len(), 64);
        assert!(d.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn digest_changes_with_different_executable() {
        let d1 = invocation_digest("/usr/bin/id", &[], "/tmp", "test").unwrap();
        let d2 = invocation_digest("/usr/bin/env", &[], "/tmp", "test").unwrap();
        assert_ne!(d1, d2);
    }

    #[test]
    fn digest_changes_with_different_argv() {
        let d1 = invocation_digest("/usr/bin/ls", &[], "/tmp", "test").unwrap();
        let d2 = invocation_digest("/usr/bin/ls", &["-la".to_string()], "/tmp", "test").unwrap();
        assert_ne!(d1, d2);
    }

    #[test]
    fn digest_changes_with_different_reason() {
        // reason is securely bound into the digest by design
        let d1 = invocation_digest("/usr/bin/id", &[], "/tmp", "test").unwrap();
        let d2 = invocation_digest("/usr/bin/id", &[], "/tmp", "test2").unwrap();
        assert_ne!(d1, d2);
    }

    #[test]
    fn digest_matches_known_value() {
        // Regression anchor: the known digest for `/usr/bin/true [] /tmp`.
        // Recomputed manually: SHA-256 of `{"argv":[],"cwd":"/tmp","executable":"/usr/bin/true","reason":"test"}`
        let d = invocation_digest("/usr/bin/true", &[], "/tmp", "test").unwrap();
        // Re-derive with sha2 inline so the test stays self-describing.
        use sha2::{Digest as _, Sha256};
        let canonical = r#"{"argv":[],"cwd":"/tmp","executable":"/usr/bin/true","reason":"test"}"#;
        let expected = hex::encode(Sha256::digest(canonical.as_bytes()));
        assert_eq!(d, expected);
    }
}
