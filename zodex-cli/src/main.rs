use anyhow::Context;
use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use zodex_cli::{
    catalog_preservation_check, inventory, support_bundle, validate_profile, Check, Report,
    OUTPUT_SCHEMA_VERSION,
};

#[derive(Parser)]
#[command(name = "zodex", version, about = "Zodex local Control Plane client")]
struct Cli {
    #[arg(long, env = "ZODEX_STATE_DIR", default_value = ".zodex")]
    state_dir: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Status,
    Doctor,
    Init {
        #[arg(long)]
        native_codex: PathBuf,
        #[arg(long)]
        control_plane: PathBuf,
    },
    Inventory {
        #[arg(long)]
        catalog: Option<PathBuf>,
    },
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    Panic {
        #[arg(long)]
        confirm: bool,
    },
    Recover {
        #[arg(long)]
        epoch: u64,
        #[arg(long)]
        confirm: bool,
    },
    Run {
        #[arg(long)]
        native_codex: PathBuf,
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long, default_value = "default")]
        profile: String,
        #[arg(long)]
        synthetic_task: Option<String>,
        #[arg(long, hide = true)]
        codex_home: Option<PathBuf>,
    },
    Archive {
        #[arg(long)]
        agent_id: String,
    },
    Export {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Import {
        #[arg(long)]
        archive: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Delete {
        #[arg(long)]
        id: String,
        #[arg(long)]
        content: PathBuf,
    },
    SupportBundle {
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ProfileCommand {
    Validate {
        path: PathBuf,
    },
    Bind {
        path: PathBuf,
        #[arg(long)]
        workspace: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let report = match cli.command {
        Command::Status => status(&cli.state_dir),
        Command::Doctor => doctor(&cli.state_dir),
        Command::Init {
            native_codex,
            control_plane,
        } => {
            initialize(&cli.state_dir, &native_codex, &control_plane)?;
            success("init", "private state, keyring key, inert profile, enrollment, and tmux authority initialized")
        }
        Command::Inventory { catalog } => {
            let mut report = inventory(&cli.state_dir);
            if let Some(path) = catalog {
                match catalog_preservation_check(&path) {
                    Ok(check) => report.checks.push(check),
                    Err(error) => {
                        report.overall = "FAIL";
                        report.checks.push(Check {
                            id: "catalog-preservation",
                            status: "FAIL",
                            detail: error.to_string(),
                        });
                    }
                }
            }
            report
        }
        Command::Profile {
            command: ProfileCommand::Validate { path },
        } => {
            let path = resolve_profile_path(&cli.state_dir, &path);
            let profile = validate_profile(&path)
                .with_context(|| format!("profile validation failed: {}", path.display()))?;
            Report {
                schema_version: OUTPUT_SCHEMA_VERSION,
                command: "profile validate".into(),
                overall: "OK",
                checks: vec![Check {
                    id: "profile",
                    status: "OK",
                    detail: format!("{}@{}", profile.id, profile.revision),
                }],
            }
        }
        Command::Profile {
            command: ProfileCommand::Bind { path, workspace },
        } => {
            let bytes = zodex_cli::read_protected(&path)?;
            let profile: zodex_control_plane::profile::Profile = serde_json::from_slice(&bytes)?;
            let binding = zodex_control_plane::profile::bind(&profile, &workspace)?;
            let paths = zodex_control_plane::state::StatePaths::new(&cli.state_dir);
            paths.initialize()?;
            let target = paths.profiles.join(format!(
                "workspace-{}.json",
                zodex_cli::digest_json(&json!(binding.workspace))
            ));
            let planned =
                zodex_control_plane::transaction::plan(&target, serde_json::to_vec(&binding)?)?;
            let _snapshot = zodex_control_plane::transaction::snapshot(&paths, &planned)?;
            zodex_control_plane::transaction::apply(&paths, &planned)?;
            success(
                "profile bind",
                &format!("bound {}@{}", binding.profile_id, binding.profile_revision),
            )
        }
        Command::Panic { confirm } => {
            anyhow::ensure!(confirm, "panic requires --confirm");
            let response = control_request(&cli.state_dir, "panic", Value::Null)?;
            success(
                "panic",
                &format!("sticky panic engaged at epoch {}", response["epoch"]),
            )
        }
        Command::Recover { epoch, confirm } => {
            anyhow::ensure!(confirm, "recover requires --confirm");
            control_request(&cli.state_dir, "recover", json!({"epoch":epoch}))?;
            success("recover", "panic marker removed after epoch validation")
        }
        Command::Run {
            native_codex,
            workspace,
            profile,
            synthetic_task,
            codex_home,
        } => {
            let task = synthetic_task.context(
                "live model turns require a separate immediate confirmation; use --synthetic-task for quota-free validation",
            )?;
            let mut params = json!({
                "nativeCodex": native_codex,
                "workspace": workspace,
                "profileId": profile,
                "syntheticTask": task,
            });
            if let Some(codex_home) = codex_home {
                params["codexHome"] = json!(codex_home);
            }
            let record = control_request(&cli.state_dir, "spawn", params)?;
            success(
                "run",
                &format!(
                    "managed agent {} started in {}",
                    record["agent_id"], record["tmux_session"]
                ),
            )
        }
        Command::Archive { agent_id } => {
            let record = control_request(&cli.state_dir, "archive", json!({"agentId":agent_id}))?;
            success("archive", &format!("agent {} archived", record["agent_id"]))
        }
        Command::Export { input, output } => {
            let payload: zodex_control_plane::export::ExportPayload =
                serde_json::from_slice(&zodex_cli::read_protected(&input)?)?;
            let passphrase = read_passphrase()?;
            let archive = zodex_control_plane::export::export(&payload, &passphrase)?;
            write_private_new(&output, &serde_json::to_vec(&archive)?)?;
            success(
                "export",
                "authenticated archive written; installation key excluded",
            )
        }
        Command::Import { archive, output } => {
            let archive: zodex_control_plane::export::PortableArchive =
                serde_json::from_slice(&zodex_cli::read_protected(&archive)?)?;
            let passphrase = read_passphrase()?;
            let payload = zodex_control_plane::export::import_disabled(&archive, &passphrase)?;
            write_private_new(&output, &serde_json::to_vec(&payload)?)?;
            success(
                "import",
                "archive authenticated; imported profiles disabled",
            )
        }
        Command::Delete { id, content } => {
            let paths = zodex_control_plane::state::StatePaths::new(&cli.state_dir);
            paths.initialize()?;
            let canonical_parent = content
                .parent()
                .context("content has no parent")?
                .canonicalize()?;
            anyhow::ensure!(
                canonical_parent.starts_with(&paths.root),
                "deletion target is outside Zodex state"
            );
            let tombstone = paths.store.join(format!("{id}.tombstone.json"));
            zodex_control_plane::export::delete_with_tombstone(&content, &tombstone, &id)?;
            success("delete", "content removed; metadata tombstone retained")
        }
        Command::SupportBundle { output } => {
            let bundle = support_bundle(&doctor(&cli.state_dir));
            let rendered = serde_json::to_string_pretty(&bundle)? + "\n";
            if let Some(path) = output {
                anyhow::bail!("M2 is read-only; refusing to write support bundle to {}. Redirect stdout explicitly.", path.display());
            }
            print!("{rendered}");
            return Ok(());
        }
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    if report.overall == "FAIL" {
        std::process::exit(2);
    }
    Ok(())
}

fn success(command: &str, detail: &str) -> Report {
    Report {
        schema_version: OUTPUT_SCHEMA_VERSION,
        command: command.into(),
        overall: "OK",
        checks: vec![Check {
            id: "result",
            status: "OK",
            detail: detail.into(),
        }],
    }
}

fn initialize(
    state_dir: &std::path::Path,
    native_codex: &std::path::Path,
    control_plane: &std::path::Path,
) -> anyhow::Result<()> {
    validate_native_codex(native_codex)?;
    validate_executable(control_plane, "control plane")?;
    let paths = zodex_control_plane::state::StatePaths::new(state_dir);
    paths.initialize()?;
    zodex_control_plane::store::create_installation_key(
        &zodex_control_plane::store::SecretToolKeyring,
    )?;
    let profile = zodex_control_plane::profile::Profile {
        schema_version: 1,
        id: "default".into(),
        revision: "1".into(),
        route: "disabled".into(),
        skills: Default::default(),
        mcp_grants: Default::default(),
        persistence: zodex_control_plane::profile::Persistence::Encrypted,
    };
    zodex_control_plane::state::atomic_private_write(
        &paths.profiles.join("default.json"),
        &serde_json::to_vec(&profile)?,
    )?;
    let status = ProcessCommand::new(control_plane)
        .args([
            "--state-dir",
            state_dir.to_str().context("non-UTF8 state path")?,
            "enroll",
            "--client-id",
            "zodex-cli",
            "--client-version",
            env!("CARGO_PKG_VERSION"),
            "--capability",
            "status",
            "--capability",
            "panic",
            "--capability",
            "recover",
            "--capability",
            "spawn",
            "--capability",
            "archive",
        ])
        .status()?;
    anyhow::ensure!(status.success(), "control-plane enrollment failed");
    let status = ProcessCommand::new(control_plane)
        .args([
            "--state-dir",
            state_dir.to_str().context("non-UTF8 state path")?,
            "enroll",
            "--client-id",
            "zodex-desktop",
            "--client-version",
            "1",
            "--capability",
            "inventory",
            "--capability",
            "status",
        ])
        .status()?;
    anyhow::ensure!(status.success(), "Desktop enrollment failed");
    let status = ProcessCommand::new(control_plane)
        .args([
            "--state-dir",
            state_dir.to_str().context("non-UTF8 state path")?,
            "launch-tmux",
        ])
        .status()?;
    anyhow::ensure!(status.success(), "control-plane tmux launch failed");
    Ok(())
}

fn resolve_profile_path(state_dir: &std::path::Path, requested: &std::path::Path) -> PathBuf {
    if requested.exists() || requested.is_absolute() {
        requested.to_path_buf()
    } else if requested.to_str().is_some_and(|value| {
        value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    }) {
        state_dir
            .join("profiles")
            .join(format!("{}.json", requested.display()))
    } else {
        requested.to_path_buf()
    }
}

fn validate_native_codex(path: &std::path::Path) -> anyhow::Result<()> {
    validate_executable(path, "native Codex")?;
    let output = ProcessCommand::new(path).arg("--version").output()?;
    anyhow::ensure!(
        output.status.success() && String::from_utf8_lossy(&output.stdout).contains("codex-cli"),
        "configured native binary failed capability validation"
    );
    Ok(())
}

fn validate_executable(path: &std::path::Path, label: &str) -> anyhow::Result<()> {
    anyhow::ensure!(path.is_absolute(), "{label} path must be absolute");
    let metadata = fs::symlink_metadata(path)?;
    anyhow::ensure!(
        !metadata.file_type().is_symlink() && metadata.is_file(),
        "{label} must be a regular non-symlink file"
    );
    anyhow::ensure!(
        metadata.uid() == fs::metadata("/proc/self")?.uid()
            && metadata.permissions().mode() & 0o111 != 0,
        "{label} ownership/executable check failed"
    );
    Ok(())
}

fn control_request(
    state_dir: &std::path::Path,
    method: &str,
    params: Value,
) -> anyhow::Result<Value> {
    let paths = zodex_control_plane::state::StatePaths::new(state_dir);
    let mut stream = UnixStream::connect(&paths.socket)?;
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let request = json!({"api_major":1,"request_id":nonce,"idempotency_key":format!("{method}-{nonce}"),"client_id":"zodex-cli","client_version":env!("CARGO_PKG_VERSION"),"nonce":nonce,"method":method,"params":params});
    writeln!(stream, "{request}")?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    let response: Value = serde_json::from_str(&line)?;
    anyhow::ensure!(
        response["ok"] == true,
        "control request failed: {}",
        response["error_category"]
    );
    Ok(response["result"].clone())
}

fn read_passphrase() -> anyhow::Result<Vec<u8>> {
    let mut input = Vec::new();
    std::io::stdin().take(4097).read_to_end(&mut input)?;
    while input
        .last()
        .is_some_and(|byte| matches!(byte, b'\n' | b'\r'))
    {
        input.pop();
    }
    anyhow::ensure!(
        !input.is_empty() && input.len() <= 4096,
        "passphrase must be supplied through stdin"
    );
    Ok(input)
}

fn write_private_new(path: &std::path::Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    anyhow::ensure!(!path.exists(), "refusing to overwrite output");
    let parent = path.parent().context("output has no parent")?;
    let metadata = fs::symlink_metadata(parent)?;
    anyhow::ensure!(
        metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == fs::metadata("/proc/self")?.uid()
            && metadata.permissions().mode() & 0o077 == 0,
        "output directory is not private"
    );
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn status(state_dir: &std::path::Path) -> Report {
    let paths = zodex_control_plane::state::StatePaths::new(state_dir);
    let exists = state_dir.exists();
    let running = zodex_control_plane::tmux_host::observe(&paths).unwrap_or(false);
    let panicked = zodex_control_plane::panic_state::read(&paths)
        .ok()
        .flatten()
        .is_some();
    Report {
        schema_version: OUTPUT_SCHEMA_VERSION,
        command: "status".into(),
        overall: "OK",
        checks: vec![Check {
            id: "control-plane",
            status: if running {
                "OK"
            } else if exists {
                "UNKNOWN"
            } else {
                "INACTIVE"
            },
            detail: if running {
                format!(
                    "tmux Control Plane is running{}",
                    if panicked { "; panic is active" } else { "" }
                )
            } else if exists {
                "state exists; Control Plane is not observed".into()
            } else {
                "no state observed".into()
            },
        }],
    }
}

fn doctor(state_dir: &std::path::Path) -> Report {
    let mut report = inventory(state_dir);
    report.command = "doctor".into();
    report.checks.push(Check {
        id: "native-auth",
        status: "SKIPPED",
        detail: "availability must be verified without inspecting authentication artifacts".into(),
    });
    report.checks.push(Check {
        id: "durable-replay",
        status: "UNSUPPORTED",
        detail:
            "M1 proved live no-turn operations only; use last durable state plus explicit reconnect"
                .into(),
    });
    report
}
