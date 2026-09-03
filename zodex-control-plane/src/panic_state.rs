use crate::state::{atomic_private_write, create_private_new, StatePaths};
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PanicMarker {
    pub schema_version: u32,
    pub panicked: bool,
    pub epoch: u64,
}

pub fn read(paths: &StatePaths) -> anyhow::Result<Option<PanicMarker>> {
    match fs::read(&paths.panic_marker) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn engage(paths: &StatePaths) -> anyhow::Result<PanicMarker> {
    let lock = paths.panic_marker.with_extension("lock");
    create_private_new(&lock, b"panic-claim\n")
        .map_err(|error| anyhow::anyhow!("panic state is busy or stale: {error}"))?;
    let result = (|| {
        let epoch = read(paths)?.map_or(1, |marker| marker.epoch.saturating_add(1));
        let marker = PanicMarker {
            schema_version: 1,
            panicked: true,
            epoch,
        };
        atomic_private_write(&paths.panic_marker, &serde_json::to_vec(&marker)?)?;
        Ok(marker)
    })();
    let _ = fs::remove_file(lock);
    result
}

pub fn recover(paths: &StatePaths, expected_epoch: u64) -> anyhow::Result<()> {
    let marker = read(paths)?.ok_or_else(|| anyhow::anyhow!("not panicked"))?;
    anyhow::ensure!(
        marker.panicked && marker.epoch == expected_epoch,
        "panic epoch changed"
    );
    fs::remove_file(&paths.panic_marker)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn panic_is_sticky_idempotent_and_epoch_guarded() {
        let root = tempdir().unwrap();
        let paths = StatePaths::new(root.path().join("state"));
        paths.initialize().unwrap();
        let first = engage(&paths).unwrap();
        let second = engage(&paths).unwrap();
        assert!(second.epoch > first.epoch);
        assert!(recover(&paths, first.epoch).is_err());
        recover(&paths, second.epoch).unwrap();
        assert!(read(&paths).unwrap().is_none());
    }
}
