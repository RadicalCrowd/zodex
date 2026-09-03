use crate::panic_state;
use crate::state::{atomic_private_write, StatePaths};
use crate::API_MAJOR;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::os::unix::fs::PermissionsExt;

pub const MAX_REQUEST_BYTES: usize = 64 * 1024;
pub const MAX_NONCES_PER_CLIENT: usize = 1024;
pub const MAX_IDEMPOTENCY_ENTRIES: usize = 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClientRecord {
    pub client_id: String,
    pub client_version: String,
    pub capabilities: BTreeSet<String>,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClientRegistry {
    pub schema_version: u32,
    pub clients: BTreeMap<String, ClientRecord>,
}

impl ClientRegistry {
    pub fn load(paths: &StatePaths) -> anyhow::Result<Self> {
        let path = paths.root.join("clients-v1.json");
        match fs::read(path) {
            Ok(bytes) => {
                let registry: Self = serde_json::from_slice(&bytes)?;
                anyhow::ensure!(registry.schema_version == 1, "unsupported client registry");
                Ok(registry)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                schema_version: 1,
                clients: BTreeMap::new(),
            }),
            Err(error) => Err(error.into()),
        }
    }

    pub fn enroll(&mut self, record: ClientRecord) -> anyhow::Result<()> {
        anyhow::ensure!(
            !record.client_id.is_empty() && !record.client_version.is_empty(),
            "invalid client identity"
        );
        anyhow::ensure!(!record.capabilities.is_empty(), "empty capability grant");
        anyhow::ensure!(
            record.capabilities.iter().all(|capability| matches!(
                capability.as_str(),
                "status"
                    | "inventory"
                    | "panic"
                    | "recover"
                    | "spawn"
                    | "archive"
                    | "store"
                    | "apply"
            )),
            "unknown capability"
        );
        anyhow::ensure!(
            !self.clients.contains_key(&record.client_id),
            "client already enrolled"
        );
        self.clients.insert(record.client_id.clone(), record);
        Ok(())
    }

    pub fn save(&self, paths: &StatePaths) -> anyhow::Result<()> {
        atomic_private_write(
            &paths.root.join("clients-v1.json"),
            &serde_json::to_vec(self)?,
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub api_major: u32,
    pub request_id: String,
    pub idempotency_key: String,
    pub client_id: String,
    pub client_version: String,
    pub nonce: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Response {
    pub api_major: u32,
    pub request_id: String,
    pub ok: bool,
    pub result: Value,
    pub error_category: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct AuditRecord<'a> {
    schema_version: u32,
    client_id: &'a str,
    capability: &'a str,
    decision: &'a str,
    request_digest: String,
    result: &'a str,
    error_category: Option<&'a str>,
}

pub struct Engine {
    paths: StatePaths,
    registry: ClientRegistry,
    nonces: BTreeMap<String, VecDeque<String>>,
    cached: BTreeMap<(String, String), (String, Response)>,
}

impl Engine {
    pub fn new(paths: StatePaths, registry: ClientRegistry) -> Self {
        Self {
            paths,
            registry,
            nonces: BTreeMap::new(),
            cached: BTreeMap::new(),
        }
    }

    pub fn handle(&mut self, request: Request) -> Response {
        // Validate authorization before consulting the retry cache. Otherwise a
        // previously successful response could be replayed after a client is
        // disabled or its protocol version changes.
        if let Err(error) = self.validate_envelope(&request) {
            let response = Response {
                api_major: API_MAJOR,
                request_id: request.request_id.clone(),
                ok: false,
                result: Value::Null,
                error_category: Some(error.to_string()),
            };
            let _ = self.audit(&request, &response);
            return response;
        }
        let fingerprint = operation_fingerprint(&request);
        if let Some(response) = self
            .cached
            .get(&(request.client_id.clone(), request.idempotency_key.clone()))
        {
            if response.0 == fingerprint {
                return response.1.clone();
            }
            return Response {
                api_major: API_MAJOR,
                request_id: request.request_id,
                ok: false,
                result: Value::Null,
                error_category: Some("idempotency-key-reused-for-different-operation".into()),
            };
        }
        let outcome = self.validate_and_execute(&request);
        let response = match outcome {
            Ok(result) => Response {
                api_major: API_MAJOR,
                request_id: request.request_id.clone(),
                ok: true,
                result,
                error_category: None,
            },
            Err(error) => Response {
                api_major: API_MAJOR,
                request_id: request.request_id.clone(),
                ok: false,
                result: Value::Null,
                error_category: Some(error.to_string()),
            },
        };
        if self.cached.len() >= MAX_IDEMPOTENCY_ENTRIES {
            if let Some(key) = self.cached.keys().next().cloned() {
                self.cached.remove(&key);
            }
        }
        self.cached.insert(
            (request.client_id.clone(), request.idempotency_key.clone()),
            (fingerprint, response.clone()),
        );
        let _ = self.audit(&request, &response);
        response
    }

    fn validate_envelope(&self, request: &Request) -> anyhow::Result<()> {
        anyhow::ensure!(request.api_major == API_MAJOR, "unsupported-api-version");
        anyhow::ensure!(
            !request.request_id.is_empty()
                && !request.idempotency_key.is_empty()
                && !request.nonce.is_empty(),
            "invalid-envelope"
        );
        let client = self
            .registry
            .clients
            .get(&request.client_id)
            .ok_or_else(|| anyhow::anyhow!("not-enrolled"))?;
        anyhow::ensure!(
            client.enabled && client.client_version == request.client_version,
            "client-disabled-or-version-mismatch"
        );
        Ok(())
    }

    fn validate_and_execute(&mut self, request: &Request) -> anyhow::Result<Value> {
        anyhow::ensure!(request.api_major == API_MAJOR, "unsupported-api-version");
        anyhow::ensure!(
            !request.request_id.is_empty()
                && !request.idempotency_key.is_empty()
                && !request.nonce.is_empty(),
            "invalid-envelope"
        );
        let client = self
            .registry
            .clients
            .get(&request.client_id)
            .ok_or_else(|| anyhow::anyhow!("not-enrolled"))?;
        anyhow::ensure!(
            client.enabled && client.client_version == request.client_version,
            "client-disabled-or-version-mismatch"
        );
        anyhow::ensure!(
            client.capabilities.contains(&request.method),
            "capability-denied"
        );
        let nonces = self.nonces.entry(request.client_id.clone()).or_default();
        anyhow::ensure!(!nonces.contains(&request.nonce), "replayed-nonce");
        nonces.push_back(request.nonce.clone());
        if nonces.len() > MAX_NONCES_PER_CLIENT {
            nonces.pop_front();
        }
        match request.method.as_str() {
            "status" => Ok(
                json!({ "panicked": panic_state::read(&self.paths)?.is_some(), "apiMajor": API_MAJOR }),
            ),
            "inventory" => Ok(json!({
                "schemaVersion": 1,
                "panicked": panic_state::read(&self.paths)?.is_some(),
                "agents": crate::agent::inventory(&self.paths)?,
            })),
            "panic" => {
                let marker = panic_state::engage(&self.paths)?;
                let incomplete = crate::agent::stop_all(&self.paths)?;
                Ok(
                    json!({"schemaVersion":1,"panicked":true,"epoch":marker.epoch,"incomplete":incomplete}),
                )
            }
            "recover" => {
                let epoch = request
                    .params
                    .get("epoch")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| anyhow::anyhow!("missing-epoch"))?;
                anyhow::ensure!(
                    crate::agent::recovery_ready(&self.paths)?,
                    "managed-agents-still-active"
                );
                panic_state::recover(&self.paths, epoch)?;
                Ok(json!({"recovered": true}))
            }
            "spawn" => {
                anyhow::ensure!(panic_state::read(&self.paths)?.is_none(), "panic-active");
                let native = request
                    .params
                    .get("nativeCodex")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing-native-codex"))?;
                let workspace = request
                    .params
                    .get("workspace")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing-workspace"))?;
                let workspace_path = std::path::Path::new(workspace);
                anyhow::ensure!(workspace_path.is_absolute(), "workspace must be absolute");
                let canonical_workspace = workspace_path.canonicalize()?;
                anyhow::ensure!(
                    canonical_workspace.is_dir(),
                    "workspace must be a directory"
                );
                if let Some(profile_id) = request.params.get("profileId").and_then(Value::as_str) {
                    anyhow::ensure!(
                        !profile_id.is_empty()
                            && profile_id.bytes().all(|byte| byte.is_ascii_alphanumeric()
                                || byte == b'-'
                                || byte == b'_'),
                        "invalid profile id"
                    );
                    let profile_path = self.paths.profiles.join(format!("{profile_id}.json"));
                    let metadata = fs::symlink_metadata(&profile_path)?;
                    anyhow::ensure!(
                        metadata.is_file()
                            && !metadata.file_type().is_symlink()
                            && metadata.permissions().mode() & 0o077 == 0,
                        "profile is not a private regular file"
                    );
                    let profile: crate::profile::Profile =
                        serde_json::from_slice(&fs::read(profile_path)?)?;
                    crate::profile::validate(&profile)?;
                    anyhow::ensure!(
                        profile.route == "disabled" || profile.route.contains('/'),
                        "profile route is not exact"
                    );
                }
                let task = request
                    .params
                    .get("syntheticTask")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("live-spawn-requires-separate-confirmation"))?;
                let codex_home = request
                    .params
                    .get("codexHome")
                    .and_then(Value::as_str)
                    .map(std::path::Path::new);
                Ok(serde_json::to_value(crate::agent::launch(
                    &self.paths,
                    std::path::Path::new(native),
                    &canonical_workspace,
                    codex_home,
                    Some(task),
                )?)?)
            }
            "archive" => {
                let agent_id = request
                    .params
                    .get("agentId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing-agent-id"))?;
                Ok(serde_json::to_value(crate::agent::archive(
                    &self.paths,
                    agent_id,
                )?)?)
            }
            _ => anyhow::bail!("method-not-implemented"),
        }
    }

    fn audit(&self, request: &Request, response: &Response) -> anyhow::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let digest = Sha256::digest(serde_json::to_vec(request)?)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let record = AuditRecord {
            schema_version: 1,
            client_id: &request.client_id,
            capability: &request.method,
            decision: if response.ok { "allowed" } else { "denied" },
            request_digest: digest,
            result: if response.ok { "ok" } else { "error" },
            error_category: response.error_category.as_deref(),
        };
        let path = self.paths.root.join("audit-v1.jsonl");
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)?;
        serde_json::to_writer(&mut file, &record)?;
        file.write_all(b"\n")?;
        file.sync_data()?;
        Ok(())
    }
}

fn operation_fingerprint(request: &Request) -> String {
    // Request IDs and nonces are transport-level retry values.  Bind the
    // idempotency key to the actual operation so it cannot be reused for a
    // different method or parameter set.
    let value = json!({
        "api_major": request.api_major,
        "client_id": request.client_id,
        "client_version": request.client_version,
        "method": request.method,
        "params": request.params,
    });
    Sha256::digest(serde_json::to_vec(&value).expect("JSON value is serializable"))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn request(method: &str) -> Request {
        Request {
            api_major: 1,
            request_id: "r1".into(),
            idempotency_key: "i1".into(),
            client_id: "cli".into(),
            client_version: "1".into(),
            nonce: "n1".into(),
            method: method.into(),
            params: Value::Null,
        }
    }

    #[test]
    fn enrollment_capability_nonce_and_idempotency_are_enforced() {
        let root = tempdir().unwrap();
        let paths = StatePaths::new(root.path().join("state"));
        paths.initialize().unwrap();
        let mut registry = ClientRegistry {
            schema_version: 1,
            clients: BTreeMap::new(),
        };
        registry
            .enroll(ClientRecord {
                client_id: "cli".into(),
                client_version: "1".into(),
                capabilities: BTreeSet::from(["status".into()]),
                enabled: true,
            })
            .unwrap();
        let mut engine = Engine::new(paths.clone(), registry);
        let first = engine.handle(request("status"));
        assert!(first.ok);
        let duplicate = engine.handle(request("status"));
        assert_eq!(duplicate.result, first.result);
        let mut reused = request("status");
        reused.params = json!({"unexpected": true});
        assert_eq!(
            engine.handle(reused).error_category.as_deref(),
            Some("idempotency-key-reused-for-different-operation")
        );
        let mut replay = request("status");
        replay.idempotency_key = "i2".into();
        assert_eq!(
            engine.handle(replay).error_category.as_deref(),
            Some("replayed-nonce")
        );
        let mut denied = request("panic");
        denied.idempotency_key = "i3".into();
        denied.nonce = "n3".into();
        assert_eq!(
            engine.handle(denied).error_category.as_deref(),
            Some("capability-denied")
        );
        let audit = fs::read_to_string(paths.root.join("audit-v1.jsonl")).unwrap();
        assert!(!audit.contains("params"));
    }
}
