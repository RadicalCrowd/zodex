use serde_json::{json, Value};
use std::os::unix::net::UnixStream;
use std::path::Path;
use tungstenite::{client, Message, WebSocket};

pub struct AppServerAdapter {
    socket: WebSocket<UnixStream>,
    next_id: u64,
}

impl AppServerAdapter {
    pub fn connect(socket_path: &Path) -> anyhow::Result<Self> {
        let stream = UnixStream::connect(socket_path)?;
        let (socket, _) = client("ws://localhost/rpc", stream)?;
        let mut adapter = Self { socket, next_id: 1 };
        adapter.request(
            "initialize",
            json!({
                "clientInfo": {"name":"zodex-control-plane","version":env!("CARGO_PKG_VERSION")},
                "capabilities": {"experimentalApi":true}
            }),
        )?;
        adapter.socket.send(Message::Text(
            json!({"method":"initialized","params":{}})
                .to_string()
                .into(),
        ))?;
        Ok(adapter)
    }

    pub fn request(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.socket.send(Message::Text(
            json!({"id":id,"method":method,"params":params})
                .to_string()
                .into(),
        ))?;
        loop {
            match self.socket.read()? {
                Message::Text(text) => {
                    let message: Value = serde_json::from_str(&text)?;
                    if message.get("id").and_then(Value::as_u64) != Some(id) {
                        continue;
                    }
                    if let Some(error) = message.get("error") {
                        anyhow::bail!("App Server request failed: {error}");
                    }
                    return Ok(message.get("result").cloned().unwrap_or(Value::Null));
                }
                Message::Close(_) => anyhow::bail!("App Server disconnected"),
                Message::Ping(payload) => self.socket.send(Message::Pong(payload))?,
                _ => {}
            }
        }
    }

    pub fn start_thread(&mut self, workspace: &Path) -> anyhow::Result<String> {
        let result = self.request(
            "thread/start",
            json!({"cwd":workspace,"ephemeral":false,"environments":[]}),
        )?;
        result
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| anyhow::anyhow!("thread/start returned no id"))
    }

    pub fn inject_synthetic_item(&mut self, thread_id: &str, text: &str) -> anyhow::Result<()> {
        self.request("thread/inject_items", json!({"threadId":thread_id,"items":[{"type":"message","role":"user","content":[{"type":"input_text","text":text}]}]}))?;
        Ok(())
    }

    pub fn read_thread(&mut self, thread_id: &str) -> anyhow::Result<Value> {
        self.request(
            "thread/read",
            json!({"threadId":thread_id,"includeTurns":true}),
        )
    }

    pub fn archive_thread(&mut self, thread_id: &str) -> anyhow::Result<()> {
        self.request("thread/archive", json!({"threadId":thread_id}))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::{Child, Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};
    use tempfile::tempdir;

    struct ChildGuard(Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn real_app_server_supports_no_turn_vertical_slice_without_model_call() {
        let codex = match std::env::var("ZODEX_TEST_NATIVE_CODEX") {
            Ok(path) => path,
            Err(_) => return,
        };
        let root = tempdir().unwrap();
        let codex_home = root.path().join("codex-home");
        let workspace = root.path().join("workspace");
        fs::create_dir(&codex_home).unwrap();
        fs::create_dir(&workspace).unwrap();
        let socket = root.path().join("app-server.sock");
        let child = Command::new(codex)
            .args([
                "app-server",
                "--listen",
                &format!("unix://{}", socket.display()),
            ])
            .env("CODEX_HOME", &codex_home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let _guard = ChildGuard(child);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !socket.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        let mut adapter = AppServerAdapter::connect(&socket).unwrap();
        let thread_id = adapter.start_thread(&workspace).unwrap();
        adapter
            .inject_synthetic_item(&thread_id, "M4 synthetic sentinel")
            .unwrap();
        let read = adapter.read_thread(&thread_id).unwrap();
        assert_eq!(
            read.pointer("/thread/id").and_then(Value::as_str),
            Some(thread_id.as_str())
        );
        adapter.archive_thread(&thread_id).unwrap();
    }
}
