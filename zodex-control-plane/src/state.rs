use anyhow::Context;
use std::fs;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct StatePaths {
    pub root: PathBuf,
    pub run: PathBuf,
    pub store: PathBuf,
    pub profiles: PathBuf,
    pub snapshots: PathBuf,
    pub agents: PathBuf,
    pub socket: PathBuf,
    pub authority_lock: PathBuf,
    pub panic_marker: PathBuf,
}

impl StatePaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            run: root.join("run"),
            store: root.join("store"),
            profiles: root.join("profiles"),
            snapshots: root.join("snapshots"),
            agents: root.join("agents"),
            socket: root.join("run/control-v1.sock"),
            authority_lock: root.join("run/authority-v1.json"),
            panic_marker: root.join("panic-v1.json"),
            root,
        }
    }

    pub fn initialize(&self) -> anyhow::Result<()> {
        for path in [
            &self.root,
            &self.run,
            &self.store,
            &self.profiles,
            &self.snapshots,
            &self.agents,
        ] {
            ensure_private_directory(path)?;
        }
        Ok(())
    }
}

pub fn ensure_private_directory(path: &Path) -> anyhow::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            anyhow::ensure!(
                !metadata.file_type().is_symlink(),
                "directory is a symlink: {}",
                path.display()
            );
            anyhow::ensure!(metadata.is_dir(), "not a directory: {}", path.display());
            anyhow::ensure!(
                metadata.uid() == fs::metadata("/proc/self")?.uid(),
                "directory has wrong owner: {}",
                path.display()
            );
            anyhow::ensure!(
                metadata.permissions().mode() & 0o077 == 0,
                "directory is not private: {}",
                path.display()
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).with_context(|| format!("create {}", path.display()))?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
        Err(error) => return Err(error).with_context(|| format!("inspect {}", path.display())),
    }
    Ok(())
}

pub fn atomic_private_write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path.parent().context("state file has no parent")?;
    ensure_private_directory(parent)?;
    let temporary = path.with_extension(format!("new-{}", std::process::id()));
    let result = (|| {
        use std::io::Write;
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true).mode(0o600);
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        fs::File::open(parent)?.sync_all()?;
        Ok::<_, anyhow::Error>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Create a private file without replacing an existing path.  This is used for
/// ownership claims (authority locks, panic locks, and one-shot records) where
/// an atomic rename would incorrectly allow a concurrent writer to win.
pub fn create_private_new(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    let parent = path.parent().context("state file has no parent")?;
    ensure_private_directory(parent)?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true).mode(0o600);
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}
