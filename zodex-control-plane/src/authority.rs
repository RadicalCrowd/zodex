use crate::state::{create_private_new, StatePaths};
use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::fs::MetadataExt;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityRecord {
    pub schema_version: u32,
    pub pid: u32,
    pub uid: u32,
    pub process_start_ticks: u64,
    pub socket: String,
}

pub struct AuthorityGuard {
    paths: StatePaths,
    record: AuthorityRecord,
}

impl AuthorityGuard {
    pub fn acquire(paths: &StatePaths) -> anyhow::Result<Self> {
        if paths.socket.exists() || paths.authority_lock.exists() {
            anyhow::bail!("authority state already exists; foreign/stale authority is not adopted");
        }
        let record = AuthorityRecord {
            schema_version: 1,
            pid: std::process::id(),
            uid: current_uid()?,
            process_start_ticks: process_start_ticks(std::process::id())?,
            socket: paths.socket.display().to_string(),
        };
        // `create_new` is the singleton claim.  A check followed by rename
        // would permit two simultaneous servers to overwrite one another's
        // authority record.
        if let Err(error) = create_private_new(&paths.authority_lock, &serde_json::to_vec(&record)?)
        {
            anyhow::bail!("authority state already exists or cannot be claimed: {error}");
        }
        Ok(Self {
            paths: paths.clone(),
            record,
        })
    }

    pub fn record(&self) -> &AuthorityRecord {
        &self.record
    }
}

impl Drop for AuthorityGuard {
    fn drop(&mut self) {
        if let Ok(bytes) = fs::read(&self.paths.authority_lock) {
            if serde_json::from_slice::<AuthorityRecord>(&bytes).is_ok_and(|found| {
                found.pid == self.record.pid
                    && found.process_start_ticks == self.record.process_start_ticks
            }) {
                let _ = fs::remove_file(&self.paths.authority_lock);
                let _ = fs::remove_file(&self.paths.socket);
            }
        }
    }
}

pub fn current_uid() -> anyhow::Result<u32> {
    Ok(fs::metadata("/proc/self")?.uid())
}

pub fn process_start_ticks(pid: u32) -> anyhow::Result<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let end = stat
        .rfind(')')
        .ok_or_else(|| anyhow::anyhow!("invalid proc stat"))?;
    Ok(stat[end + 2..]
        .split_whitespace()
        .nth(19)
        .ok_or_else(|| anyhow::anyhow!("proc stat has no start time"))?
        .parse()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn duplicate_and_stale_authority_fail_closed() {
        let root = tempdir().unwrap();
        let paths = StatePaths::new(root.path().join("state"));
        paths.initialize().unwrap();
        let guard = AuthorityGuard::acquire(&paths).unwrap();
        assert!(AuthorityGuard::acquire(&paths).is_err());
        drop(guard);
        assert!(!paths.authority_lock.exists());
        crate::state::create_private_new(&paths.authority_lock, b"stale").unwrap();
        assert!(AuthorityGuard::acquire(&paths).is_err());
    }
}
