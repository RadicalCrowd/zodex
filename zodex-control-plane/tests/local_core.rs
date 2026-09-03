use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::Command;
use std::sync::Mutex;
use tempfile::tempdir;
use zeroize::Zeroizing;
use zodex_control_plane::state::StatePaths;
use zodex_control_plane::store::Keyring;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_zodex-control-plane")
}

struct MemoryKeyring(Mutex<Option<Vec<u8>>>);

impl Keyring for MemoryKeyring {
    fn load(&self) -> anyhow::Result<Option<Zeroizing<Vec<u8>>>> {
        Ok(self.0.lock().unwrap().clone().map(Zeroizing::new))
    }

    fn store(&self, key: &[u8]) -> anyhow::Result<()> {
        *self.0.lock().unwrap() = Some(key.to_vec());
        Ok(())
    }
}

fn call(paths: &StatePaths, request: Value) -> Value {
    let mut stream = UnixStream::connect(&paths.socket).unwrap();
    writeln!(stream, "{}", request).unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

#[test]
fn tmux_authority_serves_scoped_idempotent_panic_and_recovery() {
    let root = tempdir().unwrap();
    let paths = StatePaths::new(root.path().join("state"));
    let enroll = Command::new(binary())
        .args([
            "--state-dir",
            paths.root.to_str().unwrap(),
            "enroll",
            "--client-id",
            "test-cli",
            "--client-version",
            "1",
            "--capability",
            "status",
            "--capability",
            "panic",
            "--capability",
            "recover",
        ])
        .status()
        .unwrap();
    assert!(enroll.success());
    let launch = Command::new(binary())
        .args(["--state-dir", paths.root.to_str().unwrap(), "launch-tmux"])
        .status()
        .unwrap();
    assert!(launch.success());
    let cleanup_socket = paths.run.join("tmux-v1.sock");

    let status = call(
        &paths,
        json!({"api_major":1,"request_id":"r1","idempotency_key":"i1","client_id":"test-cli","client_version":"1","nonce":"n1","method":"status"}),
    );
    assert_eq!(status["ok"], true);
    let duplicate = call(
        &paths,
        json!({"api_major":1,"request_id":"different","idempotency_key":"i1","client_id":"test-cli","client_version":"1","nonce":"different","method":"status"}),
    );
    assert_eq!(duplicate["request_id"], "r1");
    let panicked = call(
        &paths,
        json!({"api_major":1,"request_id":"r2","idempotency_key":"i2","client_id":"test-cli","client_version":"1","nonce":"n2","method":"panic"}),
    );
    let epoch = panicked["result"]["epoch"].as_u64().unwrap();
    assert!(paths.panic_marker.exists());
    let recovered = call(
        &paths,
        json!({"api_major":1,"request_id":"r3","idempotency_key":"i3","client_id":"test-cli","client_version":"1","nonce":"n3","method":"recover","params":{"epoch":epoch}}),
    );
    assert_eq!(recovered["ok"], true);
    assert!(!paths.panic_marker.exists());

    let duplicate_launch = Command::new(binary())
        .args(["--state-dir", paths.root.to_str().unwrap(), "launch-tmux"])
        .status()
        .unwrap();
    assert!(!duplicate_launch.success());
    let killed = Command::new("tmux")
        .args(["-S", cleanup_socket.to_str().unwrap(), "kill-server"])
        .status()
        .unwrap();
    assert!(killed.success());
}

#[test]
fn managed_native_app_server_is_registered_encrypted_archived_and_stopped() {
    let native_codex = match std::env::var("ZODEX_TEST_NATIVE_CODEX") {
        Ok(path) => fs::canonicalize(path).unwrap(),
        Err(_) => return,
    };
    let root = tempdir().unwrap();
    let paths = StatePaths::new(root.path().join("state"));
    paths.initialize().unwrap();
    let workspace = root.path().join("workspace");
    let codex_home = root.path().join("codex-home");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(&codex_home).unwrap();
    let tmux_socket = paths.run.join("tmux-v1.sock");
    assert!(Command::new("tmux")
        .args([
            "-S",
            tmux_socket.to_str().unwrap(),
            "new-session",
            "-d",
            "-s",
            "zodex-control-v1",
            "sleep",
            "30",
        ])
        .status()
        .unwrap()
        .success());
    let keyring = MemoryKeyring(Mutex::new(Some(vec![7u8; 32])));
    let record = zodex_control_plane::agent::launch_with_keyring(
        &paths,
        std::path::Path::new(binary()),
        &native_codex,
        &workspace,
        Some(&codex_home),
        Some("M4 synthetic managed task"),
        &keyring,
    )
    .unwrap();
    assert_eq!(record.state, "running");
    assert!(record.app_server_socket.exists());
    assert!(paths
        .store
        .join(format!("thread-{}.json.enc", record.agent_id))
        .exists());
    let archived =
        zodex_control_plane::agent::archive_with_keyring(&paths, &record.agent_id, &keyring)
            .unwrap();
    assert_eq!(archived.state, "archived");
    let panic_target = zodex_control_plane::agent::launch_with_keyring(
        &paths,
        std::path::Path::new(binary()),
        &native_codex,
        &workspace,
        Some(&codex_home),
        Some("M4 synthetic panic target"),
        &keyring,
    )
    .unwrap();
    let marker = zodex_control_plane::panic_state::engage(&paths).unwrap();
    let incomplete = zodex_control_plane::agent::stop_all(&paths).unwrap();
    assert!(incomplete.is_empty());
    assert!(!Command::new("tmux")
        .args([
            "-S",
            tmux_socket.to_str().unwrap(),
            "has-session",
            "-t",
            &panic_target.tmux_session,
        ])
        .status()
        .unwrap()
        .success());
    assert!(paths.panic_marker.exists());
    assert!(zodex_control_plane::agent::recovery_ready(&paths).unwrap());
    zodex_control_plane::panic_state::recover(&paths, marker.epoch).unwrap();
    assert!(!paths.panic_marker.exists());
    assert!(Command::new("tmux")
        .args([
            "-S",
            tmux_socket.to_str().unwrap(),
            "has-session",
            "-t",
            "zodex-control-v1",
        ])
        .status()
        .unwrap()
        .success());
    assert!(Command::new("tmux")
        .args(["-S", tmux_socket.to_str().unwrap(), "kill-server"])
        .status()
        .unwrap()
        .success());
}
