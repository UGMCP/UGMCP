use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::safety::{GuardAction, LineGuard};
use crate::system::{detect_shell, has_child_processes, home_dir, shell_kind};
use crate::util::{lock, now_secs};

const TAIL_LIMIT: usize = 262_144;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub shell: String,
    pub pid: Option<u32>,
    pub running: bool,
    pub created_at: i64,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSnapshot {
    pub info: TerminalInfo,
    pub output: String,
    pub seq: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteResult {
    pub written: bool,
    pub confirmation: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTerminalRequest {
    pub id: Option<String>,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub shell: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TerminalEvent {
    Output { id: String, seq: u64, data: String },
    Exit { id: String, code: Option<i32> },
    Cwd { id: String, cwd: String },
    Confirm { id: String, command: String },
}

type Listener = Arc<dyn Fn(TerminalEvent) + Send + Sync>;

struct Tail {
    text: String,
    seq: u64,
}

struct Session {
    id: String,
    title: String,
    shell: String,
    cwd: Mutex<String>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    guard: Mutex<LineGuard>,
    tail: Mutex<Tail>,
    running: AtomicBool,
    created_at: i64,
    exit_code: Mutex<Option<i32>>,
    reader: Mutex<Option<JoinHandle<()>>>,
    scratch: PathBuf,
}

pub struct TerminalManager {
    sessions: Mutex<std::collections::HashMap<String, Arc<Session>>>,
    listener: Mutex<Option<Listener>>,
}

impl TerminalManager {
    pub fn new() -> Self {
        crate::system::reap_orphaned_shells();
        Self {
            sessions: Mutex::new(std::collections::HashMap::new()),
            listener: Mutex::new(None),
        }
    }

    pub fn set_listener(&self, listener: Listener) {
        *lock(&self.listener) = Some(listener);
    }

    pub fn create(&self, request: CreateTerminalRequest) -> Result<TerminalInfo, AppError> {
        let shell = detect_shell(request.shell.as_deref())?;
        let cwd = resolve_cwd(request.cwd.as_deref());
        if !Path::new(&cwd).is_dir() {
            return Err(AppError::terminal(format!(
                "Working directory does not exist: {cwd}"
            )));
        }
        let id = request
            .id
            .filter(|id| !id.trim().is_empty())
            .unwrap_or_else(|| format!("ws_{}", &uuid::Uuid::new_v4().simple().to_string()[..8]));
        if lock(&self.sessions).contains_key(&id) {
            return Err(AppError::terminal(
                "A workspace with that id already exists.",
            ));
        }
        let title = request.title.unwrap_or_else(|| "Terminal".into());
        let (cmd, scratch) = prepare_command(&shell, &id, &cwd)?;
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|err| AppError::pty(format!("Could not open a terminal: {err}")))?;
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|err| AppError::terminal(format!("Could not start the shell: {err}")))?;
        drop(pair.slave);
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|err| AppError::pty(format!("Could not read the terminal: {err}")))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|err| AppError::pty(format!("Could not write to the terminal: {err}")))?;
        let session = Arc::new(Session {
            id: id.clone(),
            title,
            shell,
            cwd: Mutex::new(cwd),
            master: Mutex::new(pair.master),
            writer: Mutex::new(writer),
            child: Mutex::new(child),
            guard: Mutex::new(LineGuard::default()),
            tail: Mutex::new(Tail {
                text: String::new(),
                seq: 0,
            }),
            running: AtomicBool::new(true),
            created_at: now_secs(),
            exit_code: Mutex::new(None),
            reader: Mutex::new(None),
            scratch,
        });
        let reader_session = session.clone();
        let listener = lock(&self.listener).clone();
        let handle = std::thread::spawn(move || read_loop(reader_session, reader, listener));
        *lock(&session.reader) = Some(handle);
        let info = session_info(&session);
        lock(&self.sessions).insert(id, session);
        Ok(info)
    }

    pub fn write(&self, id: &str, data: &str) -> Result<WriteResult, AppError> {
        let session = self.session(id)?;
        if !session.running.load(Ordering::SeqCst) {
            return Err(AppError::terminal("This terminal process has exited."));
        }
        let action = lock(&session.guard).push(data);
        match action {
            GuardAction::Forward(text) => {
                write_all(&session, &text)?;
                Ok(WriteResult {
                    written: true,
                    confirmation: None,
                })
            }
            GuardAction::Confirm { forward, command } => {
                if !forward.is_empty() {
                    write_all(&session, &forward)?;
                }
                self.emit(TerminalEvent::Confirm {
                    id: id.to_string(),
                    command: command.clone(),
                });
                Ok(WriteResult {
                    written: false,
                    confirmation: Some(command),
                })
            }
            GuardAction::Held => Ok(WriteResult {
                written: false,
                confirmation: lock(&session.guard).pending().map(str::to_string),
            }),
        }
    }

    pub fn confirm(&self, id: &str, approve: bool) -> Result<(), AppError> {
        let session = self.session(id)?;
        let data = if approve {
            lock(&session.guard).approve()
        } else {
            lock(&session.guard).deny()
        };
        if let Some(data) = data {
            write_all(&session, &data)?;
        }
        Ok(())
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), AppError> {
        let session = self.session(id)?;
        let cols = cols.max(2);
        let rows = rows.max(1);
        lock(&session.master)
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|err| AppError::pty(format!("Could not resize the terminal: {err}")))?;
        Ok(())
    }

    pub fn close(&self, id: &str, force: bool) -> Result<(), AppError> {
        let session = {
            let sessions = lock(&self.sessions);
            sessions
                .get(id)
                .cloned()
                .ok_or_else(|| AppError::not_found("Terminal session not found."))?
        };
        if !force && session.running.load(Ordering::SeqCst) {
            if let Some(pid) = lock(&session.child).process_id() {
                if has_child_processes(pid) {
                    return Err(AppError::new(
                        "NEEDS_CONFIRM",
                        "Terminal process is still running. Close workspace?",
                    ));
                }
            }
        }
        stop_session(&session);
        lock(&self.sessions).remove(id);
        Ok(())
    }

    pub fn restart(&self, id: &str) -> Result<TerminalInfo, AppError> {
        let session = self.session(id)?;
        let title = session.title.clone();
        let shell = session.shell.clone();
        let cwd = lock(&session.cwd).clone();
        drop(session);
        self.close(id, true)?;
        self.create(CreateTerminalRequest {
            id: Some(id.to_string()),
            title: Some(title),
            cwd: Some(cwd),
            shell: Some(shell),
        })
    }

    pub fn list(&self) -> Vec<TerminalInfo> {
        lock(&self.sessions)
            .values()
            .map(|session| session_info(session))
            .collect()
    }

    pub fn snapshot(&self, id: &str) -> Result<TerminalSnapshot, AppError> {
        let session = self.session(id)?;
        let tail = lock(&session.tail);
        Ok(TerminalSnapshot {
            info: session_info(&session),
            output: tail.text.clone(),
            seq: tail.seq,
        })
    }

    pub fn set_cwd(&self, id: &str, cwd: String) -> Result<(), AppError> {
        let session = self.session(id)?;
        if Path::new(&cwd).is_dir() {
            *lock(&session.cwd) = cwd;
        }
        Ok(())
    }

    pub fn shutdown(&self) {
        let sessions: Vec<_> = lock(&self.sessions)
            .drain()
            .map(|(_, session)| session)
            .collect();
        for session in sessions {
            stop_session(&session);
        }
    }

    fn session(&self, id: &str) -> Result<Arc<Session>, AppError> {
        lock(&self.sessions)
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::not_found("Terminal session not found."))
    }

    fn emit(&self, event: TerminalEvent) {
        if let Some(listener) = lock(&self.listener).clone() {
            listener(event);
        }
    }
}

impl Drop for TerminalManager {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn session_info(session: &Session) -> TerminalInfo {
    TerminalInfo {
        id: session.id.clone(),
        title: session.title.clone(),
        cwd: lock(&session.cwd).clone(),
        shell: session.shell.clone(),
        pid: lock(&session.child).process_id(),
        running: session.running.load(Ordering::SeqCst),
        created_at: session.created_at,
        exit_code: *lock(&session.exit_code),
    }
}

fn write_all(session: &Session, data: &str) -> Result<(), AppError> {
    if data.is_empty() {
        return Ok(());
    }
    let mut writer = lock(&session.writer);
    writer
        .write_all(data.as_bytes())
        .map_err(|err| AppError::pty(format!("Could not write to the terminal: {err}")))?;
    writer
        .flush()
        .map_err(|err| AppError::pty(format!("Could not write to the terminal: {err}")))?;
    Ok(())
}

fn stop_session(session: &Session) {
    session.running.store(false, Ordering::SeqCst);
    #[cfg(unix)]
    if let Some(pid) = lock(&session.child).process_id() {
        unsafe {
            libc::kill(pid as i32, libc::SIGHUP);
        }
    }
    let _ = lock(&session.child).kill();
    if let Some(handle) = lock(&session.reader).take() {
        let _ = handle.join();
    }
    let _ = fs::remove_dir_all(&session.scratch);
}

fn read_loop(session: Arc<Session>, mut reader: Box<dyn Read + Send>, listener: Option<Listener>) {
    let mut buf = [0u8; 8192];
    let mut carry = Vec::new();
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let text = decode_utf8(&mut carry, &buf[..n]);
                if text.is_empty() {
                    continue;
                }
                if let Some(cwd) = extract_osc7(&text) {
                    *lock(&session.cwd) = cwd.clone();
                    emit(
                        &listener,
                        TerminalEvent::Cwd {
                            id: session.id.clone(),
                            cwd,
                        },
                    );
                }
                let seq = {
                    let mut tail = lock(&session.tail);
                    tail.seq += 1;
                    tail.text.push_str(&text);
                    if tail.text.len() > TAIL_LIMIT {
                        let mut cut = tail.text.len() - TAIL_LIMIT;
                        while cut < tail.text.len() && !tail.text.is_char_boundary(cut) {
                            cut += 1;
                        }
                        tail.text.drain(..cut);
                    }
                    tail.seq
                };
                emit(
                    &listener,
                    TerminalEvent::Output {
                        id: session.id.clone(),
                        seq,
                        data: text,
                    },
                );
            }
            Err(_) => break,
        }
    }
    session.running.store(false, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(20));
    let code = lock(&session.child)
        .try_wait()
        .ok()
        .flatten()
        .map(|status| status.exit_code() as i32);
    *lock(&session.exit_code) = code;
    emit(
        &listener,
        TerminalEvent::Exit {
            id: session.id.clone(),
            code,
        },
    );
}

fn emit(listener: &Option<Listener>, event: TerminalEvent) {
    if let Some(listener) = listener {
        listener(event);
    }
}

fn decode_utf8(carry: &mut Vec<u8>, chunk: &[u8]) -> String {
    carry.extend_from_slice(chunk);
    match String::from_utf8(carry.clone()) {
        Ok(text) => {
            carry.clear();
            text
        }
        Err(err) => {
            let valid = err.utf8_error().valid_up_to();
            let text = String::from_utf8_lossy(&carry[..valid]).into_owned();
            let rest = carry[valid..].to_vec();
            *carry = rest;
            if carry.len() > 8 {
                carry.drain(..1);
            }
            text
        }
    }
}

fn extract_osc7(data: &str) -> Option<String> {
    let marker = "\u{1b}]7;";
    let rest = data.split(marker).nth(1)?;
    let end = rest.find('\u{7}').or_else(|| rest.find("\u{1b}\\"))?;
    let payload = &rest[..end];
    let after_scheme = payload.split("://").nth(1)?;
    let path = after_scheme.find('/').map(|index| &after_scheme[index..])?;
    normalize_osc_path(&percent_decode(path))
}

fn normalize_osc_path(decoded: &str) -> Option<String> {
    if !decoded.starts_with('/') {
        return None;
    }
    #[cfg(windows)]
    {
        let bytes = decoded.as_bytes();
        if bytes.len() >= 3 && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
            let rest = decoded[3..].replace('/', "\\");
            return Some(format!("{}:{rest}", decoded[1..2].to_ascii_uppercase()));
        }
    }
    Some(decoded.to_string())
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(value) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn resolve_cwd(requested: Option<&str>) -> String {
    if let Some(cwd) = requested.map(str::trim).filter(|s| !s.is_empty()) {
        let expanded = expand_home(cwd);
        if Path::new(&expanded).is_dir() {
            return expanded;
        }
    }
    let home = home_dir();
    if Path::new(&home).is_dir() {
        home
    } else {
        std::env::temp_dir().to_string_lossy().into_owned()
    }
}

fn expand_home(cwd: &str) -> String {
    let home = home_dir();
    if cwd == "~" {
        return home;
    }
    if let Some(rest) = cwd.strip_prefix("~/").or_else(|| cwd.strip_prefix("~\\")) {
        return Path::new(&home).join(rest).to_string_lossy().into_owned();
    }
    cwd.to_string()
}

fn prepare_command(
    shell: &str,
    id: &str,
    cwd: &str,
) -> Result<(CommandBuilder, PathBuf), AppError> {
    let scratch = std::env::temp_dir().join(format!("unit-agent-{id}"));
    fs::create_dir_all(&scratch).map_err(|err| AppError::terminal(err.to_string()))?;
    let mut cmd = CommandBuilder::new(shell);
    cmd.cwd(cwd);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("UNIT_AGENT", "1");
    cmd.env("UNIT_AGENT_WS", id);
    match shell_kind(shell) {
        "bash" => {
            let rc = scratch.join("bashrc");
            fs::write(&rc, BASH_RC).map_err(|err| AppError::terminal(err.to_string()))?;
            cmd.arg("--rcfile");
            cmd.arg(&rc);
            cmd.arg("-i");
        }
        "zsh" => {
            fs::write(scratch.join(".zshenv"), ZSH_ENV)
                .map_err(|err| AppError::terminal(err.to_string()))?;
            fs::write(scratch.join(".zshrc"), ZSH_RC)
                .map_err(|err| AppError::terminal(err.to_string()))?;
            cmd.env("ZDOTDIR", scratch.as_os_str());
            cmd.arg("-i");
        }
        "fish" => {
            cmd.arg("-i");
            cmd.arg("-C");
            cmd.arg(FISH_INIT);
        }
        "powershell" => {
            cmd.arg("-NoLogo");
            cmd.arg("-NoExit");
            cmd.arg("-Command");
            cmd.arg(POWERSHELL_INIT);
        }
        "cmd" => {
            cmd.arg("/Q");
            cmd.arg("/K");
        }
        _ => {
            #[cfg(unix)]
            cmd.arg("-i");
        }
    }
    Ok((cmd, scratch))
}

const BASH_RC: &str = r#"
[ -f /etc/bash.bashrc ] && . /etc/bash.bashrc
[ -f "$HOME/.bashrc" ] && . "$HOME/.bashrc"
__unit_agent_cwd() { printf '\033]7;file://%s%s\a' "${HOSTNAME:-localhost}" "$PWD"; }
case ";${PROMPT_COMMAND:-};" in
  *";__unit_agent_cwd;"*) ;;
  *) PROMPT_COMMAND="__unit_agent_cwd${PROMPT_COMMAND:+;$PROMPT_COMMAND}" ;;
esac
"#;

const ZSH_ENV: &str = r#"
[ -f "$HOME/.zshenv" ] && . "$HOME/.zshenv"
"#;

const ZSH_RC: &str = r#"
[ -f "$HOME/.zshrc" ] && . "$HOME/.zshrc"
__unit_agent_cwd() { printf '\033]7;file://%s%s\a' "${HOST:-localhost}" "$PWD"; }
autoload -Uz add-zsh-hook 2>/dev/null
add-zsh-hook precmd __unit_agent_cwd 2>/dev/null
"#;

const FISH_INIT: &str = r#"function __unit_agent_cwd --on-variable PWD; printf '\033]7;file://%s%s\a' (hostname) "$PWD"; end"#;

const POWERSHELL_INIT: &str = r#"
function global:prompt {
  $path = (Get-Location).Path -replace '\\','/'
  if ($path -match '^[A-Za-z]:') { $uri = "file://localhost/$path" } else { $uri = "file://localhost$path" }
  $esc = [char]27
  Write-Host -NoNewline ($esc + ']7;' + $uri + [char]7)
  'PS ' + (Get-Location).Path + '> '
}
"#;

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    fn wait_for(manager: &TerminalManager, id: &str, needle: &str) -> String {
        let start = Instant::now();
        loop {
            let output = manager.snapshot(id).map(|s| s.output).unwrap_or_default();
            if output.contains(needle) || start.elapsed() > Duration::from_secs(5) {
                return output;
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    }

    #[test]
    fn osc7_extracts_unix_and_windows_paths() {
        let unix = "\u{1b}]7;file://host/home/ubuntu\u{7}";
        assert_eq!(extract_osc7(unix).as_deref(), Some("/home/ubuntu"));
        let win = "\u{1b}]7;file://localhost/C:/Users/ada\u{7}";
        let path = extract_osc7(win).unwrap();
        if cfg!(windows) {
            assert_eq!(path, "C:\\Users\\ada");
        } else {
            assert_eq!(path, "/C:/Users/ada");
        }
    }

    #[test]
    fn echo_pwd_whoami_resize_and_independent_sessions() {
        if !Path::new("/dev/pts").exists() {
            return;
        }
        let shell = detect_shell(Some("/bin/bash")).expect("bash");
        let manager = TerminalManager::new();
        let cwd = home_dir();
        let first = manager
            .create(CreateTerminalRequest {
                id: Some("ws_test_a".into()),
                title: Some("Terminal".into()),
                cwd: Some(cwd.clone()),
                shell: Some(shell.clone()),
            })
            .expect("create a");
        manager.write(&first.id, "echo Unit Agent\r").unwrap();
        let output = wait_for(&manager, &first.id, "Unit Agent");
        assert!(output.contains("Unit Agent"), "echo output was {output:?}");

        manager.write(&first.id, "pwd\r").unwrap();
        let output = wait_for(&manager, &first.id, cwd.trim_end_matches('/'));
        assert!(
            output.contains(cwd.trim_end_matches('/')),
            "pwd output was {output:?}"
        );

        let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
        manager.write(&first.id, "whoami\r").unwrap();
        let output = wait_for(&manager, &first.id, &user);
        assert!(output.contains(&user), "whoami output was {output:?}");

        manager.resize(&first.id, 40, 12).unwrap();
        if Path::new("/usr/bin/stty").exists() || Path::new("/bin/stty").exists() {
            manager.write(&first.id, "stty size\r").unwrap();
            let output = wait_for(&manager, &first.id, "12 40");
            assert!(output.contains("12 40"), "stty output was {output:?}");
        }

        let second = manager
            .create(CreateTerminalRequest {
                id: Some("ws_test_b".into()),
                title: Some("Terminal".into()),
                cwd: Some("/tmp".into()),
                shell: Some(shell),
            })
            .expect("create b");
        manager.write(&second.id, "echo OTHER\r").unwrap();
        let other = wait_for(&manager, &second.id, "OTHER");
        assert!(other.contains("OTHER"));
        assert!(
            !manager
                .snapshot(&first.id)
                .unwrap()
                .output
                .contains("echo OTHER\rOTHER")
                || first.id != second.id
        );

        manager.close(&second.id, true).unwrap();
        assert!(manager.list().iter().all(|info| info.id != second.id));
        manager.write(&first.id, "echo STILL\r").unwrap();
        let still = wait_for(&manager, &first.id, "STILL");
        assert!(still.contains("STILL"));
        manager.close(&first.id, true).unwrap();
        assert!(manager.list().is_empty());
    }

    #[test]
    fn osc7_extracts_path() {
        let data = "\u{1b}]7;file://host/home/ada%20code\u{7}";
        assert_eq!(extract_osc7(data).as_deref(), Some("/home/ada code"));
    }
}
