use clap::{Parser, Subcommand};
use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use zodex_control_plane::authority::{current_uid, AuthorityGuard};
use zodex_control_plane::protocol::{
    ClientRecord, ClientRegistry, Engine, Request, MAX_REQUEST_BYTES,
};
use zodex_control_plane::state::StatePaths;

#[derive(Parser)]
#[command(name = "zodex-control-plane", version)]
struct Cli {
    #[arg(long)]
    state_dir: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    LaunchTmux,
    Observe,
    Serve,
    AgentHost {
        #[arg(long)]
        native_codex: PathBuf,
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long)]
        socket: PathBuf,
        #[arg(long)]
        codex_home: Option<PathBuf>,
    },
    Enroll {
        #[arg(long)]
        client_id: String,
        #[arg(long)]
        client_version: String,
        #[arg(long = "capability", required = true)]
        capabilities: Vec<String>,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let paths = StatePaths::new(cli.state_dir);
    paths.initialize()?;
    match cli.command {
        Command::LaunchTmux => zodex_control_plane::tmux_host::launch(&paths),
        Command::Observe => {
            println!(
                "{}",
                serde_json::json!({"schemaVersion": 1, "running": zodex_control_plane::tmux_host::observe(&paths)?})
            );
            Ok(())
        }
        Command::Serve => serve(paths),
        Command::AgentHost {
            native_codex,
            workspace,
            socket,
            codex_home,
        } => {
            let mut child = zodex_control_plane::agent::agent_host(
                &native_codex,
                &workspace,
                &socket,
                codex_home.as_deref(),
            )?;
            let status = child.wait()?;
            anyhow::ensure!(status.success(), "native App Server exited unsuccessfully");
            Ok(())
        }
        Command::Enroll {
            client_id,
            client_version,
            capabilities,
        } => {
            let mut registry = ClientRegistry::load(&paths)?;
            registry.enroll(ClientRecord {
                client_id,
                client_version,
                capabilities: capabilities.into_iter().collect::<BTreeSet<_>>(),
                enabled: true,
            })?;
            registry.save(&paths)
        }
    }
}

fn serve(paths: StatePaths) -> anyhow::Result<()> {
    let _authority = AuthorityGuard::acquire(&paths)?;
    let listener = UnixListener::bind(&paths.socket)?;
    fs::set_permissions(&paths.socket, fs::Permissions::from_mode(0o600))?;
    let registry = ClientRegistry::load(&paths)?;
    let mut engine = Engine::new(paths, registry);
    for stream in listener.incoming() {
        handle(stream?, &mut engine)?;
    }
    Ok(())
}

fn handle(mut stream: UnixStream, engine: &mut Engine) -> anyhow::Result<()> {
    anyhow::ensure!(peer_uid(&stream)? == current_uid()?, "foreign peer refused");
    let reader_stream = stream.try_clone()?;
    let mut reader = BufReader::new(reader_stream);
    loop {
        let mut line = Vec::new();
        let bytes = reader
            .by_ref()
            .take((MAX_REQUEST_BYTES + 1) as u64)
            .read_until(b'\n', &mut line)?;
        if bytes == 0 {
            return Ok(());
        }
        anyhow::ensure!(line.len() <= MAX_REQUEST_BYTES, "request too large");
        let request: Request = serde_json::from_slice(&line)?;
        serde_json::to_writer(&mut stream, &engine.handle(request))?;
        stream.write_all(b"\n")?;
        stream.flush()?;
    }
}

fn peer_uid(stream: &UnixStream) -> anyhow::Result<u32> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut credentials as *mut _ as *mut libc::c_void,
            &mut length,
        )
    };
    anyhow::ensure!(
        result == 0 && length as usize == std::mem::size_of::<libc::ucred>(),
        "peer credential check failed"
    );
    Ok(credentials.uid)
}
