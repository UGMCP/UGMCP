use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::{json, Value};

use crate::auth::AuthService;
use crate::config::PryselConfig;
use crate::connection::ConnectionService;
use crate::control::{tool_catalog, ControlHub, MCP_ADDR, MCP_PROTOCOL};
use crate::storage::Storage;
use crate::terminal::TerminalManager;

pub const CLI_HELP: &str = "\
Unit Agent

  unit-agent                  Open the desktop
  unit-agent --headless       Control API and terminals without a window
  unit-agent serve            Same as --headless
  unit-agent mcp              Speak MCP on stdin for Claude, Codex, and other clients
  unit-agent status           Application status from the running control API
  unit-agent diagnostics      Local health, including when Prysel is offline
  unit-agent mcp status       Local MCP listener
  unit-agent mcp start        Accept AI tool calls again
  unit-agent mcp stop         Stop AI tool calls without closing terminals
  unit-agent ai list          Connected AI clients
  unit-agent workspace list   Workspaces on this machine
  unit-agent workspace create [name]
  unit-agent exec <command>   Run in the shared persistent terminal
  unit-agent git status
  unit-agent project detect
  unit-agent terminal list

The desktop and this CLI share one backend when the app is running.
MCP listens on 127.0.0.1:47823. It does not bind a public address.
";

#[derive(Debug)]
pub struct McpGuard {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    pub addr: SocketAddr,
}

impl Drop for McpGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn bind(hub: Arc<ControlHub>, addr: &str) -> Result<McpGuard, String> {
    let parsed: SocketAddr = addr
        .parse()
        .map_err(|_| format!("invalid MCP address {addr}"))?;
    if !parsed.ip().is_loopback() {
        return Err(
            "refusing to bind MCP outside the loopback interface. Remote control stays off.".into(),
        );
    }
    let listener =
        TcpListener::bind(parsed).map_err(|err| format!("could not bind MCP on {addr}: {err}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|err| err.to_string())?;
    let addr = listener.local_addr().map_err(|err| err.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();
    let thread = thread::spawn(move || accept_loop(listener, hub, stop_thread));
    Ok(McpGuard {
        stop,
        thread: Some(thread),
        addr,
    })
}

fn accept_loop(listener: TcpListener, hub: Arc<ControlHub>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let hub = hub.clone();
                thread::spawn(move || session(stream, hub));
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(err) => {
                log::warn!("MCP accept failed: {err}");
                break;
            }
        }
    }
}

fn session(stream: TcpStream, hub: Arc<ControlHub>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(120)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(15)));
    let mut writer = match stream.try_clone() {
        Ok(writer) => writer,
        Err(_) => return,
    };
    let mut reader = BufReader::new(stream);
    let mut ai_session: Option<String> = None;
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(_) => {
                let _ = write_msg(
                    &mut writer,
                    &json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}),
                );
                continue;
            }
        };
        if message.get("id").is_none() {
            continue;
        }
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        let response = match method {
            "initialize" => {
                let name = params
                    .pointer("/clientInfo/name")
                    .and_then(Value::as_str)
                    .unwrap_or("MCP");
                ai_session = Some(hub.open_session(name));
                ok_result(
                    id,
                    json!({
                        "protocolVersion": MCP_PROTOCOL,
                        "capabilities": { "tools": { "listChanged": false }, "resources": {} },
                        "serverInfo": { "name": "Unit Agent", "version": env!("CARGO_PKG_VERSION") },
                    }),
                )
            }
            "tools/list" => ok_result(id, json!({ "tools": tool_catalog() })),
            "tools/call" => {
                let session_id = ai_session
                    .get_or_insert_with(|| hub.open_session("MCP"))
                    .clone();
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let payload = hub.call_tool(&session_id, name, &args);
                let is_error = payload.get("success") == Some(&Value::Bool(false));
                ok_result(
                    id,
                    json!({
                        "content": [{ "type": "text", "text": payload.to_string() }],
                        "isError": is_error,
                    }),
                )
            }
            "resources/list" => ok_result(id, json!({ "resources": resource_list() })),
            "resources/read" => {
                let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
                match resource_body(&hub, uri) {
                    Some(body) => ok_result(
                        id,
                        json!({
                            "contents": [{ "uri": uri, "mimeType": "application/json", "text": body }],
                        }),
                    ),
                    None => err_result(id, -32002, "Unknown resource"),
                }
            }
            "ping" => ok_result(id, json!({})),
            _ => err_result(id, -32601, "Method not found"),
        };
        if write_msg(&mut writer, &response).is_err() {
            break;
        }
    }
    if let Some(id) = ai_session {
        hub.disconnect_session(&id);
    }
}

fn resource_list() -> Value {
    json!([
        { "uri": "unit-agent://status", "name": "status", "mimeType": "application/json" },
        { "uri": "unit-agent://workspaces", "name": "workspaces", "mimeType": "application/json" },
        { "uri": "unit-agent://ai/sessions", "name": "ai-sessions", "mimeType": "application/json" },
        { "uri": "unit-agent://system", "name": "system", "mimeType": "application/json" },
    ])
}

fn resource_body(hub: &ControlHub, uri: &str) -> Option<String> {
    let value = match uri {
        "unit-agent://status" => hub.status_value(),
        "unit-agent://workspaces" => json!({ "workspaces": hub.terminals.list() }),
        "unit-agent://ai/sessions" => json!({ "clients": hub.clients() }),
        "unit-agent://system" => hub.call_tool("resource", "unit_agent_system_info", &json!({})),
        _ => return None,
    };
    Some(value.to_string())
}

fn ok_result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn err_result(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn write_msg(writer: &mut TcpStream, value: &Value) -> std::io::Result<()> {
    writeln!(writer, "{value}")?;
    writer.flush()
}

pub fn mcp_main(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        None | Some("--stdio") => serve_stdio(),
        Some("status") => cli(&["mcp".into(), "status".into()]),
        Some("start") => cli(&["mcp".into(), "start".into()]),
        Some("stop") => cli(&["mcp".into(), "stop".into()]),
        Some(other) => {
            eprintln!("unknown mcp command {other}\n{CLI_HELP}");
            2
        }
    }
}

pub fn serve_stdio() -> i32 {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .try_init();
    let addr: SocketAddr = match MCP_ADDR.parse() {
        Ok(addr) => addr,
        Err(err) => {
            eprintln!("unit-agent mcp: {err}");
            return 1;
        }
    };
    if let Ok(stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(400)) {
        eprintln!("unit-agent mcp: using the desktop control API at {MCP_ADDR}");
        bridge(stream);
        return 0;
    }
    eprintln!(
        "unit-agent mcp: nothing is listening on {MCP_ADDR}. Starting a separate headless hub. These PTYs are not the desktop's."
    );
    let hub = embedded_hub();
    stdio_session(hub);
    0
}

fn bridge(mut stream: TcpStream) {
    let mut outbound = match stream.try_clone() {
        Ok(stream) => stream,
        Err(err) => {
            eprintln!("unit-agent mcp: {err}");
            return;
        }
    };
    thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut stdin = stdin.lock();
        let _ = std::io::copy(&mut stdin, &mut outbound);
    });
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    let _ = std::io::copy(&mut stream, &mut stdout);
}

fn embedded_hub() -> Arc<ControlHub> {
    let config = PryselConfig::load();
    let storage = Arc::new(Storage::open());
    let auth = Arc::new(AuthService::new(config.clone(), storage.clone()));
    let connection = Arc::new(ConnectionService::new());
    let terminals = Arc::new(TerminalManager::new());
    Arc::new(ControlHub::new(
        terminals, storage, auth, connection, config,
    ))
}

fn stdio_session(hub: Arc<ControlHub>) {
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();
    let mut ai_session: Option<String> = None;
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(_) => {
                let _ = writeln!(
                    writer,
                    "{}",
                    json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}})
                );
                let _ = writer.flush();
                continue;
            }
        };
        if message.get("id").is_none() {
            continue;
        }
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        let response = match method {
            "initialize" => {
                let name = params
                    .pointer("/clientInfo/name")
                    .and_then(Value::as_str)
                    .unwrap_or("MCP");
                ai_session = Some(hub.open_session(name));
                ok_result(
                    id,
                    json!({
                        "protocolVersion": MCP_PROTOCOL,
                        "capabilities": { "tools": { "listChanged": false }, "resources": {} },
                        "serverInfo": { "name": "Unit Agent", "version": env!("CARGO_PKG_VERSION") },
                    }),
                )
            }
            "tools/list" => ok_result(id, json!({ "tools": tool_catalog() })),
            "tools/call" => {
                let session_id = ai_session
                    .get_or_insert_with(|| hub.open_session("MCP"))
                    .clone();
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let payload = hub.call_tool(&session_id, name, &args);
                let is_error = payload.get("success") == Some(&Value::Bool(false));
                ok_result(
                    id,
                    json!({
                        "content": [{ "type": "text", "text": payload.to_string() }],
                        "isError": is_error,
                    }),
                )
            }
            "resources/list" => ok_result(id, json!({ "resources": resource_list() })),
            "resources/read" => {
                let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
                match resource_body(&hub, uri) {
                    Some(body) => ok_result(
                        id,
                        json!({ "contents": [{ "uri": uri, "mimeType": "application/json", "text": body }] }),
                    ),
                    None => err_result(id, -32002, "Unknown resource"),
                }
            }
            "ping" => ok_result(id, json!({})),
            _ => err_result(id, -32601, "Method not found"),
        };
        if writeln!(writer, "{response}").is_err() {
            break;
        }
        if writer.flush().is_err() {
            break;
        }
    }
    if let Some(id) = ai_session {
        hub.disconnect_session(&id);
    }
}

pub fn cli(args: &[String]) -> i32 {
    let (tool, arguments) = match args {
        [cmd] if cmd == "status" => ("unit_agent_app_status", json!({})),
        [cmd] if cmd == "diagnostics" => ("unit_agent_diagnostics", json!({})),
        [a, b] if a == "mcp" && b == "status" => ("unit_agent_mcp_status", json!({})),
        [a, b] if a == "mcp" && b == "start" => ("unit_agent_mcp_start", json!({})),
        [a, b] if a == "mcp" && b == "stop" => ("unit_agent_mcp_stop", json!({})),
        [a, b] if a == "ai" && b == "list" => ("unit_agent_ai_sessions", json!({})),
        [a, b] if a == "workspace" && b == "list" => ("unit_agent_workspace_list", json!({})),
        [a, b] if a == "terminal" && b == "list" => ("unit_agent_process_list", json!({})),
        [a, b] if a == "git" && b == "status" => ("unit_agent_git_status", json!({})),
        [a, b] if a == "project" && b == "detect" => ("unit_agent_project_detect", json!({})),
        [a, b, name] if a == "workspace" && b == "create" => {
            ("unit_agent_workspace_create", json!({ "name": name }))
        }
        [a, b] if a == "workspace" && b == "create" => {
            ("unit_agent_workspace_create", json!({ "name": "Terminal" }))
        }
        [a, rest @ ..] if a == "exec" && !rest.is_empty() => {
            ("unit_agent_execute", json!({ "command": rest.join(" ") }))
        }
        _ => {
            eprintln!("{CLI_HELP}");
            return 2;
        }
    };
    match call_running(tool, arguments) {
        Ok(text) => {
            println!("{text}");
            0
        }
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}

fn call_running(tool: &str, arguments: Value) -> Result<String, String> {
    let addr: SocketAddr = MCP_ADDR
        .parse()
        .map_err(|_| format!("invalid MCP address {MCP_ADDR}"))?;
    let stream = TcpStream::connect_timeout(&addr, Duration::from_millis(400)).map_err(|_| {
        format!("Unit Agent is not listening on {MCP_ADDR}. Start the desktop or unit-agent --headless.")
    })?;
    let mut writer = stream.try_clone().map_err(|err| err.to_string())?;
    let mut reader = BufReader::new(stream);
    rpc(
        &mut writer,
        &mut reader,
        1,
        "initialize",
        json!({ "protocolVersion": MCP_PROTOCOL, "clientInfo": { "name": "unit-agent", "version": env!("CARGO_PKG_VERSION") } }),
    )?;
    let result = rpc(
        &mut writer,
        &mut reader,
        2,
        "tools/call",
        json!({ "name": tool, "arguments": arguments }),
    )?;
    let text = result
        .pointer("/content/0/text")
        .and_then(Value::as_str)
        .unwrap_or("");
    match serde_json::from_str::<Value>(text) {
        Ok(value) => serde_json::to_string_pretty(&value).map_err(|err| err.to_string()),
        Err(_) => Ok(text.to_string()),
    }
}

fn rpc(
    writer: &mut TcpStream,
    reader: &mut BufReader<TcpStream>,
    id: i64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let request = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    writeln!(writer, "{request}").map_err(|err| err.to_string())?;
    writer.flush().map_err(|err| err.to_string())?;
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|err| err.to_string())?;
    if line.is_empty() {
        return Err("MCP closed the connection".into());
    }
    let value: Value = serde_json::from_str(line.trim()).map_err(|err| err.to_string())?;
    if let Some(error) = value.get("error") {
        return Err(error.to_string());
    }
    Ok(value.get("result").cloned().unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hub() -> Arc<ControlHub> {
        let dir = std::env::temp_dir().join(format!("unit-agent-mcp-{}", uuid::Uuid::new_v4()));
        let storage = Arc::new(Storage::open_in(dir.join("config"), dir.join("data")));
        let config = PryselConfig::load();
        let auth = Arc::new(AuthService::new(config.clone(), storage.clone()));
        let connection = Arc::new(ConnectionService::new());
        let terminals = Arc::new(TerminalManager::new());
        Arc::new(ControlHub::new(
            terminals, storage, auth, connection, config,
        ))
    }

    #[test]
    fn refuses_a_public_bind() {
        let error = bind(hub(), "0.0.0.0:9").expect_err("public bind");
        assert!(error.contains("loopback"), "{error}");
    }

    #[test]
    fn lists_tools_and_ignores_malformed_json() {
        let guard = bind(hub(), "127.0.0.1:0").expect("bind");
        let stream = TcpStream::connect(guard.addr).expect("connect");
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        let init = rpc(
            &mut writer,
            &mut reader,
            1,
            "initialize",
            json!({ "clientInfo": { "name": "Claude" } }),
        )
        .expect("init");
        assert_eq!(init["serverInfo"]["name"], "Unit Agent");
        let tools = rpc(&mut writer, &mut reader, 2, "tools/list", json!({})).expect("tools");
        let names = tools["tools"].as_array().cloned().unwrap_or_default();
        assert!(names
            .iter()
            .any(|tool| tool["name"] == "unit_agent_execute"));
        assert!(names
            .iter()
            .any(|tool| tool["name"] == "unit_agent_workspace_list"));
        writeln!(writer, "not json").unwrap();
        let mut bad = String::new();
        reader.read_line(&mut bad).unwrap();
        assert!(bad.contains("Parse error"), "{bad}");
        let status = rpc(
            &mut writer,
            &mut reader,
            3,
            "tools/call",
            json!({ "name": "unit_agent_app_status", "arguments": {} }),
        )
        .expect("status");
        let text = status["content"][0]["text"].as_str().unwrap_or("");
        assert!(text.contains("Unit Agent"), "{text}");
        let resources =
            rpc(&mut writer, &mut reader, 4, "resources/list", json!({})).expect("resources");
        assert!(resources["resources"]
            .to_string()
            .contains("unit-agent://status"));
    }
}
