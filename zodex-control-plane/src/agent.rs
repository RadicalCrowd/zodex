use crate::app_server::AppServerAdapter;
use crate::panic_state;
use crate::state::{atomic_private_write, StatePaths};
use crate::store::{encrypt, persist, Keyring, SecretToolKeyring};
use crate::tmux_host;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRecord {
    pub schema_version: u32,
    pub agent_id: String,
    pub native_thread_id: String,
    pub canonical_workspace: PathBuf,
    pub tmux_session: String,
    pub app_server_socket: PathBuf,
    pub state: String,
    pub synthetic: bool,
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(default)]
    pub profile_revision: Option<String>,
    #[serde(default)]
    pub route_intent: Option<String>,
}

pub fn launch(
    paths: &StatePaths,
    native_codex: &Path,
    workspace: &Path,
    codex_home: Option<&Path>,
    synthetic_task: Option<&str>,
) -> anyhow::Result<AgentRecord> {
    launch_with_keyring(
        paths,
        &std::env::current_exe()?,
        native_codex,
        workspace,
        codex_home,
        synthetic_task,
        &SecretToolKeyring,
    )
}

pub fn launch_with_keyring(
    paths: &StatePaths,
    host_executable: &Path,
    native_codex: &Path,
    workspace: &Path,
    codex_home: Option<&Path>,
    synthetic_task: Option<&str>,
    keyring: &dyn Keyring,
) -> anyhow::Result<AgentRecord> {
    anyhow::ensure!(
        panic_state::read(paths)?.is_none(),
        "panic is active; spawn refused"
    );
    validate_native_codex(native_codex)?;
    let workspace = workspace.canonicalize()?;
    let agent_id = random_id()?;
    let tmux_session = format!("zodex-agent-{agent_id}");
    let app_server_socket = paths.run.join(format!("agent-{agent_id}.sock"));
    let tmux_socket = tmux_host::tmux_socket(paths);
    let mut command = Command::new("tmux");
    command
        .args([
            "-S",
            tmux_socket
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("non-UTF8 tmux socket"))?,
            "new-session",
            "-d",
            "-s",
            &tmux_session,
            "--",
        ])
        .arg(host_executable)
        .arg("--state-dir")
        .arg(&paths.root)
        .arg("agent-host")
        .arg("--native-codex")
        .arg(native_codex)
        .arg("--workspace")
        .arg(&workspace)
        .arg("--socket")
        .arg(&app_server_socket);
    if let Some(codex_home) = codex_home {
        command.arg("--codex-home").arg(codex_home);
    }
    anyhow::ensure!(command.status()?.success(), "tmux refused managed agent");
    let mut guard = SessionGuard {
        tmux_socket: tmux_socket.clone(),
        session: tmux_session.clone(),
        armed: true,
    };
    wait_for_socket(&app_server_socket, Duration::from_secs(5))?;
    let mut adapter = AppServerAdapter::connect(&app_server_socket)?;
    let native_thread_id = adapter.start_thread(&workspace)?;
    if let Some(task) = synthetic_task {
        adapter.inject_synthetic_item(&native_thread_id, task)?;
    }
    let record = AgentRecord {
        schema_version: 1,
        agent_id: agent_id.clone(),
        native_thread_id,
        canonical_workspace: workspace,
        tmux_session,
        app_server_socket,
        state: "running".into(),
        synthetic: synthetic_task.is_some(),
        profile_id: None,
        profile_revision: None,
        route_intent: None,
    };
    atomic_private_write(
        &paths.root.join("agents").join(format!("{agent_id}.json")),
        &serde_json::to_vec(&record)?,
    )?;
    let event = event_record(
        "thread.created",
        &record,
        serde_json::json!({"state":"running"}),
    )?;
    let envelope = encrypt(
        keyring,
        &event,
        &BTreeMap::from([("agentId".into(), record.agent_id.clone())]),
    )?;
    persist(paths, &format!("thread-{}", record.agent_id), &envelope)?;
    guard.armed = false;
    Ok(record)
}

struct SessionGuard {
    tmux_socket: PathBuf,
    session: String,
    armed: bool,
}
impl Drop for SessionGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = Command::new("tmux")
                .args([
                    "-S",
                    self.tmux_socket.to_str().unwrap_or_default(),
                    "kill-session",
                    "-t",
                    &self.session,
                ])
                .status();
        }
    }
}

pub fn agent_host(
    native_codex: &Path,
    workspace: &Path,
    socket: &Path,
    codex_home: Option<&Path>,
) -> anyhow::Result<Child> {
    validate_native_codex(native_codex)?;
    anyhow::ensure!(
        workspace.is_absolute() && socket.is_absolute(),
        "agent host paths must be absolute"
    );
    let mut command = Command::new(native_codex);
    command
        .args([
            "app-server",
            "--listen",
            &format!("unix://{}", socket.display()),
        ])
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(codex_home) = codex_home {
        command.env("CODEX_HOME", codex_home);
    }
    Ok(command.spawn()?)
}

pub fn stop_all(paths: &StatePaths) -> anyhow::Result<Vec<String>> {
    let mut incomplete = Vec::new();
    for entry in fs::read_dir(&paths.agents)? {
        let entry = entry?;
        if entry.path().extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        anyhow::ensure!(
            metadata.is_file()
                && !metadata.file_type().is_symlink()
                && metadata.uid() == fs::metadata("/proc/self")?.uid()
                && metadata.permissions().mode() & 0o077 == 0,
            "agent record is not a private regular file"
        );
        let mut record: AgentRecord = match serde_json::from_slice(&fs::read(entry.path())?) {
            Ok(record) => record,
            Err(error) => {
                incomplete.push(format!("invalid-record:{error}"));
                continue;
            }
        };
        if record.state != "running" {
            continue;
        }
        let status = Command::new("tmux")
            .args([
                "-S",
                tmux_host::tmux_socket(paths).to_str().unwrap_or_default(),
                "kill-session",
                "-t",
                &record.tmux_session,
            ])
            .status();
        if !status.is_ok_and(|status| status.success()) {
            incomplete.push(format!("stop-failed:{}", record.agent_id));
            continue;
        }
        record.state = "stopped".into();
        atomic_private_write(&entry.path(), &serde_json::to_vec(&record)?)?;
        if record.app_server_socket.exists() {
            let _ = fs::remove_file(&record.app_server_socket);
        }
    }
    Ok(incomplete)
}

pub fn recovery_ready(paths: &StatePaths) -> anyhow::Result<bool> {
    for entry in fs::read_dir(&paths.agents)? {
        let entry = entry?;
        if entry.path().extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        anyhow::ensure!(
            metadata.is_file()
                && !metadata.file_type().is_symlink()
                && metadata.uid() == fs::metadata("/proc/self")?.uid()
                && metadata.permissions().mode() & 0o077 == 0,
            "agent record is not a private regular file"
        );
        let record: AgentRecord = serde_json::from_slice(&fs::read(entry.path())?)?;
        if record.state == "running" {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn archive_with_keyring(
    paths: &StatePaths,
    agent_id: &str,
    keyring: &dyn Keyring,
) -> anyhow::Result<AgentRecord> {
    anyhow::ensure!(
        agent_id.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "invalid agent id"
    );
    let record_path = paths.agents.join(format!("{agent_id}.json"));
    let metadata = fs::symlink_metadata(&record_path)?;
    anyhow::ensure!(
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == fs::metadata("/proc/self")?.uid()
            && metadata.permissions().mode() & 0o077 == 0,
        "agent record is not a private regular file"
    );
    let mut record: AgentRecord = serde_json::from_slice(&fs::read(&record_path)?)?;
    anyhow::ensure!(record.agent_id == agent_id, "agent identity mismatch");
    let mut adapter = AppServerAdapter::connect(&record.app_server_socket)?;
    adapter.archive_thread(&record.native_thread_id)?;
    drop(adapter);
    let stopped = Command::new("tmux")
        .args([
            "-S",
            tmux_host::tmux_socket(paths).to_str().unwrap_or_default(),
            "kill-session",
            "-t",
            &record.tmux_session,
        ])
        .status()?;
    anyhow::ensure!(stopped.success(), "failed to stop archived managed agent");
    if record.app_server_socket.exists() {
        let _ = fs::remove_file(&record.app_server_socket);
    }
    record.state = "archived".into();
    atomic_private_write(&record_path, &serde_json::to_vec(&record)?)?;
    let event = event_record("thread.archived", &record, serde_json::json!({}))?;
    let envelope = encrypt(
        keyring,
        &event,
        &BTreeMap::from([("agentId".into(), record.agent_id.clone())]),
    )?;
    persist(paths, &format!("archive-{}", record.agent_id), &envelope)?;
    Ok(record)
}

pub fn archive(paths: &StatePaths, agent_id: &str) -> anyhow::Result<AgentRecord> {
    archive_with_keyring(paths, agent_id, &SecretToolKeyring)
}

pub fn inventory(paths: &StatePaths) -> anyhow::Result<Vec<serde_json::Value>> {
    let mut records = Vec::new();
    for entry in fs::read_dir(&paths.agents)? {
        let entry = entry?;
        if entry.path().extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let record: AgentRecord = serde_json::from_slice(&fs::read(entry.path())?)?;
        records.push(serde_json::json!({
            "agentId": record.agent_id,
            "nativeThreadId": record.native_thread_id,
            "state": record.state,
            "synthetic": record.synthetic,
        }));
    }
    records.sort_by(|left, right| left["agentId"].as_str().cmp(&right["agentId"].as_str()));
    Ok(records)
}

fn validate_native_codex(path: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(path.is_absolute(), "native Codex path must be absolute");
    let metadata = fs::symlink_metadata(path)?;
    anyhow::ensure!(
        !metadata.file_type().is_symlink()
            && metadata.is_file()
            && metadata.uid() == fs::metadata("/proc/self")?.uid()
            && metadata.permissions().mode() & 0o111 != 0,
        "native Codex identity check failed"
    );
    let output = Command::new(path).arg("--version").output()?;
    anyhow::ensure!(
        output.status.success() && String::from_utf8_lossy(&output.stdout).contains("codex-cli"),
        "native Codex capability check failed"
    );
    Ok(())
}

fn random_id() -> anyhow::Result<String> {
    let mut bytes = [0u8; 8];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn wait_for_socket(path: &Path, timeout: Duration) -> anyhow::Result<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.exists() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    anyhow::bail!("managed App Server did not become ready")
}

fn event_record(
    event_type: &str,
    record: &AgentRecord,
    payload: serde_json::Value,
) -> anyhow::Result<Vec<u8>> {
    let payload_bytes = serde_json::to_vec(&payload)?;
    let payload_digest = Sha256::digest(&payload_bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 1,
        "eventId": format!("{}-{event_type}", record.agent_id),
        "threadId": record.native_thread_id,
        "agentId": record.agent_id,
        "sequence": 1,
        "eventType": event_type,
        "sensitivityClass": "metadata",
        "payloadDigest": payload_digest,
        "payload": payload,
    }))?)
}
