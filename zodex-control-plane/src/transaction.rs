use crate::state::{atomic_private_write, StatePaths};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

fn validate_target(paths: &StatePaths, target: &Path) -> anyhow::Result<()> {
    let root = paths.root.canonicalize()?;
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("transaction target has no parent"))?;
    let canonical_parent = parent.canonicalize()?;
    anyhow::ensure!(
        canonical_parent.starts_with(&root),
        "transaction target is foreign state"
    );
    // Reject symlinked path components and symlink replacement of an existing
    // target.  A lexical `starts_with` check alone can escape through a link.
    let relative_parent = target
        .parent()
        .and_then(|parent| parent.strip_prefix(&paths.root).ok())
        .ok_or_else(|| anyhow::anyhow!("transaction target is foreign state"))?;
    let mut current = paths.root.clone();
    for component in relative_parent.components() {
        current.push(component.as_os_str());
        if let Ok(metadata) = fs::symlink_metadata(&current) {
            anyhow::ensure!(
                !metadata.file_type().is_symlink(),
                "transaction path is a symlink"
            );
        }
    }
    if let Ok(metadata) = fs::symlink_metadata(target) {
        anyhow::ensure!(
            !metadata.file_type().is_symlink(),
            "transaction target is a symlink"
        );
        anyhow::ensure!(
            metadata.is_file(),
            "transaction target is not a regular file"
        );
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub target: PathBuf,
    pub expected_digest: Option<String>,
    pub replacement: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub target: PathBuf,
    pub original_digest: Option<String>,
    pub snapshot_path: PathBuf,
}

pub fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn plan(target: &Path, replacement: Vec<u8>) -> anyhow::Result<Plan> {
    let expected_digest = match fs::read(target) {
        Ok(bytes) => Some(digest(&bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    Ok(Plan {
        target: target.to_path_buf(),
        expected_digest,
        replacement,
    })
}

pub fn apply(paths: &StatePaths, plan: &Plan) -> anyhow::Result<String> {
    validate_target(paths, &plan.target)?;
    let observed = match fs::read(&plan.target) {
        Ok(bytes) => Some(digest(&bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    anyhow::ensure!(
        observed == plan.expected_digest,
        "concurrent edit; compare-and-swap refused"
    );
    atomic_private_write(&plan.target, &plan.replacement)?;
    let verified = digest(&fs::read(&plan.target)?);
    anyhow::ensure!(
        verified == digest(&plan.replacement),
        "post-apply verification failed"
    );
    Ok(verified)
}

pub fn snapshot(paths: &StatePaths, plan: &Plan) -> anyhow::Result<Snapshot> {
    validate_target(paths, &plan.target)?;
    let snapshot_id = digest(plan.target.as_os_str().as_encoded_bytes());
    let snapshot_path = paths.snapshots.join(format!("{snapshot_id}.snapshot"));
    let bytes = match fs::read(&plan.target) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    anyhow::ensure!(
        bytes.is_empty() == plan.expected_digest.is_none()
            || plan.expected_digest.as_deref() == Some(digest(&bytes).as_str()),
        "snapshot no longer matches plan"
    );
    atomic_private_write(&snapshot_path, &bytes)?;
    Ok(Snapshot {
        target: plan.target.clone(),
        original_digest: plan.expected_digest.clone(),
        snapshot_path,
    })
}

pub fn rollback(
    paths: &StatePaths,
    snapshot: &Snapshot,
    expected_applied_digest: &str,
) -> anyhow::Result<()> {
    validate_target(paths, &snapshot.target)?;
    let current = fs::read(&snapshot.target)?;
    anyhow::ensure!(
        digest(&current) == expected_applied_digest,
        "concurrent edit; rollback refused"
    );
    let original = fs::read(&snapshot.snapshot_path)?;
    match &snapshot.original_digest {
        Some(expected) => {
            anyhow::ensure!(digest(&original) == *expected, "snapshot digest mismatch");
            atomic_private_write(&snapshot.target, &original)?;
        }
        None => {
            fs::remove_file(&snapshot.target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn compare_and_swap_refuses_concurrent_and_foreign_edits() {
        let root = tempdir().unwrap();
        let paths = StatePaths::new(root.path().join("state"));
        paths.initialize().unwrap();
        let target = paths.profiles.join("default.json");
        let planned = plan(&target, b"new".to_vec()).unwrap();
        apply(&paths, &planned).unwrap();
        let conflict = plan(&target, b"next".to_vec()).unwrap();
        fs::write(&target, b"owner-edit").unwrap();
        assert!(apply(&paths, &conflict).is_err());
        let foreign = plan(root.path().join("foreign").as_path(), b"x".to_vec()).unwrap();
        assert!(apply(&paths, &foreign).is_err());
    }

    #[test]
    fn rollback_restores_only_when_applied_state_is_unchanged() {
        let root = tempdir().unwrap();
        let paths = StatePaths::new(root.path().join("state"));
        paths.initialize().unwrap();
        let target = paths.profiles.join("default.json");
        atomic_private_write(&target, b"old").unwrap();
        let planned = plan(&target, b"new".to_vec()).unwrap();
        let saved = snapshot(&paths, &planned).unwrap();
        let applied = apply(&paths, &planned).unwrap();
        rollback(&paths, &saved, &applied).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"old");
        let planned = plan(&target, b"new".to_vec()).unwrap();
        let saved = snapshot(&paths, &planned).unwrap();
        let applied = apply(&paths, &planned).unwrap();
        fs::write(&target, b"owner-edit").unwrap();
        assert!(rollback(&paths, &saved, &applied).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"owner-edit");
    }

    #[test]
    fn symlinked_transaction_components_are_refused() {
        use std::os::unix::fs::symlink;
        let root = tempdir().unwrap();
        let paths = StatePaths::new(root.path().join("state"));
        paths.initialize().unwrap();
        let foreign = root.path().join("foreign");
        fs::create_dir(&foreign).unwrap();
        let link = paths.root.join("linked");
        symlink(&foreign, &link).unwrap();
        let target = link.join("file");
        let planned = plan(&target, b"unsafe".to_vec()).unwrap();
        assert!(apply(&paths, &planned).is_err());
    }
}
