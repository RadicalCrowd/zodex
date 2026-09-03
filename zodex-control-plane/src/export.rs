use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::Argon2;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::Read;
use std::path::Path;
use zeroize::Zeroizing;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PortableArchive {
    pub schema_version: u32,
    pub kdf: String,
    pub cipher: String,
    pub salt: String,
    pub nonce: String,
    pub ciphertext: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExportPayload {
    pub schema_version: u32,
    pub profiles: Vec<Value>,
    pub conversations: Vec<Value>,
    pub audit_metadata: Vec<Value>,
}

pub fn export(payload: &ExportPayload, passphrase: &[u8]) -> anyhow::Result<PortableArchive> {
    anyhow::ensure!(passphrase.len() >= 12, "export passphrase is too short");
    reject_forbidden_fields(&serde_json::to_value(payload)?)?;
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 12];
    fill_random(&mut salt)?;
    fill_random(&mut nonce)?;
    let key = derive_key(passphrase, &salt)?;
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|_| anyhow::anyhow!("invalid export key"))?;
    let plaintext = serde_json::to_vec(payload)?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_slice())
        .map_err(|_| anyhow::anyhow!("archive encryption failed"))?;
    Ok(PortableArchive {
        schema_version: 1,
        kdf: "Argon2id-v19".into(),
        cipher: "AES-256-GCM".into(),
        salt: BASE64.encode(salt),
        nonce: BASE64.encode(nonce),
        ciphertext: BASE64.encode(ciphertext),
    })
}

pub fn import_disabled(
    archive: &PortableArchive,
    passphrase: &[u8],
) -> anyhow::Result<ExportPayload> {
    anyhow::ensure!(
        archive.schema_version == 1
            && archive.kdf == "Argon2id-v19"
            && archive.cipher == "AES-256-GCM",
        "unsupported archive format"
    );
    let salt = BASE64.decode(&archive.salt)?;
    let nonce = BASE64.decode(&archive.nonce)?;
    anyhow::ensure!(
        salt.len() == 16 && nonce.len() == 12,
        "invalid archive metadata"
    );
    let key = derive_key(passphrase, &salt)?;
    let cipher =
        Aes256Gcm::new_from_slice(&key).map_err(|_| anyhow::anyhow!("invalid export key"))?;
    let ciphertext = BASE64.decode(&archive.ciphertext)?;
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(Nonce::from_slice(&nonce), ciphertext.as_slice())
            .map_err(|_| anyhow::anyhow!("archive authentication failed"))?,
    );
    let mut payload: ExportPayload = serde_json::from_slice(&plaintext)?;
    reject_forbidden_fields(&serde_json::to_value(&payload)?)?;
    for profile in &mut payload.profiles {
        if let Some(object) = profile.as_object_mut() {
            object.insert("enabled".into(), Value::Bool(false));
        }
    }
    Ok(payload)
}

pub fn delete_with_tombstone(content: &Path, tombstone: &Path, id: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        id.bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "invalid deletion id"
    );
    if content.exists() {
        fs::remove_file(content)?;
    }
    crate::state::atomic_private_write(
        tombstone,
        &serde_json::to_vec(&serde_json::json!({"schemaVersion":1,"id":id,"deleted":true}))?,
    )
}

fn derive_key(passphrase: &[u8], salt: &[u8]) -> anyhow::Result<Zeroizing<Vec<u8>>> {
    let mut key = Zeroizing::new(vec![0u8; 32]);
    Argon2::default()
        .hash_password_into(passphrase, salt, &mut key)
        .map_err(|error| anyhow::anyhow!("KDF failed: {error}"))?;
    Ok(key)
}

fn reject_forbidden_fields(value: &Value) -> anyhow::Result<()> {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                let normalized = key.to_ascii_lowercase().replace(['-', '_'], "");
                anyhow::ensure!(
                    ![
                        "installationkey",
                        "token",
                        "secret",
                        "cookie",
                        "authorization",
                        "apikey",
                        "privatekey"
                    ]
                    .iter()
                    .any(|forbidden| normalized.contains(forbidden)),
                    "forbidden secret field in export"
                );
                reject_forbidden_fields(value)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                reject_forbidden_fields(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn fill_random(bytes: &mut [u8]) -> anyhow::Result<()> {
    fs::File::open("/dev/urandom")?.read_exact(bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn portable_archive_is_authenticated_excludes_secrets_and_imports_disabled() {
        let payload = ExportPayload {
            schema_version: 1,
            profiles: vec![serde_json::json!({"id":"default","enabled":true})],
            conversations: vec![serde_json::json!({"message":"synthetic"})],
            audit_metadata: vec![],
        };
        let archive = export(&payload, b"correct horse battery staple").unwrap();
        assert!(!serde_json::to_string(&archive)
            .unwrap()
            .contains("synthetic"));
        assert!(import_disabled(&archive, b"wrong passphrase").is_err());
        let imported = import_disabled(&archive, b"correct horse battery staple").unwrap();
        assert_eq!(imported.profiles[0]["enabled"], false);
        let forbidden = ExportPayload {
            profiles: vec![serde_json::json!({"api_key":"bad"})],
            ..payload
        };
        assert!(export(&forbidden, b"correct horse battery staple").is_err());
    }

    #[test]
    fn deletion_removes_content_and_leaves_metadata_only_tombstone() {
        let root = tempdir().unwrap();
        let private = root.path().join("private");
        fs::create_dir(&private).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
        let content = private.join("thread.enc");
        let tombstone = private.join("thread.tombstone.json");
        fs::write(&content, b"ciphertext").unwrap();
        delete_with_tombstone(&content, &tombstone, "thread-1").unwrap();
        assert!(!content.exists());
        let value: Value = serde_json::from_slice(&fs::read(tombstone).unwrap()).unwrap();
        assert_eq!(value["deleted"], true);
        assert!(value.get("message").is_none());
    }
}
