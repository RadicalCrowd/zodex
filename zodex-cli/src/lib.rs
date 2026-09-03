use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, File, Metadata};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const OUTPUT_SCHEMA_VERSION: u32 = 1;
pub const MAX_INPUT_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Error)]
pub enum InspectionError {
    #[error("input is a symbolic link: {0}")]
    Symlink(PathBuf),
    #[error("input is not a regular file: {0}")]
    NotRegular(PathBuf),
    #[error("input owner does not match current user: {0}")]
    WrongOwner(PathBuf),
    #[error("input grants group/other permissions: {0}")]
    UnsafeMode(PathBuf),
    #[error("input exceeds {MAX_INPUT_BYTES} bytes: {0}")]
    Oversized(PathBuf),
    #[error("I/O error for {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub schema_version: u32,
    pub id: String,
    pub revision: String,
    #[serde(default)]
    pub route: Option<String>,
    #[serde(default)]
    pub persistence: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub id: &'static str,
    pub status: &'static str,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub command: String,
    pub overall: &'static str,
    pub checks: Vec<Check>,
}

fn io_error(path: &Path, source: std::io::Error) -> InspectionError {
    InspectionError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn validate_metadata(path: &Path, metadata: &Metadata) -> Result<(), InspectionError> {
    if metadata.file_type().is_symlink() {
        return Err(InspectionError::Symlink(path.to_path_buf()));
    }
    if !metadata.is_file() {
        return Err(InspectionError::NotRegular(path.to_path_buf()));
    }
    if metadata.len() > MAX_INPUT_BYTES {
        return Err(InspectionError::Oversized(path.to_path_buf()));
    }
    if let Some(uid) = current_uid() {
        if metadata.uid() != uid {
            return Err(InspectionError::WrongOwner(path.to_path_buf()));
        }
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(InspectionError::UnsafeMode(path.to_path_buf()));
    }
    Ok(())
}

fn current_uid() -> Option<u32> {
    fs::metadata("/proc/self")
        .ok()
        .map(|metadata| metadata.uid())
}

pub fn read_protected(path: &Path) -> Result<Vec<u8>, InspectionError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    validate_metadata(path, &metadata)?;
    let file = File::open(path).map_err(|source| io_error(path, source))?;
    let opened = file.metadata().map_err(|source| io_error(path, source))?;
    validate_metadata(path, &opened)?;
    if metadata.dev() != opened.dev() || metadata.ino() != opened.ino() {
        return Err(InspectionError::Io {
            path: path.to_path_buf(),
            source: std::io::Error::other("input changed during inspection"),
        });
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    file.take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| io_error(path, source))?;
    if bytes.len() as u64 > MAX_INPUT_BYTES {
        return Err(InspectionError::Oversized(path.to_path_buf()));
    }
    Ok(bytes)
}

pub fn validate_profile(path: &Path) -> Result<Profile, anyhow::Error> {
    let bytes = read_protected(path)?;
    // The Control Plane persists versioned JSON profiles.  TOML remains
    // accepted for hand-authored profiles so `profile validate` can inspect
    // both the generated default and user-owned files without a format
    // conversion step.
    let profile: Profile = if bytes.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b'{') {
        let native: zodex_control_plane::profile::Profile = serde_json::from_slice(&bytes)?;
        zodex_control_plane::profile::validate(&native)?;
        Profile {
            schema_version: native.schema_version,
            id: native.id,
            revision: native.revision,
            route: Some(native.route),
            persistence: Some(match native.persistence {
                zodex_control_plane::profile::Persistence::Encrypted => "encrypted".into(),
                zodex_control_plane::profile::Persistence::LiveOnly => "live-only".into(),
            }),
        }
    } else {
        toml::from_str(std::str::from_utf8(&bytes)?)?
    };
    anyhow::ensure!(
        profile.schema_version == 1,
        "unsupported profile schema version"
    );
    anyhow::ensure!(!profile.id.trim().is_empty(), "profile id is empty");
    anyhow::ensure!(
        !profile.revision.trim().is_empty(),
        "profile revision is empty"
    );
    if let Some(route) = &profile.route {
        anyhow::ensure!(
            route == "disabled"
                || (!route.to_ascii_lowercase().contains("auto")
                    && !route.to_ascii_lowercase().contains("combo")
                    && !route.to_ascii_lowercase().contains("fallback")
                    && !route.eq_ignore_ascii_case("default")
                    && !route.contains(',')
                    && !route.contains('|')
                    && route.split('/').count() >= 2),
            "unsafe route intent"
        );
    }
    Ok(profile)
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase().replace(['-', '_'], "");
    [
        "secret",
        "token",
        "password",
        "cookie",
        "authorization",
        "apikey",
        "privatekey",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

pub fn redact(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| {
                    let value = if is_sensitive_key(key) {
                        Value::String("[REDACTED]".into())
                    } else {
                        redact(value)
                    };
                    (key.clone(), value)
                })
                .collect::<Map<_, _>>(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(redact).collect()),
        Value::String(text) if looks_sensitive(text) => Value::String("[REDACTED]".into()),
        _ => value.clone(),
    }
}

fn looks_sensitive(text: &str) -> bool {
    text.starts_with("sk-")
        || text.starts_with("Bearer ")
        || text.matches('.').count() == 2 && text.starts_with("eyJ")
        || text.contains("/cap/")
}

pub fn digest_json(value: &Value) -> String {
    let canonical = serde_json::to_vec(value).expect("JSON value is serializable");
    Sha256::digest(canonical)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn inventory(state_dir: &Path) -> Report {
    let paths = [
        ("state-directory", state_dir.to_path_buf()),
        ("profiles", state_dir.join("profiles")),
        ("skills", state_dir.join("registry/skills")),
        ("mcp-registry", state_dir.join("registry/mcp")),
        ("adapters", state_dir.join("registry/adapters")),
        ("routes", state_dir.join("registry/routes")),
        ("panic-marker", state_dir.join("panic-v1.json")),
        ("control-socket", state_dir.join("run/control-v1.sock")),
        ("authority-record", state_dir.join("run/authority-v1.json")),
        ("tmux-socket", state_dir.join("run/tmux-v1.sock")),
        (
            "package-provenance",
            state_dir.join("package-provenance.json"),
        ),
    ];
    let mut checks = paths
        .iter()
        .map(|(id, path)| inspect_inventory_path(id, path))
        .collect::<Vec<_>>();
    checks.push(command_check("tmux", "tmux"));
    checks.push(command_check("keyring-client", "secret-tool"));
    checks.push(match std::env::var_os("ZODEX_NATIVE_CODEX") {
        Some(path) => inspect_executable("native-codex", Path::new(&path)),
        None => Check {
            id: "native-codex",
            status: "UNKNOWN",
            detail: "set ZODEX_NATIVE_CODEX to an owner-supplied absolute path".into(),
        },
    });
    let overall = if checks.iter().any(|check| check.status == "FAIL") {
        "FAIL"
    } else {
        "OK"
    };
    Report {
        schema_version: OUTPUT_SCHEMA_VERSION,
        command: "inventory".into(),
        overall,
        checks,
    }
}

pub fn catalog_preservation_check(path: &Path) -> Result<Check, anyhow::Error> {
    let bytes = read_protected(path)?;
    let value: Value = serde_json::from_slice(&bytes)?;
    let models = value
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("catalog models must be an array"))?;
    let mut opencode = 0usize;
    let mut kilo = 0usize;
    for model in models {
        let slug = model
            .get("slug")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let description = model
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if slug.contains("/opencode/") {
            anyhow::ensure!(
                !description.trim().is_empty(),
                "OpenCode model description missing"
            );
            opencode += 1;
        }
        if slug.contains("/kilo/") {
            anyhow::ensure!(
                !description.trim().is_empty(),
                "Kilo model description missing"
            );
            kilo += 1;
        }
    }
    anyhow::ensure!(opencode > 0, "no OpenCode selections found");
    anyhow::ensure!(kilo > 0, "no Kilo selections found");
    Ok(Check {
        id: "catalog-preservation",
        status: "OK",
        detail: format!("OpenCode={opencode}; Kilo={kilo}; descriptions present"),
    })
}

fn inspect_inventory_path(id: &'static str, path: &Path) -> Check {
    let (status, detail) = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            ("FAIL", "symbolic link refused".into())
        }
        Ok(metadata) => {
            let wrong_owner = current_uid().is_some_and(|uid| metadata.uid() != uid);
            if wrong_owner {
                ("FAIL", "wrong owner".into())
            } else if metadata.permissions().mode() & 0o077 != 0 {
                ("FAIL", "group/other permissions refused".into())
            } else {
                ("OK", "present with owner-only permissions".into())
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            ("UNKNOWN", "not present".into())
        }
        Err(error) => ("UNKNOWN", format!("metadata unavailable: {}", error.kind())),
    };
    Check { id, status, detail }
}

fn command_check(id: &'static str, command: &str) -> Check {
    match find_command(command) {
        Some(_) => Check {
            id,
            status: "OK",
            detail: "available".into(),
        },
        None => Check {
            id,
            status: "FAIL",
            detail: "required command unavailable".into(),
        },
    }
}

fn find_command(command: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|directory| directory.join(command))
            .find(|candidate| {
                candidate.is_file()
                    && fs::metadata(candidate)
                        .ok()
                        .is_some_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
            })
    })
}

fn inspect_executable(id: &'static str, path: &Path) -> Check {
    if !path.is_absolute() {
        return Check {
            id,
            status: "FAIL",
            detail: "configured path is not absolute".into(),
        };
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Check {
            id,
            status: "FAIL",
            detail: "configured path is a symlink".into(),
        },
        Ok(metadata) if metadata.is_file() && metadata.permissions().mode() & 0o111 != 0 => Check {
            id,
            status: "OK",
            detail: "owner-supplied executable is present".into(),
        },
        Ok(_) => Check {
            id,
            status: "FAIL",
            detail: "configured path is not executable".into(),
        },
        Err(error) => Check {
            id,
            status: "FAIL",
            detail: format!("configured path unavailable: {}", error.kind()),
        },
    }
}

pub fn support_bundle(report: &Report) -> Value {
    let value = serde_json::to_value(report).expect("report is serializable");
    let redacted = redact(&value);
    json!({
        "schemaVersion": OUTPUT_SCHEMA_VERSION,
        "kind": "zodex-support-bundle",
        "redacted": true,
        "reportDigest": digest_json(&redacted),
        "report": redacted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use tempfile::tempdir;

    #[test]
    fn protected_reader_rejects_symlink_mode_and_size() {
        let root = tempdir().unwrap();
        let good = root.path().join("good.toml");
        fs::write(&good, "schema_version=1\nid='x'\nrevision='1'\n").unwrap();
        fs::set_permissions(&good, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(read_protected(&good).is_ok());
        let link = root.path().join("link");
        symlink(&good, &link).unwrap();
        assert!(matches!(
            read_protected(&link),
            Err(InspectionError::Symlink(_))
        ));
        fs::set_permissions(&good, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            read_protected(&good),
            Err(InspectionError::UnsafeMode(_))
        ));
        let large = root.path().join("large");
        fs::write(&large, vec![0; MAX_INPUT_BYTES as usize + 1]).unwrap();
        fs::set_permissions(&large, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(matches!(
            read_protected(&large),
            Err(InspectionError::Oversized(_))
        ));
    }

    #[test]
    fn profile_is_strict_and_rejects_fallback_routes() {
        let root = tempdir().unwrap();
        let profile = root.path().join("profile.toml");
        fs::write(
            &profile,
            "schema_version=1\nid='default'\nrevision='1'\nroute='auto'\n",
        )
        .unwrap();
        fs::set_permissions(&profile, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(validate_profile(&profile)
            .unwrap_err()
            .to_string()
            .contains("unsafe route"));
        fs::write(
            &profile,
            "schema_version=1\nid='default'\nrevision='1'\nunknown=true\n",
        )
        .unwrap();
        assert!(validate_profile(&profile).is_err());
    }

    #[test]
    fn generated_control_plane_json_profile_is_validated() {
        let root = tempdir().unwrap();
        let profile = root.path().join("default.json");
        fs::write(
            &profile,
            r#"{"schema_version":1,"id":"default","revision":"1","route":"disabled","skills":[],"mcp_grants":[],"persistence":"encrypted"}"#,
        )
        .unwrap();
        fs::set_permissions(&profile, fs::Permissions::from_mode(0o600)).unwrap();
        let validated = validate_profile(&profile).unwrap();
        assert_eq!(validated.id, "default");
        assert_eq!(validated.route.as_deref(), Some("disabled"));
    }

    #[test]
    fn redaction_is_recursive_and_deterministic() {
        let input = json!({"token":"abc", "nested":{"value":"Bearer abc"}, "safe":"ok"});
        let once = redact(&input);
        assert_eq!(once["token"], "[REDACTED]");
        assert_eq!(once["nested"]["value"], "[REDACTED]");
        assert_eq!(digest_json(&once), digest_json(&redact(&input)));
    }

    #[test]
    fn inventory_does_not_create_state() {
        let root = tempdir().unwrap();
        let missing = root.path().join("missing");
        let report = inventory(&missing);
        assert_eq!(report.overall, "OK");
        assert!(!missing.exists());
    }

    #[test]
    fn inventory_refuses_symlinked_and_open_state() {
        let root = tempdir().unwrap();
        let state = root.path().join("state");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(inventory(&state).overall, "FAIL");
        fs::remove_dir(&state).unwrap();
        symlink(root.path(), &state).unwrap();
        assert_eq!(inventory(&state).overall, "FAIL");
    }

    #[test]
    fn catalog_check_requires_both_preserved_groups_and_descriptions() {
        let root = tempdir().unwrap();
        let catalog = root.path().join("models.json");
        fs::write(
            &catalog,
            r#"{"models":[{"slug":"router/opencode/a","description":"OpenCode Free"},{"slug":"router/kilo/b","description":"Kilo Free"}]}"#,
        )
        .unwrap();
        fs::set_permissions(&catalog, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(catalog_preservation_check(&catalog).unwrap().status, "OK");
        fs::write(
            &catalog,
            r#"{"models":[{"slug":"router/opencode/a","description":""}]}"#,
        )
        .unwrap();
        assert!(catalog_preservation_check(&catalog).is_err());
    }
}
