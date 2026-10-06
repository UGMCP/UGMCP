use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use crate::error::AppError;
use crate::system::{detect_shell, host_identity};
use crate::terminal::{CreateTerminalRequest, TerminalManager};

const DEFAULT_BIND: &str = "127.0.0.1:47822";
const MAX_HEADER: usize = 16 * 1024;
const MAX_BODY: usize = 256 * 1024;

pub fn serve(args: &[String]) -> i32 {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .try_init();
    match parse_args(
        args,
        env_value("UNIT_AGENT_BIND"),
        env_value("UNIT_AGENT_SERVER_TOKEN"),
    ) {
        Ok(ParseOutcome::Help) => {
            println!("{HELP}");
            0
        }
        Ok(ParseOutcome::Run(options)) => match run(options) {
            Ok(()) => 0,
            Err(err) => {
                eprintln!("unit-agent server: {err}");
                1
            }
        },
        Err(err) => {
            eprintln!("{err}");
            2
        }
    }
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

const HELP: &str = "\
Unit Agent server

Usage:
  unit-agent-server [--bind ADDR] [--token TOKEN]
  unit-agent serve [--bind ADDR] [--token TOKEN]

Listens on 127.0.0.1:47822 unless --bind or UNIT_AGENT_BIND is set.
A token is required when the address is not a loopback address.
UNIT_AGENT_SERVER_TOKEN is preferred over --token because process arguments
are visible to other users on the machine.

Endpoints:
  GET    /health
  GET    /api/v1/status
  GET    /api/v1/terminals
  POST   /api/v1/terminals
  GET    /api/v1/terminals/{id}
  POST   /api/v1/terminals/{id}/input
  POST   /api/v1/terminals/{id}/resize
  POST   /api/v1/terminals/{id}/confirm
  DELETE /api/v1/terminals/{id}
";

enum ParseOutcome {
    Help,
    Run(ServerOptions),
}

struct ServerOptions {
    bind: String,
    token: String,
}

fn parse_args(
    args: &[String],
    env_bind: Option<String>,
    env_token: Option<String>,
) -> Result<ParseOutcome, String> {
    let mut bind = env_bind;
    let mut token_arg: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if arg == "-h" || arg == "--help" {
            return Ok(ParseOutcome::Help);
        }
        if let Some(value) = arg.strip_prefix("--bind=") {
            bind = Some(value.to_string());
        } else if arg == "--bind" {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| "missing value for --bind".to_string())?;
            bind = Some(value.clone());
        } else if let Some(value) = arg.strip_prefix("--token=") {
            token_arg = Some(value.to_string());
        } else if arg == "--token" {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| "missing value for --token".to_string())?;
            token_arg = Some(value.clone());
        } else {
            return Err(format!(
                "unknown argument {arg}\nRun unit-agent-server --help"
            ));
        }
        index += 1;
    }
    Ok(ParseOutcome::Run(ServerOptions {
        bind: bind.unwrap_or_else(|| DEFAULT_BIND.into()),
        token: env_token.or(token_arg).unwrap_or_default(),
    }))
}

fn run(options: ServerOptions) -> Result<(), String> {
    let running = start(&options.bind, options.token)?;
    eprintln!(
        "Unit Agent server listening on http://{} (mode server)",
        running.addr
    );
    wait_for_stop();
    drop(running);
    Ok(())
}

fn wait_for_stop() {
    let (tx, rx) = std::sync::mpsc::channel();
    if ctrlc::set_handler(move || {
        let _ = tx.send(());
    })
    .is_err()
    {
        loop {
            thread::sleep(Duration::from_secs(3600));
        }
    }
    let _ = rx.recv();
}

struct Running {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Shared {
    terminals: TerminalManager,
    token: String,
}

fn start(bind: &str, token: String) -> Result<Running, String> {
    let addr: SocketAddr = bind
        .parse()
        .map_err(|_| format!("invalid bind address {bind}"))?;
    validate_bind(addr, &token)?;
    let listener =
        TcpListener::bind(addr).map_err(|err| format!("could not bind {addr}: {err}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|err| err.to_string())?;
    let addr = listener.local_addr().map_err(|err| err.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = stop.clone();
    let thread = thread::spawn(move || accept_loop(listener, stop_thread, token));
    Ok(Running {
        addr,
        stop,
        thread: Some(thread),
    })
}

fn validate_bind(addr: SocketAddr, token: &str) -> Result<(), String> {
    if !is_loopback(addr.ip()) && token.is_empty() {
        return Err(
            "refusing to listen outside the loopback interface without UNIT_AGENT_SERVER_TOKEN"
                .into(),
        );
    }
    Ok(())
}

fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_loopback(),
    }
}

fn accept_loop(listener: TcpListener, stop: Arc<AtomicBool>, token: String) {
    let shared = Arc::new(Shared {
        terminals: TerminalManager::new(),
        token,
    });
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let shared = shared.clone();
                thread::spawn(move || handle_client(stream, &shared));
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(err) => {
                log::warn!("accept failed: {err}");
                break;
            }
        }
    }
    shared.terminals.shutdown();
}

struct Request {
    method: String,
    path: String,
    query: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn handle_client(mut stream: TcpStream, shared: &Shared) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(15)));
    let response = match read_request(&mut stream) {
        Ok(request) => dispatch(shared, &request),
        Err(err) => error_response(400, &err),
    };
    let _ = write_response(&mut stream, &response);
}

fn read_request(stream: &mut TcpStream) -> Result<Request, AppError> {
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    while !header.windows(4).any(|window| window == b"\r\n\r\n") {
        if header.len() >= MAX_HEADER {
            return Err(AppError::new(
                "BAD_REQUEST",
                "Request headers are too large.",
            ));
        }
        let read = stream
            .read(&mut byte)
            .map_err(|_| AppError::new("BAD_REQUEST", "Could not read the request."))?;
        if read == 0 {
            return Err(AppError::new("BAD_REQUEST", "Incomplete request."));
        }
        header.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&header);
    let mut lines = text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| AppError::new("BAD_REQUEST", "Missing request line."))?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| AppError::new("BAD_REQUEST", "Missing method."))?
        .to_string();
    let target = parts
        .next()
        .ok_or_else(|| AppError::new("BAD_REQUEST", "Missing path."))?;
    let (path, query) = split_target(target);
    let mut headers = HashMap::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    if headers
        .get("transfer-encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"))
    {
        return Err(AppError::new(
            "BAD_REQUEST",
            "Chunked requests are not supported.",
        ));
    }
    let length = headers
        .get("content-length")
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|_| AppError::new("BAD_REQUEST", "Content-Length is invalid."))?
        .unwrap_or(0);
    if length > MAX_BODY {
        return Err(AppError::new("BAD_REQUEST", "Request body is too large."));
    }
    let mut body = vec![0u8; length];
    stream
        .read_exact(&mut body)
        .map_err(|_| AppError::new("BAD_REQUEST", "Incomplete request body."))?;
    Ok(Request {
        method,
        path,
        query,
        headers,
        body,
    })
}

fn split_target(target: &str) -> (String, String) {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    (decode_component(path), query.to_string())
}

fn decode_component(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(value) = u8::from_str_radix(
                std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or(""),
                16,
            ) {
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

struct Response {
    status: u16,
    body: Vec<u8>,
}

fn dispatch(shared: &Shared, request: &Request) -> Response {
    if !authorized(shared, request) {
        return error_response(
            401,
            &AppError::new("UNAUTHORIZED", "A server token is required."),
        );
    }
    let route = route_of(&request.method, &request.path);
    match route {
        Route::Health => json_response(200, &json!({ "status": "ok" })),
        Route::Status => json_response(200, &status_body()),
        Route::List => json_response(200, &shared.terminals.list()),
        Route::Create => match create_terminal(shared, request) {
            Ok(info) => json_response(201, &info),
            Err(err) => error_response(status_for(&err), &err),
        },
        Route::Get(id) => match shared.terminals.snapshot(&id) {
            Ok(snapshot) => json_response(200, &snapshot),
            Err(err) => error_response(status_for(&err), &err),
        },
        Route::Delete(id) => {
            let force = query_flag(&request.query, "force");
            match shared.terminals.close(&id, force) {
                Ok(()) => json_response(200, &json!({ "closed": true })),
                Err(err) => error_response(status_for(&err), &err),
            }
        }
        Route::Input(id) => match write_input(shared, &id, request) {
            Ok(result) => json_response(200, &result),
            Err(err) => error_response(status_for(&err), &err),
        },
        Route::Resize(id) => match resize_terminal(shared, &id, request) {
            Ok(()) => json_response(200, &json!({ "resized": true })),
            Err(err) => error_response(status_for(&err), &err),
        },
        Route::Confirm(id) => match confirm_terminal(shared, &id, request) {
            Ok(()) => json_response(200, &json!({ "confirmed": true })),
            Err(err) => error_response(status_for(&err), &err),
        },
        Route::Missing => error_response(404, &AppError::not_found("Not found.")),
    }
}

fn route_of(method: &str, path: &str) -> Route {
    match (method, path) {
        ("GET", "/health") => Route::Health,
        ("GET", "/api/v1/status") => Route::Status,
        ("GET", "/api/v1/terminals") => Route::List,
        ("POST", "/api/v1/terminals") => Route::Create,
        ("GET", path) => terminal_id(path, "")
            .map(Route::Get)
            .unwrap_or(Route::Missing),
        ("DELETE", path) => terminal_id(path, "")
            .map(Route::Delete)
            .unwrap_or(Route::Missing),
        ("POST", path) => post_terminal_route(path),
        _ => Route::Missing,
    }
}

fn post_terminal_route(path: &str) -> Route {
    if let Some(id) = terminal_id(path, "/input") {
        Route::Input(id)
    } else if let Some(id) = terminal_id(path, "/resize") {
        Route::Resize(id)
    } else if let Some(id) = terminal_id(path, "/confirm") {
        Route::Confirm(id)
    } else {
        Route::Missing
    }
}

enum Route {
    Health,
    Status,
    List,
    Create,
    Get(String),
    Delete(String),
    Input(String),
    Resize(String),
    Confirm(String),
    Missing,
}

fn authorized(shared: &Shared, request: &Request) -> bool {
    if request.method == "GET" && request.path == "/health" {
        return true;
    }
    if shared.token.is_empty() {
        return true;
    }
    presented_token(&request.headers).is_some_and(|token| tokens_match(&shared.token, &token))
}

fn presented_token(headers: &HashMap<String, String>) -> Option<String> {
    let value = headers.get("authorization")?;
    let (scheme, token) = value.split_once(char::is_whitespace)?;
    if scheme.eq_ignore_ascii_case("bearer") {
        let token = token.trim();
        if !token.is_empty() {
            return Some(token.to_string());
        }
    }
    None
}

fn tokens_match(expected: &str, provided: &str) -> bool {
    let left = expected.as_bytes();
    let right = provided.as_bytes();
    let mut diff = left.len() ^ right.len();
    let len = left.len().max(right.len());
    for index in 0..len {
        let a = left.get(index).copied().unwrap_or(0);
        let b = right.get(index).copied().unwrap_or(0);
        diff |= (a ^ b) as usize;
    }
    diff == 0 && !expected.is_empty()
}

fn terminal_id(path: &str, suffix: &str) -> Option<String> {
    let rest = path.strip_prefix("/api/v1/terminals/")?;
    let id = if suffix.is_empty() {
        rest
    } else {
        rest.strip_suffix(suffix)?
    };
    if valid_id(id) {
        Some(id.to_string())
    } else {
        None
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn query_flag(query: &str, name: &str) -> bool {
    query.split('&').any(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        key == name && (value == "1" || value.eq_ignore_ascii_case("true"))
    })
}

fn status_body() -> StatusBody {
    let host = host_identity();
    StatusBody {
        name: "Unit Agent",
        version: env!("CARGO_PKG_VERSION"),
        mode: "server",
        platform: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        shell: detect_shell(None).unwrap_or_else(|_| "unavailable".into()),
        computer_name: host.computer_name,
        server_name: host.server_name,
        network: host.network,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusBody {
    name: &'static str,
    version: &'static str,
    mode: &'static str,
    platform: &'static str,
    arch: &'static str,
    shell: String,
    computer_name: String,
    server_name: String,
    network: String,
}

fn create_terminal(
    shared: &Shared,
    request: &Request,
) -> Result<crate::terminal::TerminalInfo, AppError> {
    let body = json_body(request)?;
    let id = optional_string(&body, "id")?;
    if let Some(id) = &id {
        if !valid_id(id) {
            return Err(AppError::new("BAD_REQUEST", "Terminal id is invalid."));
        }
    }
    shared.terminals.create(CreateTerminalRequest {
        id,
        title: optional_string(&body, "title")?,
        cwd: optional_string(&body, "cwd")?,
        shell: optional_string(&body, "shell")?,
    })
}

fn write_input(
    shared: &Shared,
    id: &str,
    request: &Request,
) -> Result<crate::terminal::WriteResult, AppError> {
    let body = json_body(request)?;
    let data = body
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::new("BAD_REQUEST", "Input requires a data string."))?;
    shared.terminals.write(id, data)
}

fn resize_terminal(shared: &Shared, id: &str, request: &Request) -> Result<(), AppError> {
    let body = json_body(request)?;
    let cols = body
        .get("cols")
        .and_then(Value::as_u64)
        .ok_or_else(|| AppError::new("BAD_REQUEST", "Resize requires cols and rows."))?;
    let rows = body
        .get("rows")
        .and_then(Value::as_u64)
        .ok_or_else(|| AppError::new("BAD_REQUEST", "Resize requires cols and rows."))?;
    let cols = u16::try_from(cols).unwrap_or(u16::MAX);
    let rows = u16::try_from(rows).unwrap_or(u16::MAX);
    shared.terminals.resize(id, cols, rows)
}

fn confirm_terminal(shared: &Shared, id: &str, request: &Request) -> Result<(), AppError> {
    let body = json_body(request)?;
    let approve = body
        .get("approve")
        .and_then(Value::as_bool)
        .ok_or_else(|| AppError::new("BAD_REQUEST", "Confirm requires approve true or false."))?;
    shared.terminals.confirm(id, approve)
}

fn json_body(request: &Request) -> Result<Value, AppError> {
    if request.body.is_empty() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    serde_json::from_slice(&request.body)
        .map_err(|_| AppError::new("BAD_REQUEST", "Request body must be JSON."))
}

fn optional_string(body: &Value, name: &str) -> Result<Option<String>, AppError> {
    match body.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(AppError::new(
            "BAD_REQUEST",
            format!("{name} must be a string."),
        )),
    }
}

fn status_for(err: &AppError) -> u16 {
    match err.code.as_str() {
        "NOT_FOUND" => 404,
        "UNAUTHORIZED" => 401,
        "BAD_REQUEST" => 400,
        "NEEDS_CONFIRM" => 409,
        "SHELL_UNAVAILABLE" | "TERMINAL_START_FAILED" => 400,
        _ => 500,
    }
}

fn json_response(status: u16, value: &impl Serialize) -> Response {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    Response { status, body }
}

fn error_response(status: u16, err: &AppError) -> Response {
    json_response(status, err)
}

fn write_response(stream: &mut TcpStream, response: &Response) -> std::io::Result<()> {
    let reason = reason_phrase(response.status);
    let header = format!(
        "HTTP/1.1 {} {reason}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        response.body.len()
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(&response.body)?;
    stream.flush()
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        409 => "Conflict",
        _ => "Error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_public_bind_without_a_token() {
        let public_addr: SocketAddr = "0.0.0.0:47822".parse().unwrap();
        assert!(validate_bind(public_addr, "").is_err());
        let loopback: SocketAddr = "127.0.0.1:47822".parse().unwrap();
        assert!(validate_bind(loopback, "").is_ok());
        assert!(validate_bind(public_addr, "secret").is_ok());
    }

    #[test]
    fn compares_tokens_without_accepting_a_prefix() {
        assert!(tokens_match("secret", "secret"));
        assert!(!tokens_match("secret", "secre"));
        assert!(!tokens_match("secret", "secret2"));
        assert!(!tokens_match("", "secret"));
        assert!(!tokens_match("secret", ""));
    }

    #[test]
    fn parse_prefers_the_environment_token() {
        let outcome = parse_args(
            &[
                "--bind".into(),
                "127.0.0.1:9".into(),
                "--token".into(),
                "from-argv".into(),
            ],
            None,
            Some("from-env".into()),
        )
        .unwrap();
        match outcome {
            ParseOutcome::Run(options) => {
                assert_eq!(options.bind, "127.0.0.1:9");
                assert_eq!(options.token, "from-env");
            }
            ParseOutcome::Help => panic!("expected options"),
        }
    }

    #[test]
    fn server_health_echo_and_confirmation() {
        if !std::path::Path::new("/dev/pts").exists() && !cfg!(windows) {
            return;
        }
        let running = start("127.0.0.1:0", "test-token".into()).expect("bind");
        let addr = running.addr;
        let (status, body) = http(addr, "GET", "/health", None, None);
        assert_eq!(status, 200, "{body}");
        assert!(body.contains("ok"));

        let (status, body) = http(addr, "GET", "/api/v1/status", None, None);
        assert_eq!(status, 401, "{body}");

        let (status, body) = http(addr, "GET", "/api/v1/status", Some("test-token"), None);
        assert_eq!(status, 200, "{body}");
        assert!(body.contains("\"mode\":\"server\""));
        assert!(body.contains(std::env::consts::OS));

        let (status, body) = http(
            addr,
            "POST",
            "/api/v1/terminals",
            Some("test-token"),
            Some(r#"{"id":"srv_a"}"#),
        );
        if status != 201 {
            if body.contains("Could not open a terminal") || body.contains("SHELL_UNAVAILABLE") {
                return;
            }
            panic!("create failed {status} {body}");
        }

        let (status, body) = http(
            addr,
            "POST",
            "/api/v1/terminals/srv_a/input",
            Some("test-token"),
            Some(r#"{"data":"echo Unit Agent\r"}"#),
        );
        assert_eq!(status, 200, "{body}");

        let start_time = std::time::Instant::now();
        let mut output = String::new();
        while start_time.elapsed() < Duration::from_secs(5) {
            let (status, body) = http(
                addr,
                "GET",
                "/api/v1/terminals/srv_a",
                Some("test-token"),
                None,
            );
            assert_eq!(status, 200, "{body}");
            output = body;
            if output.contains("Unit Agent") {
                break;
            }
            thread::sleep(Duration::from_millis(40));
        }
        assert!(output.contains("Unit Agent"), "echo output was {output:?}");

        let (status, body) = http(
            addr,
            "POST",
            "/api/v1/terminals/srv_a/input",
            Some("test-token"),
            Some(r#"{"data":"rm -rf /\r"}"#),
        );
        assert_eq!(status, 200, "{body}");
        assert!(body.contains("rm -rf /"), "{body}");
        assert!(body.contains("\"written\":false"), "{body}");

        let (status, _) = http(
            addr,
            "DELETE",
            "/api/v1/terminals/srv_a?force=true",
            Some("test-token"),
            None,
        );
        assert_eq!(status, 200);
    }

    fn http(
        addr: SocketAddr,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<&str>,
    ) -> (u16, String) {
        let mut stream = TcpStream::connect(addr).expect("connect");
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
        let body = body.unwrap_or("");
        let mut request = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Length: {}\r\n",
            body.len()
        );
        if let Some(token) = token {
            request.push_str(&format!("Authorization: Bearer {token}\r\n"));
        }
        if !body.is_empty() {
            request.push_str("Content-Type: application/json\r\n");
        }
        request.push_str("\r\n");
        request.push_str(body);
        stream.write_all(request.as_bytes()).unwrap();
        let mut buffer = String::new();
        let _ = stream.read_to_string(&mut buffer);
        let (head, response_body) = buffer.split_once("\r\n\r\n").unwrap_or(("", &buffer));
        let status = head
            .split_whitespace()
            .nth(1)
            .unwrap_or("0")
            .parse()
            .unwrap_or(0);
        (status, response_body.to_string())
    }
}
