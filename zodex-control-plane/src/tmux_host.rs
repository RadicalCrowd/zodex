use crate::state::StatePaths;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

pub const SESSION_NAME: &str = "zodex-control-v1";

pub fn tmux_socket(paths: &StatePaths) -> PathBuf {
    paths.run.join("tmux-v1.sock")
}

pub fn launch(paths: &StatePaths) -> anyhow::Result<()> {
    let socket = tmux_socket(paths);
    anyhow::ensure!(
        !socket.exists(),
        "tmux authority socket already exists; refusing adoption"
    );
    anyhow::ensure!(
        !paths.socket.exists() && !paths.authority_lock.exists(),
        "control authority state already exists"
    );
    let executable = std::env::current_exe()?;
    let status = Command::new("tmux")
        .args([
            "-S",
            socket
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("non-UTF8 tmux socket"))?,
            "new-session",
            "-d",
            "-s",
            SESSION_NAME,
            "--",
        ])
        .arg(&executable)
        .arg("--state-dir")
        .arg(&paths.root)
        .arg("serve")
        .status()?;
    anyhow::ensure!(status.success(), "tmux refused control-plane launch");
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if paths.socket.exists() && paths.authority_lock.exists() {
            let metadata = fs::metadata(&paths.socket)?;
            anyhow::ensure!(
                metadata.permissions().mode() & 0o077 == 0,
                "control socket is not private"
            );
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let _ = Command::new("tmux")
        .args([
            "-S",
            socket.to_str().unwrap_or_default(),
            "kill-session",
            "-t",
            SESSION_NAME,
        ])
        .status();
    anyhow::bail!("control plane did not become ready")
}

pub fn observe(paths: &StatePaths) -> anyhow::Result<bool> {
    let socket = tmux_socket(paths);
    if !socket.exists() {
        return Ok(false);
    }
    let status = Command::new("tmux")
        .args([
            "-S",
            socket
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("non-UTF8 tmux socket"))?,
            "has-session",
            "-t",
            SESSION_NAME,
        ])
        .status()?;
    Ok(status.success() && paths.socket.exists() && paths.authority_lock.exists())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn observation_does_not_create_or_adopt_state() {
        let root = tempdir().unwrap();
        let paths = StatePaths::new(root.path().join("state"));
        paths.initialize().unwrap();
        assert!(!observe(&paths).unwrap());
        fs::write(tmux_socket(&paths), b"foreign").unwrap();
        assert!(!observe(&paths).unwrap());
        assert!(launch(&paths).is_err());
    }
}
