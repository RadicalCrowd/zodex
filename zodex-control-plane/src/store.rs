use crate::state::{atomic_private_write, StatePaths};
use crate::STORE_SCHEMA_VERSION;
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use zeroize::Zeroizing;

pub trait Keyring {
    fn load(&self) -> anyhow::Result<Option<Zeroizing<Vec<u8>>>>;
    fn store(&self, key: &[u8]) -> anyhow::Result<()>;
}

pub struct SecretToolKeyring;

impl Keyring for SecretToolKeyring {
    fn load(&self) -> anyhow::Result<Option<Zeroizing<Vec<u8>>>> {
        let mut command = secret_tool_command();
        let output = command
            .args([
                "lookup",
                "service",
                "zodex",
                "account",
                "installation-key-v1",
            ])
            .output()?;
        if !output.status.success() {
            return Ok(None);
        }
        let encoded = String::from_utf8(output.stdout)?;
        Ok(Some(Zeroizing::new(BASE64.decode(encoded.trim())?)))
    }

    fn store(&self, key: &[u8]) -> anyhow::Result<()> {
        let mut command = secret_tool_command();
        let mut child = command
            .args([
                "store",
                "--label=Zodex installation key",
                "service",
                "zodex",
                "account",
                "installation-key-v1",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("missing keyring stdin"))?
            .write_all(BASE64.encode(key).as_bytes())?;
        let output = child.wait_with_output()?;
        anyhow::ensure!(
            output.status.success(),
            "OS keyring refused installation key"
        );
        Ok(())
    }
}

fn secret_tool_command() -> Command {
    let mut command = Command::new("secret-tool");
    command.env_clear().env("PATH", "/usr/bin:/bin");
    for name in ["DBUS_SESSION_BUS_ADDRESS", "XDG_RUNTIME_DIR"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub schema_version: u32,
    pub algorithm: String,
    pub nonce: String,
    pub aad: String,
    pub ciphertext: String,
}

pub fn create_installation_key(keyring: &dyn Keyring) -> anyhow::Result<()> {
    anyhow::ensure!(keyring.load()?.is_none(), "installation key already exists");
    let mut key = Zeroizing::new(vec![0u8; 32]);
    fill_random(&mut key)?;
    keyring.store(&key)?;
    anyhow::ensure!(
        keyring
            .load()?
            .is_some_and(|loaded| loaded.as_slice() == key.as_slice()),
        "keyring readback mismatch"
    );
    Ok(())
}

pub fn encrypt(
    keyring: &dyn Keyring,
    record: &[u8],
    metadata: &BTreeMap<String, String>,
) -> anyhow::Result<Envelope> {
    let key = keyring
        .load()?
        .ok_or_else(|| anyhow::anyhow!("keyring unavailable; plaintext fallback forbidden"))?;
    anyhow::ensure!(key.len() == 32, "invalid installation key");
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|_| anyhow::anyhow!("invalid installation key"))?;
    let mut nonce = [0u8; 12];
    fill_random(&mut nonce)?;
    let aad = serde_json::to_vec(metadata)?;
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: record,
                aad: &aad,
            },
        )
        .map_err(|_| anyhow::anyhow!("encryption failed"))?;
    Ok(Envelope {
        schema_version: STORE_SCHEMA_VERSION,
        algorithm: "AES-256-GCM".into(),
        nonce: BASE64.encode(nonce),
        aad: BASE64.encode(aad),
        ciphertext: BASE64.encode(ciphertext),
    })
}

pub fn decrypt(keyring: &dyn Keyring, envelope: &Envelope) -> anyhow::Result<Zeroizing<Vec<u8>>> {
    anyhow::ensure!(
        envelope.schema_version == STORE_SCHEMA_VERSION && envelope.algorithm == "AES-256-GCM",
        "unsupported store migration"
    );
    let key = keyring
        .load()?
        .ok_or_else(|| anyhow::anyhow!("keyring unavailable; plaintext fallback forbidden"))?;
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|_| anyhow::anyhow!("invalid installation key"))?;
    let nonce = BASE64.decode(&envelope.nonce)?;
    anyhow::ensure!(nonce.len() == 12, "invalid nonce");
    let aad = BASE64.decode(&envelope.aad)?;
    let ciphertext = BASE64.decode(&envelope.ciphertext)?;
    Ok(Zeroizing::new(
        cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| anyhow::anyhow!("store authentication failed"))?,
    ))
}

pub fn persist(paths: &StatePaths, id: &str, envelope: &Envelope) -> anyhow::Result<()> {
    anyhow::ensure!(
        id.bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "invalid record id"
    );
    atomic_private_write(
        &paths.store.join(format!("{id}.json.enc")),
        &serde_json::to_vec(envelope)?,
    )
}

fn fill_random(bytes: &mut [u8]) -> anyhow::Result<()> {
    fs::File::open("/dev/urandom")?.read_exact(bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::tempdir;

    #[derive(Default)]
    struct MemoryKeyring {
        key: Mutex<Option<Vec<u8>>>,
        locked: Mutex<bool>,
    }
    impl Keyring for MemoryKeyring {
        fn load(&self) -> anyhow::Result<Option<Zeroizing<Vec<u8>>>> {
            anyhow::ensure!(!*self.locked.lock().unwrap(), "keyring locked");
            Ok(self.key.lock().unwrap().clone().map(Zeroizing::new))
        }
        fn store(&self, key: &[u8]) -> anyhow::Result<()> {
            anyhow::ensure!(!*self.locked.lock().unwrap(), "keyring locked");
            *self.key.lock().unwrap() = Some(key.to_vec());
            Ok(())
        }
    }

    #[test]
    fn encrypted_store_fails_closed_and_has_no_plaintext() {
        let keyring = MemoryKeyring::default();
        assert!(encrypt(&keyring, b"sentinel", &BTreeMap::new()).is_err());
        create_installation_key(&keyring).unwrap();
        let metadata = BTreeMap::from([("workspace".into(), "one".into())]);
        let envelope = encrypt(&keyring, b"SYNTHETIC_SECRET_SENTINEL", &metadata).unwrap();
        assert!(!serde_json::to_string(&envelope)
            .unwrap()
            .contains("SYNTHETIC_SECRET_SENTINEL"));
        assert_eq!(
            decrypt(&keyring, &envelope).unwrap().as_slice(),
            b"SYNTHETIC_SECRET_SENTINEL"
        );
        let mut tampered = envelope.clone();
        tampered.aad = BASE64.encode(b"foreign");
        assert!(decrypt(&keyring, &tampered).is_err());
        *keyring.locked.lock().unwrap() = true;
        assert!(decrypt(&keyring, &envelope).is_err());
    }

    #[test]
    fn persisted_envelope_is_private() {
        let root = tempdir().unwrap();
        let paths = StatePaths::new(root.path().join("state"));
        paths.initialize().unwrap();
        let keyring = MemoryKeyring::default();
        create_installation_key(&keyring).unwrap();
        let envelope = encrypt(&keyring, b"content", &BTreeMap::new()).unwrap();
        persist(&paths, "thread-1", &envelope).unwrap();
        let path = paths.store.join("thread-1.json.enc");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o077, 0);
    }
}
