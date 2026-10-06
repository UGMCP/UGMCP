use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::AppError;
use crate::safety::danger_in_json;
use crate::storage::{SavedMcp, Storage};
use crate::util::lock;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolInfo {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerInfo {
    pub id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub status: String,
    pub tools: Vec<McpToolInfo>,
    pub message: String,
    pub running: bool,
}

struct Router {
    pending: Mutex<HashMap<i64, mpsc::Sender<Value>>>,
    tools: Mutex<Vec<McpToolInfo>>,
    status: Mutex<String>,
    message: Mutex<String>,
}

struct LiveServer {
    child: Child,
    stdin: ChildStdin,
    router: Arc<Router>,
    next_id: Mutex<i64>,
    reader: Option<std::thread::JoinHandle<()>>,
}

pub struct McpManager {
    storage: Arc<Storage>,
    live: Mutex<HashMap<String, LiveServer>>,
}

impl McpManager {
    pub fn new(storage: Arc<Storage>) -> Self {
        Self {
            storage,
            live: Mutex::new(HashMap::new()),
        }
    }

    pub fn list(&self) -> Vec<McpServerInfo> {
        let saved = self.storage.mcp_servers();
        let live = lock(&self.live);
        saved
            .into_iter()
            .map(|server| {
                if let Some(running) = live.get(&server.id) {
                    info_from_saved(&server, Some(running))
                } else {
                    info_from_saved(&server, None)
                }
            })
            .collect()
    }

    pub fn connect(
        &self,
        name: String,
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    ) -> Result<McpServerInfo, AppError> {
        let command = command.trim().to_string();
        if command.is_empty() || command.contains('\0') {
            return Err(AppError::new("COMMAND_FAILED", "MCP command is empty."));
        }
        let id = Uuid::new_v4().to_string();
        let saved = SavedMcp {
            id: id.clone(),
            name: if name.trim().is_empty() {
                command.clone()
            } else {
                name.trim().to_string()
            },
            command: command.clone(),
            args: args.clone(),
            env: env.clone(),
            auto_start: false,
        };
        self.spawn(&saved)?;
        self.storage.save_mcp(saved.clone())?;
        self.list()
            .into_iter()
            .find(|server| server.id == id)
            .ok_or_else(|| {
                AppError::new("COMMAND_FAILED", "MCP server disappeared during startup.")
            })
    }

    pub fn start_saved(&self, id: &str) -> Result<McpServerInfo, AppError> {
        if lock(&self.live).contains_key(id) {
            return self
                .list()
                .into_iter()
                .find(|server| server.id == id)
                .ok_or_else(|| AppError::new("COMMAND_FAILED", "MCP server is not connected."));
        }
        let saved = self
            .storage
            .mcp_servers()
            .into_iter()
            .find(|server| server.id == id)
            .ok_or_else(|| AppError::new("COMMAND_FAILED", "MCP server is not saved."))?;
        self.spawn(&saved)?;
        self.list()
            .into_iter()
            .find(|server| server.id == id)
            .ok_or_else(|| {
                AppError::new("COMMAND_FAILED", "MCP server disappeared during startup.")
            })
    }

    pub fn disconnect(&self, id: &str) -> Result<(), AppError> {
        let mut live = lock(&self.live);
        if let Some(server) = live.remove(id) {
            stop_live(server);
        }
        Ok(())
    }

    pub fn forget(&self, id: &str) -> Result<(), AppError> {
        self.disconnect(id)?;
        self.storage.remove_mcp(id)
    }

    pub fn call_tool(
        &self,
        id: &str,
        name: &str,
        arguments: Value,
        confirmed: bool,
        acknowledged_danger: bool,
    ) -> Result<Value, AppError> {
        if !confirmed {
            return Err(AppError::permission(
                "Tool calls require explicit confirmation.",
            ));
        }
        if let Some(warning) = danger_in_json(&arguments) {
            if !acknowledged_danger {
                return Err(AppError::new(
                    "NEEDS_CONFIRM",
                    format!("This tool call includes a destructive command: {warning}"),
                ));
            }
        }
        let mut live = lock(&self.live);
        let server = live
            .get_mut(id)
            .ok_or_else(|| AppError::new("COMMAND_FAILED", "MCP server is not connected."))?;
        let id_num = {
            let mut next = lock(&server.next_id);
            *next += 1;
            *next
        };
        let (tx, rx) = mpsc::channel();
        lock(&server.router.pending).insert(id_num, tx);
        let request = json!({
            "jsonrpc": "2.0",
            "id": id_num,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        });
        if let Err(err) = write_message(&mut server.stdin, &request) {
            lock(&server.router.pending).remove(&id_num);
            return Err(AppError::new("COMMAND_FAILED", err));
        }
        drop(live);
        let response = rx
            .recv_timeout(Duration::from_secs(60))
            .map_err(|_| AppError::new("COMMAND_FAILED", "The MCP tool call timed out."))?;
        if let Some(error) = response.get("error") {
            let message = error
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("MCP tool failed.");
            return Err(AppError::new("COMMAND_FAILED", message));
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    pub fn shutdown(&self) {
        let mut live = lock(&self.live);
        let servers: Vec<_> = live.drain().map(|(_, server)| server).collect();
        drop(live);
        for server in servers {
            stop_live(server);
        }
    }

    fn spawn(&self, saved: &SavedMcp) -> Result<(), AppError> {
        let mut command = Command::new(&saved.command);
        command
            .args(&saved.args)
            .envs(&saved.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("UNIT_AGENT_MCP", "1");
        let mut child = command.spawn().map_err(|err| {
            AppError::new(
                "COMMAND_FAILED",
                format!("Could not start the MCP server: {err}"),
            )
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppError::new("COMMAND_FAILED", "MCP server has no stdin."))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::new("COMMAND_FAILED", "MCP server has no stdout."))?;
        let stderr = child.stderr.take();
        let router = Arc::new(Router {
            pending: Mutex::new(HashMap::new()),
            tools: Mutex::new(Vec::new()),
            status: Mutex::new("connecting".into()),
            message: Mutex::new(String::new()),
        });
        let router_reader = router.clone();
        let (incoming_tx, incoming_rx) = mpsc::channel::<Value>();
        let reader = std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
                            let _ = incoming_tx.send(value);
                        }
                    }
                    Err(_) => break,
                }
            }
            *lock(&router_reader.status) = "disconnected".into();
        });
        if let Some(stderr) = stderr {
            let router_err = router.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stderr);
                let mut line = String::new();
                if reader.read_line(&mut line).is_ok() {
                    let text = line.trim();
                    if !text.is_empty() {
                        let mut message = lock(&router_err.message);
                        if message.is_empty() {
                            *message = text.chars().take(240).collect();
                        }
                    }
                }
            });
        }
        let router_dispatch = router.clone();
        std::thread::spawn(move || {
            while let Ok(message) = incoming_rx.recv() {
                dispatch_server_message(&router_dispatch, message);
            }
        });

        let mut live = LiveServer {
            child,
            stdin,
            router: router.clone(),
            next_id: Mutex::new(0),
            reader: Some(reader),
        };
        if let Err(err) = initialize(&mut live) {
            stop_live(live);
            return Err(err);
        }
        lock(&self.live).insert(saved.id.clone(), live);
        Ok(())
    }
}

fn initialize(server: &mut LiveServer) -> Result<(), AppError> {
    let init_id = 1;
    *lock(&server.next_id) = 1;
    let (tx, rx) = mpsc::channel();
    lock(&server.router.pending).insert(init_id, tx);
    let request = json!({
        "jsonrpc": "2.0",
        "id": init_id,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "Unit Agent", "version": env!("CARGO_PKG_VERSION") }
        }
    });
    write_message(&mut server.stdin, &request)
        .map_err(|err| AppError::new("COMMAND_FAILED", err))?;
    let response = rx.recv_timeout(Duration::from_secs(12)).map_err(|_| {
        AppError::new(
            "COMMAND_FAILED",
            "MCP server did not complete initialization.",
        )
    })?;
    if response.get("error").is_some() {
        return Err(AppError::new(
            "COMMAND_FAILED",
            "MCP server rejected initialization.",
        ));
    }
    write_message(
        &mut server.stdin,
        &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .map_err(|err| AppError::new("COMMAND_FAILED", err))?;

    let tools_id = 2;
    *lock(&server.next_id) = 2;
    let (tx, rx) = mpsc::channel();
    lock(&server.router.pending).insert(tools_id, tx);
    write_message(
        &mut server.stdin,
        &json!({"jsonrpc": "2.0", "id": tools_id, "method": "tools/list", "params": {}}),
    )
    .map_err(|err| AppError::new("COMMAND_FAILED", err))?;
    if let Ok(response) = rx.recv_timeout(Duration::from_secs(8)) {
        if let Some(tools) = response.pointer("/result/tools").and_then(|v| v.as_array()) {
            let parsed = tools
                .iter()
                .filter_map(|tool| {
                    let name = tool.get("name")?.as_str()?.to_string();
                    let description = tool
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .chars()
                        .take(400)
                        .collect();
                    Some(McpToolInfo { name, description })
                })
                .collect();
            *lock(&server.router.tools) = parsed;
        }
    }
    *lock(&server.router.status) = "connected".into();
    Ok(())
}

fn dispatch_server_message(router: &Router, message: Value) {
    if message.get("method").is_some() {
        return;
    }
    if let Some(id) = message.get("id").and_then(|v| v.as_i64()) {
        if let Some(tx) = lock(&router.pending).remove(&id) {
            let _ = tx.send(message);
        }
    }
}

fn write_message(stdin: &mut ChildStdin, message: &Value) -> Result<(), String> {
    let data = encode_message(message)?;
    stdin
        .write_all(data.as_bytes())
        .map_err(|_| "Could not write to the MCP server.".to_string())?;
    stdin
        .flush()
        .map_err(|_| "Could not write to the MCP server.".to_string())?;
    Ok(())
}

fn stop_live(mut server: LiveServer) {
    let _ = server.child.kill();
    let _ = server.child.wait();
    if let Some(handle) = server.reader.take() {
        let _ = handle.join();
    }
}

fn info_from_saved(server: &SavedMcp, live: Option<&LiveServer>) -> McpServerInfo {
    if let Some(live) = live {
        McpServerInfo {
            id: server.id.clone(),
            name: server.name.clone(),
            command: server.command.clone(),
            args: server.args.clone(),
            status: lock(&live.router.status).clone(),
            tools: lock(&live.router.tools).clone(),
            message: lock(&live.router.message).clone(),
            running: true,
        }
    } else {
        McpServerInfo {
            id: server.id.clone(),
            name: server.name.clone(),
            command: server.command.clone(),
            args: server.args.clone(),
            status: "saved".into(),
            tools: Vec::new(),
            message: String::new(),
            running: false,
        }
    }
}

fn encode_message(value: &Value) -> Result<String, String> {
    let mut data =
        serde_json::to_string(value).map_err(|_| "Could not encode MCP message.".to_string())?;
    if data.contains('\n') {
        return Err("MCP message contained a newline.".into());
    }
    data.push('\n');
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_jsonrpc_with_a_single_newline() {
        let encoded =
            encode_message(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"})).unwrap();
        assert!(encoded.ends_with('\n'));
        assert_eq!(encoded.chars().filter(|c| *c == '\n').count(), 1);
        let parsed: Value = serde_json::from_str(encoded.trim()).unwrap();
        assert_eq!(parsed["method"], "initialize");
    }
}
