use crate::state::StatePaths;
use crate::transaction::{apply, digest, plan, rollback, snapshot, Snapshot};
use serde_json::Value;
use std::fs;
use std::path::Path;

pub fn migrate_json<F>(
    paths: &StatePaths,
    target: &Path,
    expected_version: u64,
    next_version: u64,
    transform: F,
) -> anyhow::Result<Snapshot>
where
    F: FnOnce(Value) -> anyhow::Result<Value>,
{
    let original = fs::read(target)?;
    let value: Value = serde_json::from_slice(&original)?;
    anyhow::ensure!(
        value.get("schemaVersion").and_then(Value::as_u64) == Some(expected_version),
        "unexpected migration source version"
    );
    let mut replacement = transform(value)?;
    replacement["schemaVersion"] = Value::from(next_version);
    let replacement = serde_json::to_vec(&replacement)?;
    let plan = plan(target, replacement)?;
    let snapshot = snapshot(paths, &plan)?;
    match apply(paths, &plan) {
        Ok(applied_digest) => {
            let observed: Value = serde_json::from_slice(&fs::read(target)?)?;
            if observed.get("schemaVersion").and_then(Value::as_u64) != Some(next_version) {
                rollback(paths, &snapshot, &applied_digest)?;
                anyhow::bail!("migration verification failed");
            }
            Ok(snapshot)
        }
        Err(error) => {
            anyhow::ensure!(
                digest(&fs::read(target)?) == digest(&original),
                "migration failure changed original"
            );
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::atomic_private_write;
    use tempfile::tempdir;

    #[test]
    fn migration_preserves_original_and_supports_verified_rollback() {
        let root = tempdir().unwrap();
        let paths = StatePaths::new(root.path().join("state"));
        paths.initialize().unwrap();
        let target = paths.root.join("store-meta.json");
        atomic_private_write(&target, br#"{"schemaVersion":1,"value":"old"}"#).unwrap();
        let before = fs::read(&target).unwrap();
        let failed = migrate_json(&paths, &target, 1, 2, |_| {
            anyhow::bail!("synthetic failure")
        });
        assert!(failed.is_err());
        assert_eq!(fs::read(&target).unwrap(), before);
        let snapshot = migrate_json(&paths, &target, 1, 2, |mut value| {
            value["value"] = Value::from("new");
            Ok(value)
        })
        .unwrap();
        let applied_digest = digest(&fs::read(&target).unwrap());
        rollback(&paths, &snapshot, &applied_digest).unwrap();
        assert_eq!(fs::read(&target).unwrap(), before);
    }
}
