use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub schema_version: u32,
    pub id: String,
    pub revision: String,
    pub route: String,
    #[serde(default)]
    pub skills: BTreeSet<String>,
    #[serde(default)]
    pub mcp_grants: BTreeSet<String>,
    #[serde(default)]
    pub persistence: Persistence,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Persistence {
    #[default]
    Encrypted,
    LiveOnly,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceLock {
    pub schema_version: u32,
    pub workspace: PathBuf,
    pub profile_id: String,
    pub profile_revision: String,
    pub profile_digest: String,
}

pub fn validate(profile: &Profile) -> anyhow::Result<()> {
    anyhow::ensure!(profile.schema_version == 1, "unsupported profile version");
    anyhow::ensure!(
        !profile.id.is_empty() && !profile.revision.is_empty(),
        "empty profile identity"
    );
    anyhow::ensure!(!profile.route.is_empty(), "empty exact route");
    let route = profile.route.to_ascii_lowercase();
    anyhow::ensure!(
        profile.route == "disabled"
            || (route.contains('/')
                && !route.contains("auto")
                && !route.contains("combo")
                && !route.contains("fallback")
                && route != "default"
                && !profile.route.contains(',')
                && !profile.route.contains('|')),
        "fallback/combo/alias route refused"
    );
    Ok(())
}

pub fn bind(profile: &Profile, workspace: &Path) -> anyhow::Result<WorkspaceLock> {
    validate(profile)?;
    let workspace = workspace.canonicalize()?;
    let digest = Sha256::digest(serde_json::to_vec(profile)?);
    Ok(WorkspaceLock {
        schema_version: 1,
        workspace,
        profile_id: profile.id.clone(),
        profile_revision: profile.revision.clone(),
        profile_digest: digest.iter().map(|byte| format!("{byte:02x}")).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn profile(route: &str) -> Profile {
        Profile {
            schema_version: 1,
            id: "default".into(),
            revision: "1".into(),
            route: route.into(),
            skills: BTreeSet::new(),
            mcp_grants: BTreeSet::new(),
            persistence: Persistence::Encrypted,
        }
    }

    #[test]
    fn binding_is_canonical_and_revision_frozen() {
        let root = tempdir().unwrap();
        let lock = bind(&profile("provider/model"), root.path()).unwrap();
        assert_eq!(lock.profile_revision, "1");
        assert_eq!(lock.workspace, root.path().canonicalize().unwrap());
        assert!(validate(&profile("auto")).is_err());
        assert!(validate(&profile("AUTO")).is_err());
        assert!(validate(&profile("a,b")).is_err());
    }
}
