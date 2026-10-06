use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};

use crate::auth::AuthService;
use crate::config::PryselConfig;
use crate::connection::ConnectionService;
use crate::safety::is_dangerous;
use crate::storage::{SavedWorkspace, Storage};
use crate::system::host_identity;
use crate::terminal::{CreateTerminalRequest, TerminalInfo, TerminalManager};
use crate::util::{lock, now_secs};

pub const CONTROL_API_VERSION: &str = "1";
pub const MCP_PROTOCOL: &str = "2024-11-05";
pub const MCP_ADDR: &str = "127.0.0.1:47823";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    ReadOnly,
    Safe,
    Confirm,
    Full,
}

impl Level {
    pub fn parse(text: &str) -> Self {
        match text.trim().to_ascii_lowercase().as_str() {
            "read-only" | "readonly" | "read_only" => Self::ReadOnly,
            "safe" => Self::Safe,
            "full" | "full-control" | "full_control" => Self::Full,
            _ => Self::Confirm,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::Safe => "safe",
            Self::Confirm => "confirm",
            Self::Full => "full-control",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Effect {
    Read,
    Develop,
    Mutate,
    Dangerous,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ControlEvent {
    Workspace {
        info: TerminalInfo,
        focus: bool,
    },
    Confirm {
        id: String,
        client: String,
        action: String,
        workspace: String,
        command: String,
    },
    Activity {
        entry: ActivityEntry,
    },
    Status,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEntry {
    pub at: i64,
    pub client: String,
    pub action: String,
    pub workspace: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiClient {
    pub session_id: String,
    pub client: String,
    pub connected_at: i64,
    pub permission: String,
    pub paused: bool,
}

struct Session {
    client: String,
    connected_at: i64,
    paused: bool,
}

struct AuditRow {
    at: i64,
    client: String,
    session: String,
    tool: String,
    workspace: String,
    action: String,
    permission: String,
    result: String,
}

type Listener = Arc<dyn Fn(ControlEvent) + Send + Sync>;

pub struct ControlHub {
    pub terminals: Arc<TerminalManager>,
    storage: Arc<Storage>,
    auth: Arc<AuthService>,
    connection: Arc<ConnectionService>,
    config: PryselConfig,
    level: Mutex<Level>,
    emergency: AtomicBool,
    sessions: Mutex<HashMap<String, Session>>,
    owners: Mutex<HashMap<String, String>>,
    locks: Mutex<HashMap<String, String>>,
    history: Mutex<VecDeque<Value>>,
    activity: Mutex<VecDeque<ActivityEntry>>,
    audit: Mutex<VecDeque<AuditRow>>,
    pending: Mutex<HashMap<String, mpsc::Sender<bool>>>,
    listener: Mutex<Option<Listener>>,
    mcp_enabled: AtomicBool,
}

impl ControlHub {
    pub fn new(
        terminals: Arc<TerminalManager>,
        storage: Arc<Storage>,
        auth: Arc<AuthService>,
        connection: Arc<ConnectionService>,
        config: PryselConfig,
    ) -> Self {
        let level = Level::parse(&storage.settings().ai_permission);
        let mcp_enabled = storage.settings().ai_mcp;
        Self {
            terminals,
            storage,
            auth,
            connection,
            config,
            level: Mutex::new(level),
            emergency: AtomicBool::new(false),
            sessions: Mutex::new(HashMap::new()),
            owners: Mutex::new(HashMap::new()),
            locks: Mutex::new(HashMap::new()),
            history: Mutex::new(VecDeque::new()),
            activity: Mutex::new(VecDeque::new()),
            audit: Mutex::new(VecDeque::new()),
            pending: Mutex::new(HashMap::new()),
            listener: Mutex::new(None),
            mcp_enabled: AtomicBool::new(mcp_enabled),
        }
    }

    pub fn set_listener(&self, listener: Listener) {
        *lock(&self.listener) = Some(listener);
    }

    pub fn set_level(&self, level: Level) {
        *lock(&self.level) = level;
    }

    pub fn set_mcp_enabled(&self, enabled: bool) {
        self.mcp_enabled.store(enabled, Ordering::SeqCst);
        self.emit(ControlEvent::Status);
    }

    pub fn level(&self) -> Level {
        *lock(&self.level)
    }

    pub fn open_session(&self, client: &str) -> String {
        let id = format!(
            "session_{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        lock(&self.sessions).insert(
            id.clone(),
            Session {
                client: if client.trim().is_empty() {
                    "MCP".into()
                } else {
                    client.trim().to_string()
                },
                connected_at: now_secs(),
                paused: false,
            },
        );
        self.emit(ControlEvent::Status);
        id
    }

    pub fn disconnect_session(&self, id: &str) {
        lock(&self.sessions).remove(id);
        lock(&self.locks).retain(|_, owner| owner != id);
        self.emit(ControlEvent::Status);
    }

    pub fn clients(&self) -> Vec<AiClient> {
        let level = self.level().as_str().to_string();
        lock(&self.sessions)
            .iter()
            .map(|(id, session)| AiClient {
                session_id: id.clone(),
                client: session.client.clone(),
                connected_at: session.connected_at,
                permission: level.clone(),
                paused: session.paused,
            })
            .collect()
    }

    pub fn emergency_stop(&self) {
        self.emergency.store(true, Ordering::SeqCst);
        let ids: Vec<String> = lock(&self.sessions).keys().cloned().collect();
        for id in ids {
            self.disconnect_session(&id);
        }
        self.note("user", "", "emergency stop", "", "stopped");
        self.emit(ControlEvent::Status);
    }

    pub fn clear_emergency(&self) {
        self.emergency.store(false, Ordering::SeqCst);
        self.emit(ControlEvent::Status);
    }

    pub fn resolve_confirm(&self, id: &str, allow: bool) {
        if let Some(tx) = lock(&self.pending).remove(id) {
            let _ = tx.send(allow);
        }
    }

    pub fn status_value(&self) -> Value {
        let session = self
            .auth
            .restore()
            .unwrap_or_else(|_| crate::auth::SessionView::none());
        let host = host_identity();
        json!({
            "name": "Unit Agent",
            "version": env!("CARGO_PKG_VERSION"),
            "controlApi": CONTROL_API_VERSION,
            "mcp": MCP_PROTOCOL,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "connection": self.connection.snapshot(),
            "authenticated": session.authenticated,
            "mcpRunning": self.mcp_enabled.load(Ordering::SeqCst),
            "mcpEndpoint": MCP_ADDR,
            "remote": false,
            "permission": self.level().as_str(),
            "emergencyStop": self.emergency.load(Ordering::SeqCst),
            "workspaces": self.terminals.list().len(),
            "clients": self.clients(),
            "computerName": host.computer_name,
            "serverName": host.server_name,
            "network": host.network,
        })
    }

    pub fn activity(&self) -> Vec<ActivityEntry> {
        lock(&self.activity).iter().cloned().collect()
    }

    pub fn call_tool(&self, session_id: &str, name: &str, args: &Value) -> Value {
        if !self.mcp_enabled.load(Ordering::SeqCst) && name != "unit_agent_mcp_start" {
            return fail(
                "MCP_DISABLED",
                "MCP control is turned off. Terminals keep running.",
            );
        }
        let client = self
            .clients()
            .into_iter()
            .find(|item| item.session_id == session_id)
            .map(|item| item.client)
            .unwrap_or_else(|| "MCP".into());
        if lock(&self.sessions)
            .get(session_id)
            .is_some_and(|s| s.paused)
            && !matches!(
                name,
                "unit_agent_ai_session_resume" | "unit_agent_app_status"
            )
        {
            return fail("AI_PAUSED", "This AI session is paused.");
        }
        let effect = effect_for(name, args);
        let writes_shell = matches!(
            name,
            "unit_agent_execute"
                | "unit_agent_execute_background"
                | "unit_agent_terminal_write"
                | "unit_agent_dev_start"
                | "unit_agent_build"
                | "unit_agent_test"
                | "unit_agent_lint"
        );
        if self.emergency.load(Ordering::SeqCst) && (effect != Effect::Read || writes_shell) {
            return fail(
                "AI_DISABLED",
                "AI control is stopped. The terminal is still available.",
            );
        }
        let level = self.level();
        match authorize(level, effect) {
            Decision::Deny => {
                self.audit(session_id, &client, name, args, "denied");
                return fail(
                    "PERMISSION_DENIED",
                    "This permission level does not allow that action.",
                );
            }
            Decision::Confirm => {
                let command = action_label(name, args);
                let workspace = args
                    .get("workspace_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if !self.ask(&client, name, &workspace, &command) {
                    self.audit(session_id, &client, name, args, "denied");
                    return fail("CONFIRMATION_REQUIRED", "The user denied this action.");
                }
            }
            Decision::Allow => {}
        }
        if let Some(busy) = self.busy(session_id, name, args) {
            return busy;
        }
        let result = dispatch(self, session_id, &client, name, args);
        let ok = result
            .get("success")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.audit(
            session_id,
            &client,
            name,
            args,
            if ok { "success" } else { "error" },
        );
        if effect != Effect::Read {
            self.note(
                &client,
                args.get("workspace_id")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                &action_label(name, args),
                "",
                if ok { "success" } else { "error" },
            );
        }
        result
    }

    fn ask(&self, client: &str, action: &str, workspace: &str, command: &str) -> bool {
        let id = format!(
            "confirm_{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        let (tx, rx) = mpsc::channel();
        lock(&self.pending).insert(id.clone(), tx);
        self.emit(ControlEvent::Confirm {
            id: id.clone(),
            client: client.to_string(),
            action: action.to_string(),
            workspace: workspace.to_string(),
            command: redact(command),
        });
        let allowed = rx.recv_timeout(Duration::from_secs(45)).unwrap_or(false);
        lock(&self.pending).remove(&id);
        allowed
    }

    fn busy(&self, session_id: &str, name: &str, args: &Value) -> Option<Value> {
        let workspace = args.get("workspace_id").and_then(Value::as_str)?;
        if matches!(
            name,
            "unit_agent_workspace_list"
                | "unit_agent_workspace_status"
                | "unit_agent_terminal_read"
                | "unit_agent_terminal_status"
        ) {
            return None;
        }
        let locks = lock(&self.locks);
        if let Some(owner) = locks.get(workspace) {
            if owner != session_id {
                return Some(fail(
                    "WORKSPACE_BUSY",
                    "Another AI session is using this workspace.",
                ));
            }
        }
        None
    }

    fn emit(&self, event: ControlEvent) {
        if let Some(listener) = lock(&self.listener).clone() {
            listener(event);
        }
    }

    fn note(&self, client: &str, workspace: &str, action: &str, _detail: &str, status: &str) {
        let entry = ActivityEntry {
            at: now_secs(),
            client: client.to_string(),
            action: redact(action),
            workspace: workspace.to_string(),
            status: status.to_string(),
        };
        let mut activity = lock(&self.activity);
        activity.push_back(entry.clone());
        while activity.len() > 80 {
            activity.pop_front();
        }
        drop(activity);
        self.emit(ControlEvent::Activity { entry });
    }

    fn audit(&self, session: &str, client: &str, tool: &str, args: &Value, result: &str) {
        let workspace = args
            .get("workspace_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let action = action_label(tool, args);
        let row = AuditRow {
            at: now_secs(),
            client: client.to_string(),
            session: session.to_string(),
            tool: tool.to_string(),
            workspace,
            action: redact(&action),
            permission: self.level().as_str().to_string(),
            result: result.to_string(),
        };
        let mut audit = lock(&self.audit);
        audit.push_back(row);
        while audit.len() > 200 {
            audit.pop_front();
        }
    }

    pub fn audit_list(&self) -> Value {
        let rows: Vec<Value> = lock(&self.audit)
            .iter()
            .map(|row| {
                json!({
                    "timestamp": row.at,
                    "client": row.client,
                    "session": row.session,
                    "tool": row.tool,
                    "workspace": row.workspace,
                    "action": row.action,
                    "permission": row.permission,
                    "result": row.result,
                })
            })
            .collect();
        json!({ "success": true, "records": rows })
    }
}

fn authorize(level: Level, effect: Effect) -> Decision {
    match (level, effect) {
        (Level::ReadOnly, Effect::Read) => Decision::Allow,
        (Level::ReadOnly, _) => Decision::Deny,
        (Level::Safe, Effect::Read | Effect::Develop) => Decision::Allow,
        (Level::Safe, _) => Decision::Confirm,
        (Level::Confirm, Effect::Read) => Decision::Allow,
        (Level::Confirm, _) => Decision::Confirm,
        (Level::Full, Effect::Dangerous) => Decision::Confirm,
        (Level::Full, _) => Decision::Allow,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    Allow,
    Confirm,
    Deny,
}

fn fail(code: &str, message: &str) -> Value {
    json!({ "success": false, "code": code, "message": message })
}

fn ok(extra: Value) -> Value {
    let mut value = json!({ "success": true });
    if let (Some(map), Some(extra)) = (value.as_object_mut(), extra.as_object()) {
        for (key, item) in extra {
            map.insert(key.clone(), item.clone());
        }
    }
    value
}

pub fn tool_catalog() -> Vec<Value> {
    TOOLS
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": serde_json::from_str::<Value>(tool.schema).unwrap_or_else(|_| json!({"type":"object"})),
                "permission": match tool.effect {
                    Effect::Read => "read",
                    Effect::Develop => "safe",
                    Effect::Mutate => "confirm",
                    Effect::Dangerous => "confirm",
                },
                "sideEffects": !matches!(tool.effect, Effect::Read),
                "confirmation": !matches!(tool.effect, Effect::Read | Effect::Develop),
            })
        })
        .collect()
}

struct Tool {
    name: &'static str,
    description: &'static str,
    effect: Effect,
    schema: &'static str,
}

const OBJ: &str = r#"{"type":"object","additionalProperties":true}"#;

const TOOLS: &[Tool] = &[
    Tool { name: "unit_agent_app_status", description: "Application name, version, connection, auth, MCP, workspaces, and connected AI clients.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_status", description: "Same snapshot as unit_agent_app_status, for a reconnecting AI.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_app_version", description: "Unit Agent, control API, and MCP protocol versions.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_app_get_settings", description: "Safe settings. Secrets are omitted.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_health", description: "Health of the app, MCP, terminals, and Prysel reachability.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_diagnostics", description: "Safe local diagnostics. Does not require the network.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_mcp_status", description: "Local MCP listener, permission, and connected clients.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_mcp_clients", description: "Connected AI clients.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_mcp_start", description: "Enable the local MCP listener flag. Does not bind a public address.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_mcp_stop", description: "Stop accepting AI tool calls. Existing terminals keep running.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_mcp_disconnect", description: "Disconnect one AI session. User processes keep running.", effect: Effect::Mutate, schema: r#"{"type":"object","properties":{"session_id":{"type":"string"}}}"# },
    Tool { name: "unit_agent_ai_sessions", description: "List AI sessions.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_ai_session_status", description: "State of one AI session.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_ai_session_permissions", description: "Current permission level.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_ai_session_disconnect", description: "Disconnect this AI session without killing terminals.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_ai_session_pause", description: "Pause AI actions for this session.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_ai_session_resume", description: "Resume a paused AI session.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_ai_emergency_stop", description: "User stop for new AI actions. Terminals stay up.", effect: Effect::Dangerous, schema: OBJ },
    Tool { name: "unit_agent_ai_activity", description: "Recent AI actions visible to the user.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_workspace_list", description: "Workspaces and their live PTY sessions.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_workspace_create", description: "Create a workspace PTY. It appears in the desktop.", effect: Effect::Mutate, schema: r#"{"type":"object","properties":{"name":{"type":"string"},"directory":{"type":"string"},"shell":{"type":"string"},"columns":{"type":"integer"},"rows":{"type":"integer"}}}"# },
    Tool { name: "unit_agent_workspace_close", description: "Close a workspace PTY. Does not delete project files.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_workspace_delete", description: "Remove saved workspace metadata. Does not delete project files.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_workspace_status", description: "Directory, shell, pid, and owner of a workspace.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_workspace_focus", description: "Ask the desktop to focus a workspace.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_workspace_rename", description: "Rename workspace metadata.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_workspace_set_directory", description: "Record a workspace directory and cd in its PTY.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_workspace_lock", description: "Lock a workspace to this AI session.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_workspace_unlock", description: "Unlock a workspace.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_terminal_read", description: "Read terminal output with offset and max_bytes.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_terminal_write", description: "Write bytes to the persistent PTY.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_terminal_resize", description: "Resize the PTY.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_terminal_status", description: "Pid, shell, cwd, and running state.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_process_list", description: "Pids for Unit Agent workspace shells. Does not list unrelated system processes.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_shell_info", description: "Detected shell path.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_logs_read", description: "Recent Unit Agent audit records. Secrets are redacted.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_terminal_interrupt", description: "Send Ctrl+C to the PTY.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_terminal_eof", description: "Send Ctrl+D to the PTY.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_terminal_restart", description: "Restart the workspace shell.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_execute", description: "Run a command in the workspace's persistent PTY. Later commands see the same shell state.", effect: Effect::Develop, schema: r#"{"type":"object","properties":{"workspace_id":{"type":"string"},"command":{"type":"string"},"timeout":{"type":"integer"}},"required":["command"]}"# },
    Tool { name: "unit_agent_execute_background", description: "Start a command in the PTY and return without waiting.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_execute_cancel", description: "Send Ctrl+C to the workspace PTY.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_execute_history", description: "Recent AI commands with secrets redacted.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_fs_list", description: "List a directory inside the workspace root.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_fs_stat", description: "File type, size, and modified time.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_fs_exists", description: "Whether a path exists inside the workspace.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_fs_read", description: "Read a text file with offset and limit. Secret files are refused.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_fs_write", description: "Write a file inside the workspace root.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_fs_append", description: "Append to a file inside the workspace root.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_fs_create_directory", description: "Create a directory inside the workspace root.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_fs_delete", description: "Delete a file inside the workspace root. Requires confirmation.", effect: Effect::Dangerous, schema: OBJ },
    Tool { name: "unit_agent_file_replace", description: "Replace an exact text snippet in a workspace file.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_search", description: "Search project text, skipping dependencies and build output.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_git_status", description: "git status in the workspace PTY.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_git_diff", description: "git diff in the workspace PTY.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_git_log", description: "Recent git log in the workspace PTY.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_git_branch_list", description: "git branch in the workspace PTY.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_git_remote", description: "git remote -v without credentials.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_git_commit", description: "git commit. Requires confirmation.", effect: Effect::Dangerous, schema: OBJ },
    Tool { name: "unit_agent_git_push", description: "git push. Requires confirmation.", effect: Effect::Dangerous, schema: OBJ },
    Tool { name: "unit_agent_project_detect", description: "Detect languages and frameworks in a directory.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_project_info", description: "Project name, directory, and detected tools.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_package_manager_detect", description: "Detect npm, cargo, pip, and other manifests.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_build", description: "Run the detected build command in the PTY.", effect: Effect::Develop, schema: OBJ },
    Tool { name: "unit_agent_test", description: "Run the detected test command in the PTY.", effect: Effect::Develop, schema: OBJ },
    Tool { name: "unit_agent_lint", description: "Run the detected lint command in the PTY.", effect: Effect::Develop, schema: OBJ },
    Tool { name: "unit_agent_dev_start", description: "Start the detected dev server in the PTY and return immediately.", effect: Effect::Mutate, schema: OBJ },
    Tool { name: "unit_agent_system_info", description: "OS, architecture, hostname, and shell. No secrets.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_cpu_status", description: "Load average and CPU count when the OS exposes them.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_memory_status", description: "Memory totals when the OS exposes them.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_disk_status", description: "Disk space for the workspace filesystem.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_network_status", description: "Hostname and the default-route interface.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_internet_status", description: "Whether the internet probe is online. Local tools do not depend on it.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_prysel_status", description: "Prysel reachability and whether auth is configured. No tokens.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_auth_status", description: "Whether a Prysel profile is stored. No tokens.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_user_profile", description: "Name, username, and email from the stored profile.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_port_check", description: "Whether a TCP port on loopback accepts connections.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_docker_status", description: "docker version when the docker binary exists.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_docker_ps", description: "docker ps when docker exists.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_audit_list", description: "Recent AI audit records with secrets removed.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_clipboard_read", description: "Refused unless the user enables clipboard access.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_screenshot", description: "Refused unless the user enables screenshot access.", effect: Effect::Read, schema: OBJ },
    Tool { name: "unit_agent_notification", description: "Ask the desktop to show a short notice.", effect: Effect::Mutate, schema: OBJ },
];

fn effect_for(name: &str, args: &Value) -> Effect {
    let listed = TOOLS
        .iter()
        .find(|tool| tool.name == name)
        .map(|tool| tool.effect);
    let Some(effect) = listed else {
        return Effect::Mutate;
    };
    if !matches!(
        name,
        "unit_agent_execute" | "unit_agent_execute_background" | "unit_agent_terminal_write"
    ) {
        return effect;
    }
    let command = args
        .get("command")
        .or_else(|| args.get("data"))
        .and_then(Value::as_str)
        .unwrap_or("");
    command_effect(command)
}

fn command_effect(command: &str) -> Effect {
    let line = command.trim();
    if line.is_empty() {
        return Effect::Read;
    }
    if is_dangerous(line) || starts_with_sudo(line) {
        return Effect::Dangerous;
    }
    let lower = line.to_ascii_lowercase();
    if lower.starts_with("git ") {
        return git_effect(&lower);
    }
    if lower.starts_with("docker ") || lower.starts_with("podman ") {
        if lower.contains(" rm")
            || lower.contains(" rmi")
            || lower.contains("prune")
            || lower.contains("kill")
        {
            return Effect::Dangerous;
        }
        return Effect::Mutate;
    }
    let word = lower.split_whitespace().next().unwrap_or("");
    if matches!(
        word,
        "pwd"
            | "ls"
            | "echo"
            | "printf"
            | "whoami"
            | "id"
            | "date"
            | "uname"
            | "cat"
            | "head"
            | "tail"
            | "wc"
            | "true"
            | "false"
            | "which"
            | "type"
            | "hostname"
            | "printenv"
    ) {
        return Effect::Read;
    }
    if lower.starts_with("npm test")
        || lower.starts_with("npm run test")
        || lower.starts_with("npm run build")
        || lower.starts_with("npm run lint")
        || lower.starts_with("cargo test")
        || lower.starts_with("cargo build")
        || lower.starts_with("cargo clippy")
        || lower.starts_with("pytest")
        || lower.starts_with("go test")
        || lower.starts_with("dotnet test")
    {
        return Effect::Develop;
    }
    Effect::Mutate
}

fn starts_with_sudo(line: &str) -> bool {
    let word = line.split_whitespace().next().unwrap_or("");
    matches!(word, "sudo" | "doas" | "su")
}

fn git_effect(lower: &str) -> Effect {
    if lower.contains(" push") || lower.contains(" reset") || lower.contains(" clean") {
        Effect::Dangerous
    } else if lower.contains(" commit")
        || lower.contains(" checkout")
        || lower.contains(" switch")
        || lower.contains(" merge")
        || lower.contains(" rebase")
        || lower.contains(" pull")
    {
        Effect::Mutate
    } else {
        Effect::Read
    }
}

fn action_label(name: &str, args: &Value) -> String {
    if let Some(command) = args.get("command").and_then(Value::as_str) {
        return command.to_string();
    }
    if let Some(path) = args.get("path").and_then(Value::as_str) {
        return format!("{name} {path}");
    }
    name.to_string()
}

fn dispatch(hub: &ControlHub, session_id: &str, client: &str, name: &str, args: &Value) -> Value {
    match name {
        "unit_agent_app_status" | "unit_agent_status" => hub.status_value(),
        "unit_agent_app_version" => json!({
            "success": true,
            "version": env!("CARGO_PKG_VERSION"),
            "controlApi": CONTROL_API_VERSION,
            "mcp": MCP_PROTOCOL,
            "backend": env!("CARGO_PKG_VERSION"),
        }),
        "unit_agent_app_get_settings" => safe_settings(hub),
        "unit_agent_health" | "unit_agent_diagnostics" => health(hub),
        "unit_agent_mcp_status" => json!({
            "success": true,
            "running": hub.mcp_enabled.load(Ordering::SeqCst),
            "transport": "stdio-bridge and tcp",
            "endpoint": MCP_ADDR,
            "remote": false,
            "permission": hub.level().as_str(),
            "clients": hub.clients(),
            "version": MCP_PROTOCOL,
        }),
        "unit_agent_mcp_clients" | "unit_agent_ai_sessions" => {
            json!({ "success": true, "clients": hub.clients() })
        }
        "unit_agent_mcp_start" => {
            hub.mcp_enabled.store(true, Ordering::SeqCst);
            ok(json!({ "running": true, "endpoint": MCP_ADDR }))
        }
        "unit_agent_mcp_stop" => {
            hub.mcp_enabled.store(false, Ordering::SeqCst);
            ok(json!({ "running": false, "terminals": "left running" }))
        }
        "unit_agent_mcp_disconnect" | "unit_agent_ai_session_disconnect" => {
            let id = args
                .get("session_id")
                .and_then(Value::as_str)
                .unwrap_or(session_id);
            hub.disconnect_session(id);
            ok(json!({ "disconnected": id }))
        }
        "unit_agent_ai_session_status" | "unit_agent_ai_session_permissions" => json!({
            "success": true,
            "session_id": session_id,
            "permission": hub.level().as_str(),
            "emergencyStop": hub.emergency.load(Ordering::SeqCst),
            "clients": hub.clients(),
        }),
        "unit_agent_ai_session_pause" => {
            if let Some(session) = lock(&hub.sessions).get_mut(session_id) {
                session.paused = true;
            }
            ok(json!({ "paused": true }))
        }
        "unit_agent_ai_session_resume" => {
            if let Some(session) = lock(&hub.sessions).get_mut(session_id) {
                session.paused = false;
            }
            ok(json!({ "paused": false }))
        }
        "unit_agent_ai_emergency_stop" => {
            hub.emergency_stop();
            ok(json!({ "stopped": true }))
        }
        "unit_agent_ai_activity" => json!({ "success": true, "activity": hub.activity() }),
        "unit_agent_workspace_list" => {
            json!({ "success": true, "workspaces": hub.terminals.list() })
        }
        "unit_agent_workspace_create" | "unit_agent_terminal_create" => {
            workspace_create(hub, client, args)
        }
        "unit_agent_workspace_close" | "unit_agent_terminal_close" => {
            workspace_close(hub, args, false)
        }
        "unit_agent_workspace_delete" => workspace_close(hub, args, true),
        "unit_agent_workspace_status" | "unit_agent_terminal_status" => workspace_status(hub, args),
        "unit_agent_workspace_focus" => workspace_focus(hub, args),
        "unit_agent_workspace_rename" => workspace_rename(hub, args),
        "unit_agent_workspace_set_directory" => workspace_cd(hub, session_id, client, args),
        "unit_agent_workspace_lock" => {
            let Some(id) = workspace_id(args) else {
                return fail("INVALID_ARGUMENT", "workspace_id is required");
            };
            lock(&hub.locks).insert(id.clone(), session_id.to_string());
            ok(json!({ "locked": id }))
        }
        "unit_agent_workspace_unlock" => {
            let Some(id) = workspace_id(args) else {
                return fail("INVALID_ARGUMENT", "workspace_id is required");
            };
            lock(&hub.locks).remove(&id);
            ok(json!({ "unlocked": id }))
        }
        "unit_agent_terminal_read" => terminal_read(hub, args),
        "unit_agent_terminal_write" => terminal_write(hub, args),
        "unit_agent_terminal_resize" => {
            let Some(id) = workspace_id(args) else {
                return fail("INVALID_ARGUMENT", "workspace_id is required");
            };
            let cols = args
                .get("columns")
                .or_else(|| args.get("cols"))
                .and_then(Value::as_u64)
                .unwrap_or(80) as u16;
            let rows = args.get("rows").and_then(Value::as_u64).unwrap_or(24) as u16;
            match hub.terminals.resize(&id, cols, rows) {
                Ok(()) => ok(json!({ "columns": cols, "rows": rows })),
                Err(err) => fail(&err.code, &err.message),
            }
        }
        "unit_agent_terminal_interrupt" | "unit_agent_execute_cancel" => {
            send_bytes(hub, args, "\u{3}")
        }
        "unit_agent_terminal_eof" => send_bytes(hub, args, "\u{4}"),
        "unit_agent_terminal_restart" => {
            let Some(id) = workspace_id(args) else {
                return fail("INVALID_ARGUMENT", "workspace_id is required");
            };
            match hub.terminals.restart(&id) {
                Ok(info) => {
                    hub.emit(ControlEvent::Workspace {
                        info: info.clone(),
                        focus: true,
                    });
                    ok(json!({ "workspace": info }))
                }
                Err(err) => fail(&err.code, &err.message),
            }
        }
        "unit_agent_execute" => execute(hub, session_id, client, args, false),
        "unit_agent_execute_background" | "unit_agent_dev_start" => {
            if name == "unit_agent_dev_start" {
                let mut args = args.clone();
                if args.get("command").is_none() {
                    let command = detected_command(hub, &args, Detect::Dev)
                        .unwrap_or_else(|| "npm run dev".into());
                    args["command"] = json!(command);
                }
                return execute(hub, session_id, client, &args, true);
            }
            execute(hub, session_id, client, args, true)
        }
        "unit_agent_execute_history" => {
            json!({ "success": true, "history": lock(&hub.history).iter().cloned().collect::<Vec<_>>() })
        }
        "unit_agent_process_list" => process_list(hub),
        "unit_agent_shell_info" | "unit_agent_system_info" => system_info(hub),
        "unit_agent_logs_read" | "unit_agent_audit_list" => hub.audit_list(),
        "unit_agent_build" => run_detected(hub, session_id, client, args, Detect::Build),
        "unit_agent_test" => run_detected(hub, session_id, client, args, Detect::Test),
        "unit_agent_lint" => run_detected(hub, session_id, client, args, Detect::Lint),
        "unit_agent_git_status" => {
            git_in_pty(hub, session_id, client, args, "git status --short --branch")
        }
        "unit_agent_git_diff" => git_in_pty(hub, session_id, client, args, "git diff --stat"),
        "unit_agent_git_log" => git_in_pty(hub, session_id, client, args, "git log -5 --oneline"),
        "unit_agent_git_branch_list" => {
            git_in_pty(hub, session_id, client, args, "git branch --list")
        }
        "unit_agent_git_remote" => git_in_pty(hub, session_id, client, args, "git remote -v"),
        "unit_agent_git_commit" => git_commit(hub, session_id, client, args),
        "unit_agent_git_push" => git_in_pty(hub, session_id, client, args, "git push"),
        "unit_agent_fs_list" => fs_list(hub, args),
        "unit_agent_fs_stat" => fs_stat(hub, args),
        "unit_agent_fs_exists" => fs_exists(hub, args),
        "unit_agent_fs_read" => fs_read(hub, args),
        "unit_agent_fs_write" => fs_write(hub, args, false),
        "unit_agent_fs_append" => fs_write(hub, args, true),
        "unit_agent_fs_create_directory" => fs_mkdir(hub, args),
        "unit_agent_fs_delete" => fs_delete(hub, args),
        "unit_agent_file_replace" => file_replace(hub, args),
        "unit_agent_search" => search(hub, args),
        "unit_agent_project_detect"
        | "unit_agent_project_info"
        | "unit_agent_package_manager_detect" => project_info(hub, args),
        "unit_agent_audit_export" => hub.audit_list(),
        "unit_agent_cpu_status" => cpu_status(),
        "unit_agent_memory_status" => memory_status(),
        "unit_agent_disk_status" => disk_status(hub, args),
        "unit_agent_network_status" => {
            let host = host_identity();
            ok(
                json!({ "computerName": host.computer_name, "serverName": host.server_name, "network": host.network }),
            )
        }
        "unit_agent_internet_status" => {
            let snap = hub.connection.snapshot();
            ok(json!({ "state": snap.state, "internet": snap.internet, "detail": snap.detail }))
        }
        "unit_agent_prysel_status" | "unit_agent_auth_status" | "unit_agent_user_profile" => {
            prysel_status(hub)
        }
        "unit_agent_port_check" => port_check(args),
        "unit_agent_docker_status" => docker_cmd(&["version", "--format", "{{.Server.Version}}"]),
        "unit_agent_docker_ps" => {
            docker_cmd(&["ps", "--format", "{{.Names}}\t{{.Status}}\t{{.Ports}}"])
        }
        "unit_agent_clipboard_read" | "unit_agent_screenshot" => gated_feature(hub, name),
        "unit_agent_notification" => {
            let message = args
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Unit Agent")
                .to_string();
            hub.note(client, "", &message, "", "notice");
            ok(json!({ "shown": true }))
        }
        "unit_agent_app_minimize"
        | "unit_agent_app_focus"
        | "unit_agent_app_shutdown"
        | "unit_agent_app_restart"
        | "unit_agent_ui_focus"
        | "unit_agent_ui_show_settings" => {
            hub.emit(ControlEvent::Status);
            ok(
                json!({ "requested": name, "note": "The desktop handles window actions. Headless mode keeps the control process running." }),
            )
        }
        other => fail("INVALID_ARGUMENT", &format!("Unknown tool {other}")),
    }
}

fn safe_settings(hub: &ControlHub) -> Value {
    let settings = hub.storage.settings();
    ok(json!({
        "theme": settings.theme,
        "fontSize": settings.font_size,
        "shell": settings.shell,
        "restoreWorkspaces": settings.restore_workspaces,
        "aiMcp": settings.ai_mcp,
        "aiPermission": settings.ai_permission,
        "aiScreenshots": settings.ai_screenshots,
        "aiClipboard": settings.ai_clipboard,
        "secretConfigured": hub.config.configured(),
    }))
}

fn health(hub: &ControlHub) -> Value {
    let snap = hub.connection.snapshot();
    ok(json!({
        "app": "ok",
        "mcp": if hub.mcp_enabled.load(Ordering::SeqCst) { "ok" } else { "disabled" },
        "terminal": "ok",
        "workspace": "ok",
        "filesystem": "ok",
        "prysel": snap.prysel,
        "internet": snap.internet,
        "emergencyStop": hub.emergency.load(Ordering::SeqCst),
    }))
}

fn workspace_id(args: &Value) -> Option<String> {
    args.get("workspace_id")
        .or_else(|| args.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

fn workspace_create(hub: &ControlHub, client: &str, args: &Value) -> Value {
    let title = args
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("Terminal");
    let cwd = args
        .get("directory")
        .or_else(|| args.get("cwd"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let shell = args
        .get("shell")
        .and_then(Value::as_str)
        .map(str::to_string);
    match hub.terminals.create(CreateTerminalRequest {
        id: None,
        title: Some(title.to_string()),
        cwd: cwd.clone(),
        shell: shell.clone(),
    }) {
        Ok(info) => {
            let cols = args.get("columns").and_then(Value::as_u64).unwrap_or(0) as u16;
            let rows = args.get("rows").and_then(Value::as_u64).unwrap_or(0) as u16;
            if cols > 0 && rows > 0 {
                let _ = hub.terminals.resize(&info.id, cols, rows);
            }
            let _ = hub.storage.save_workspace(SavedWorkspace {
                id: info.id.clone(),
                title: info.title.clone(),
                cwd: info.cwd.clone(),
                shell: info.shell.clone(),
                created_at: info.created_at,
            });
            lock(&hub.owners).insert(info.id.clone(), client.to_string());
            hub.emit(ControlEvent::Workspace {
                info: info.clone(),
                focus: true,
            });
            ok(json!({ "workspace": info, "owner": client }))
        }
        Err(err) => fail(&err.code, &err.message),
    }
}

fn workspace_close(hub: &ControlHub, args: &Value, forget: bool) -> Value {
    let Some(id) = workspace_id(args) else {
        return fail("INVALID_ARGUMENT", "workspace_id is required");
    };
    if let Err(err) = hub.terminals.close(&id, true) {
        if err.code != "NOT_FOUND" {
            return fail(&err.code, &err.message);
        }
    }
    if forget {
        let _ = hub.storage.remove_workspace(&id);
    }
    lock(&hub.owners).remove(&id);
    lock(&hub.locks).remove(&id);
    ok(json!({ "closed": id, "filesDeleted": false }))
}

fn workspace_status(hub: &ControlHub, args: &Value) -> Value {
    let Some(id) = workspace_id(args) else {
        return fail("INVALID_ARGUMENT", "workspace_id is required");
    };
    match hub.terminals.snapshot(&id) {
        Ok(snap) => {
            let owner = lock(&hub.owners)
                .get(&id)
                .cloned()
                .unwrap_or_else(|| "user".into());
            let locked = lock(&hub.locks).contains_key(&id);
            ok(json!({
                "workspace": snap.info,
                "owner": owner,
                "locked": locked,
                "rows": 24,
                "columns": 80,
            }))
        }
        Err(err) => fail(&err.code, &err.message),
    }
}

fn workspace_focus(hub: &ControlHub, args: &Value) -> Value {
    let Some(id) = workspace_id(args) else {
        return fail("INVALID_ARGUMENT", "workspace_id is required");
    };
    match hub.terminals.list().into_iter().find(|info| info.id == id) {
        Some(info) => {
            hub.emit(ControlEvent::Workspace {
                info: info.clone(),
                focus: true,
            });
            ok(json!({ "focused": info.id }))
        }
        None => fail("WORKSPACE_NOT_FOUND", "Workspace not found."),
    }
}

fn workspace_rename(hub: &ControlHub, args: &Value) -> Value {
    let Some(id) = workspace_id(args) else {
        return fail("INVALID_ARGUMENT", "workspace_id is required");
    };
    let Some(name) = args.get("name").and_then(Value::as_str) else {
        return fail("INVALID_ARGUMENT", "name is required");
    };
    let settings = hub.storage.settings();
    if let Some(existing) = settings.workspaces.iter().find(|w| w.id == id) {
        let _ = hub.storage.save_workspace(SavedWorkspace {
            title: name.to_string(),
            ..existing.clone()
        });
    }
    ok(json!({ "id": id, "name": name }))
}

fn workspace_cd(hub: &ControlHub, session_id: &str, client: &str, args: &Value) -> Value {
    let Some(dir) = args
        .get("directory")
        .or_else(|| args.get("cwd"))
        .and_then(Value::as_str)
    else {
        return fail("INVALID_ARGUMENT", "directory is required");
    };
    let mut args = args.clone();
    args["command"] = json!(format!("cd {}", shell_quote(dir)));
    execute(hub, session_id, client, &args, false)
}

fn terminal_read(hub: &ControlHub, args: &Value) -> Value {
    let Some(id) = workspace_id(args) else {
        return fail("INVALID_ARGUMENT", "workspace_id is required");
    };
    match hub.terminals.snapshot(&id) {
        Ok(snap) => {
            let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
            let max_bytes = args
                .get("max_bytes")
                .and_then(Value::as_u64)
                .unwrap_or(16_384) as usize;
            let max_bytes = max_bytes.clamp(1, 64 * 1024);
            let text = snap.output;
            let start = offset.min(text.len());
            let end = (start + max_bytes).min(text.len());
            ok(json!({
                "output": redact(&text[start..end]),
                "offset": start,
                "next": end,
                "eof": end == text.len(),
            }))
        }
        Err(err) => fail(&err.code, &err.message),
    }
}

fn terminal_write(hub: &ControlHub, args: &Value) -> Value {
    let Some(id) = workspace_id(args) else {
        return fail("INVALID_ARGUMENT", "workspace_id is required");
    };
    let Some(data) = args
        .get("data")
        .or_else(|| args.get("command"))
        .and_then(Value::as_str)
    else {
        return fail("INVALID_ARGUMENT", "data is required");
    };
    match hub.terminals.write(&id, data) {
        Ok(result) => ok(json!({ "written": result.written, "confirmation": result.confirmation })),
        Err(err) => fail(&err.code, &err.message),
    }
}

fn send_bytes(hub: &ControlHub, args: &Value, bytes: &str) -> Value {
    let Some(id) = workspace_id(args) else {
        return fail("INVALID_ARGUMENT", "workspace_id is required");
    };
    match hub.terminals.write(&id, bytes) {
        Ok(_) => ok(json!({ "sent": true })),
        Err(err) => fail(&err.code, &err.message),
    }
}

fn execute(
    hub: &ControlHub,
    session_id: &str,
    client: &str,
    args: &Value,
    background: bool,
) -> Value {
    let Some(command) = args
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|c| !c.is_empty())
    else {
        return fail("INVALID_ARGUMENT", "command is required");
    };
    let Some(id) = workspace_id(args) else {
        return fail(
            "WORKSPACE_NOT_FOUND",
            "workspace_id is required. Create or list a workspace first.",
        );
    };
    let redacted = redact(command);
    lock(&hub.history).push_back(json!({
        "source": "ai",
        "client": client,
        "session": session_id,
        "workspace": id,
        "command": redacted,
        "at": now_secs(),
    }));
    let before = hub
        .terminals
        .snapshot(&id)
        .map(|s| s.output.len())
        .unwrap_or(0);
    let banner = format!("printf '[AI] %s\\n' {}\n", shell_quote(&redacted));
    if let Err(error) = write_pty(hub, &id, &banner) {
        return error;
    }
    let line = if command.ends_with('\n') || command.ends_with('\r') {
        command.to_string()
    } else if background {
        format!("{command}\r")
    } else {
        format!("{command}\n")
    };
    if let Err(error) = write_pty(hub, &id, &line) {
        return error;
    }
    if background {
        return ok(
            json!({ "running": true, "workspace_id": id, "command": redacted, "process_id": id }),
        );
    }
    let marker = format!("__UA{}__", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let marker_line = format!("printf '{marker}:%s\\n' \"$?\"\n");
    if let Err(error) = write_pty(hub, &id, &marker_line) {
        return error;
    }
    let timeout = args
        .get("timeout")
        .and_then(Value::as_u64)
        .unwrap_or(8_000)
        .clamp(200, 120_000);
    let deadline = Instant::now() + Duration::from_millis(timeout);
    loop {
        let output = hub
            .terminals
            .snapshot(&id)
            .map(|s| s.output)
            .unwrap_or_default();
        if let Some(exit_code) = exit_marker(&output, &marker) {
            let shown = output.get(before..).unwrap_or("");
            let shown = shown
                .lines()
                .filter(|line| !line.contains(&marker) && !line.contains("printf '[AI]"))
                .collect::<Vec<_>>()
                .join("\n");
            return ok(json!({
                "workspace_id": id,
                "command": redacted,
                "exit_code": exit_code,
                "output": redact(&shown),
                "running": false,
            }));
        }
        if Instant::now() >= deadline {
            let shown = output.get(before..).unwrap_or("");
            return json!({
                "success": false,
                "code": "COMMAND_TIMEOUT",
                "message": "The command is still running in the workspace terminal.",
                "running": true,
                "workspace_id": id,
                "command": redacted,
                "output": redact(shown),
                "process_id": id,
            });
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn write_pty(hub: &ControlHub, id: &str, data: &str) -> Result<(), Value> {
    match hub.terminals.write(id, data) {
        Ok(result)
            if result
                .confirmation
                .as_ref()
                .is_some_and(|command| data.contains(command.as_str())) =>
        {
            hub.terminals
                .confirm(id, true)
                .map_err(|err| fail(&err.code, &err.message))
        }
        Ok(result) if result.confirmation.is_some() => Err(fail(
            "CONFIRMATION_REQUIRED",
            "The terminal is already waiting on another confirmation.",
        )),
        Ok(_) => Ok(()),
        Err(err) => Err(fail(&err.code, &err.message)),
    }
}

fn exit_marker(output: &str, marker: &str) -> Option<i32> {
    for line in output.lines().rev() {
        if line.contains("printf") {
            continue;
        }
        let Some(pos) = line.rfind(marker) else {
            continue;
        };
        let rest = line[pos + marker.len()..].trim_start_matches(':').trim();
        let token = rest.split_whitespace().next().unwrap_or("");
        if let Ok(code) = token.parse::<i32>() {
            return Some(code);
        }
    }
    None
}

fn process_list(hub: &ControlHub) -> Value {
    let processes: Vec<Value> = hub
        .terminals
        .list()
        .into_iter()
        .map(|info| {
            json!({
                "workspace_id": info.id,
                "pid": info.pid,
                "running": info.running,
                "shell": info.shell,
                "cwd": info.cwd,
                "title": info.title,
            })
        })
        .collect();
    ok(json!({ "processes": processes }))
}

fn git_in_pty(
    hub: &ControlHub,
    session_id: &str,
    client: &str,
    args: &Value,
    command: &str,
) -> Value {
    let mut args = args.clone();
    args["command"] = json!(command);
    execute(hub, session_id, client, &args, false)
}

fn git_commit(hub: &ControlHub, session_id: &str, client: &str, args: &Value) -> Value {
    let message = args
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if message.is_empty() || message.contains('\n') || message.contains('\'') {
        return fail(
            "INVALID_ARGUMENT",
            "Commit message must be a single line without quotes.",
        );
    }
    let mut args = args.clone();
    args["command"] = json!(format!("git commit -m {}", shell_quote(message)));
    execute(hub, session_id, client, &args, false)
}

#[derive(Clone, Copy)]
enum Detect {
    Build,
    Test,
    Lint,
    Dev,
}

fn run_detected(
    hub: &ControlHub,
    session_id: &str,
    client: &str,
    args: &Value,
    kind: Detect,
) -> Value {
    let Some(command) = detected_command(hub, args, kind) else {
        return fail(
            "INVALID_ARGUMENT",
            "No build, test, or lint command was detected.",
        );
    };
    let mut args = args.clone();
    args["command"] = json!(command);
    execute(hub, session_id, client, &args, false)
}

fn detected_command(hub: &ControlHub, args: &Value, kind: Detect) -> Option<String> {
    let root = workspace_root(hub, args);
    let has = |name: &str| root.join(name).exists();
    match kind {
        Detect::Build if has("Cargo.toml") => Some("cargo build".into()),
        Detect::Build if has("package.json") => Some("npm run build".into()),
        Detect::Build if has("go.mod") => Some("go build".into()),
        Detect::Build if has("pom.xml") => Some("mvn -q package".into()),
        Detect::Test if has("Cargo.toml") => Some("cargo test".into()),
        Detect::Test if has("package.json") => Some("npm test".into()),
        Detect::Test if has("pyproject.toml") || has("pytest.ini") => Some("pytest".into()),
        Detect::Test if has("go.mod") => Some("go test ./...".into()),
        Detect::Lint if has("Cargo.toml") => Some("cargo clippy -- -D warnings".into()),
        Detect::Lint if has("package.json") => Some("npm run lint".into()),
        Detect::Dev if has("package.json") => Some("npm run dev".into()),
        Detect::Dev if has("Cargo.toml") => Some("cargo run".into()),
        _ => None,
    }
}

fn workspace_root(hub: &ControlHub, args: &Value) -> PathBuf {
    if let Some(id) = workspace_id(args) {
        if let Ok(snap) = hub.terminals.snapshot(&id) {
            return PathBuf::from(snap.info.cwd);
        }
    }
    if let Some(dir) = args
        .get("directory")
        .or_else(|| args.get("path"))
        .and_then(Value::as_str)
    {
        return PathBuf::from(dir);
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn project_info(hub: &ControlHub, args: &Value) -> Value {
    let root = workspace_root(hub, args);
    let mut tech = Vec::new();
    let mut managers = Vec::new();
    let mark = |tech: &mut Vec<String>,
                managers: &mut Vec<String>,
                file: &str,
                name: &str,
                manager: Option<&str>| {
        if root.join(file).exists() {
            tech.push(name.into());
            if let Some(manager) = manager {
                managers.push(manager.into());
            }
        }
    };
    mark(
        &mut tech,
        &mut managers,
        "package.json",
        "Node.js",
        Some("npm"),
    );
    mark(
        &mut tech,
        &mut managers,
        "pnpm-lock.yaml",
        "pnpm",
        Some("pnpm"),
    );
    mark(&mut tech, &mut managers, "yarn.lock", "yarn", Some("yarn"));
    mark(&mut tech, &mut managers, "bun.lockb", "bun", Some("bun"));
    mark(
        &mut tech,
        &mut managers,
        "Cargo.toml",
        "Rust",
        Some("cargo"),
    );
    mark(
        &mut tech,
        &mut managers,
        "pyproject.toml",
        "Python",
        Some("pip"),
    );
    mark(
        &mut tech,
        &mut managers,
        "requirements.txt",
        "Python",
        Some("pip"),
    );
    mark(&mut tech, &mut managers, "go.mod", "Go", Some("go"));
    mark(&mut tech, &mut managers, "pom.xml", "Java", Some("maven"));
    mark(
        &mut tech,
        &mut managers,
        "build.gradle",
        "Java",
        Some("gradle"),
    );
    mark(
        &mut tech,
        &mut managers,
        "composer.json",
        "PHP",
        Some("composer"),
    );
    mark(&mut tech, &mut managers, "Gemfile", "Ruby", Some("gem"));
    mark(&mut tech, &mut managers, "CMakeLists.txt", "C/C++", None);
    if let Ok(entries) = std::fs::read_dir(&root) {
        if entries
            .flatten()
            .any(|entry| entry.path().extension().is_some_and(|ext| ext == "csproj"))
        {
            tech.push(".NET".into());
            managers.push("dotnet".into());
        }
    }
    mark(&mut tech, &mut managers, "Dockerfile", "Docker", None);
    mark(
        &mut tech,
        &mut managers,
        "docker-compose.yml",
        "Docker Compose",
        None,
    );
    mark(
        &mut tech,
        &mut managers,
        "src-tauri/tauri.conf.json",
        "Tauri",
        None,
    );
    if root.join("package.json").exists() {
        if let Ok(text) = std::fs::read_to_string(root.join("package.json")) {
            for (needle, name) in [
                ("\"react\"", "React"),
                ("\"vue\"", "Vue"),
                ("\"svelte\"", "Svelte"),
                ("\"next\"", "Next.js"),
                ("\"nuxt\"", "Nuxt"),
            ] {
                if text.contains(needle) {
                    tech.push(name.into());
                }
            }
        }
    }
    ok(json!({
        "directory": root,
        "name": root.file_name().and_then(|n| n.to_str()).unwrap_or("project"),
        "technologies": tech,
        "packageManagers": managers,
        "git": root.join(".git").exists(),
    }))
}

fn fs_list(hub: &ControlHub, args: &Value) -> Value {
    let root = workspace_root(hub, args);
    let rel = args.get("path").and_then(Value::as_str).unwrap_or(".");
    let path = match resolve_path(&root, rel, hub.level() == Level::Full) {
        Ok(path) => path,
        Err(message) => return fail("PATH_NOT_ALLOWED", message),
    };
    let mut names = Vec::new();
    let entries = match std::fs::read_dir(&path) {
        Ok(entries) => entries,
        Err(err) => return fail("PATH_NOT_ALLOWED", &err.to_string()),
    };
    for entry in entries.flatten().take(200) {
        names.push(entry.file_name().to_string_lossy().to_string());
    }
    names.sort();
    ok(json!({ "path": path, "entries": names }))
}

fn fs_stat(hub: &ControlHub, args: &Value) -> Value {
    let root = workspace_root(hub, args);
    let Some(rel) = args.get("path").and_then(Value::as_str) else {
        return fail("INVALID_ARGUMENT", "path is required");
    };
    let path = match resolve_path(&root, rel, hub.level() == Level::Full) {
        Ok(path) => path,
        Err(message) => return fail("PATH_NOT_ALLOWED", message),
    };
    match std::fs::metadata(&path) {
        Ok(meta) => ok(json!({
            "path": path,
            "file": meta.is_file(),
            "directory": meta.is_dir(),
            "size": meta.len(),
            "modified": meta.modified().ok().and_then(|t| t.elapsed().ok()).map(|e| e.as_secs()),
        })),
        Err(err) => fail("PATH_NOT_ALLOWED", &err.to_string()),
    }
}

fn fs_exists(hub: &ControlHub, args: &Value) -> Value {
    let root = workspace_root(hub, args);
    let Some(rel) = args.get("path").and_then(Value::as_str) else {
        return fail("INVALID_ARGUMENT", "path is required");
    };
    match resolve_path(&root, rel, hub.level() == Level::Full) {
        Ok(path) => ok(json!({ "exists": path.exists(), "path": path })),
        Err(message) => fail("PATH_NOT_ALLOWED", message),
    }
}

fn fs_read(hub: &ControlHub, args: &Value) -> Value {
    let root = workspace_root(hub, args);
    let Some(rel) = args.get("path").and_then(Value::as_str) else {
        return fail("INVALID_ARGUMENT", "path is required");
    };
    let path = match resolve_path(&root, rel, hub.level() == Level::Full) {
        Ok(path) => path,
        Err(message) => return fail("PATH_NOT_ALLOWED", message),
    };
    if is_secret_path(&path) {
        return fail(
            "PATH_NOT_ALLOWED",
            "This file is treated as a secret and is not returned to AI.",
        );
    }
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) => return fail("PATH_NOT_ALLOWED", &err.to_string()),
    };
    let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(32_768) as usize;
    let limit = limit.clamp(1, 64 * 1024);
    if offset >= bytes.len() {
        return ok(json!({ "content": "", "offset": offset, "size": bytes.len() }));
    }
    let end = (offset + limit).min(bytes.len());
    let text = String::from_utf8_lossy(&bytes[offset..end]);
    ok(json!({ "content": redact(&text), "offset": offset, "next": end, "size": bytes.len() }))
}

fn fs_write(hub: &ControlHub, args: &Value, append: bool) -> Value {
    let root = workspace_root(hub, args);
    let Some(rel) = args.get("path").and_then(Value::as_str) else {
        return fail("INVALID_ARGUMENT", "path is required");
    };
    let Some(content) = args.get("content").and_then(Value::as_str) else {
        return fail("INVALID_ARGUMENT", "content is required");
    };
    let path = match resolve_path(&root, rel, false) {
        Ok(path) => path,
        Err(message) => return fail("PATH_NOT_ALLOWED", message),
    };
    if is_secret_path(&path) {
        return fail("PATH_NOT_ALLOWED", "Refusing to write a secret path.");
    }
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return fail("PATH_NOT_ALLOWED", "Could not create the parent directory.");
        }
    }
    let result = if append {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut file| {
                use std::io::Write;
                file.write_all(content.as_bytes())
            })
    } else {
        std::fs::write(&path, content)
    };
    match result {
        Ok(()) => ok(json!({ "path": path, "bytes": content.len() })),
        Err(err) => fail("PATH_NOT_ALLOWED", &err.to_string()),
    }
}

fn fs_mkdir(hub: &ControlHub, args: &Value) -> Value {
    let root = workspace_root(hub, args);
    let Some(rel) = args.get("path").and_then(Value::as_str) else {
        return fail("INVALID_ARGUMENT", "path is required");
    };
    let path = match resolve_path(&root, rel, false) {
        Ok(path) => path,
        Err(message) => return fail("PATH_NOT_ALLOWED", message),
    };
    match std::fs::create_dir_all(&path) {
        Ok(()) => ok(json!({ "path": path })),
        Err(err) => fail("PATH_NOT_ALLOWED", &err.to_string()),
    }
}

fn fs_delete(hub: &ControlHub, args: &Value) -> Value {
    let root = workspace_root(hub, args);
    let Some(rel) = args.get("path").and_then(Value::as_str) else {
        return fail("INVALID_ARGUMENT", "path is required");
    };
    let path = match resolve_path(&root, rel, false) {
        Ok(path) => path,
        Err(message) => return fail("PATH_NOT_ALLOWED", message),
    };
    if path == root {
        return fail("PATH_NOT_ALLOWED", "Refusing to delete the workspace root.");
    }
    let result = if path.is_dir() {
        std::fs::remove_dir(&path)
    } else {
        std::fs::remove_file(&path)
    };
    match result {
        Ok(()) => ok(json!({ "deleted": path })),
        Err(err) => fail("PATH_NOT_ALLOWED", &err.to_string()),
    }
}

fn file_replace(hub: &ControlHub, args: &Value) -> Value {
    let root = workspace_root(hub, args);
    let Some(rel) = args.get("path").and_then(Value::as_str) else {
        return fail("INVALID_ARGUMENT", "path is required");
    };
    let Some(find) = args.get("find").and_then(Value::as_str) else {
        return fail("INVALID_ARGUMENT", "find is required");
    };
    let Some(replace) = args.get("replace").and_then(Value::as_str) else {
        return fail("INVALID_ARGUMENT", "replace is required");
    };
    if find.is_empty() {
        return fail("INVALID_ARGUMENT", "find must not be empty");
    }
    let path = match resolve_path(&root, rel, false) {
        Ok(path) => path,
        Err(message) => return fail("PATH_NOT_ALLOWED", message),
    };
    if is_secret_path(&path) {
        return fail("PATH_NOT_ALLOWED", "Refusing to edit a secret path.");
    }
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) => return fail("PATH_NOT_ALLOWED", &err.to_string()),
    };
    if !text.contains(find) {
        return fail("INVALID_ARGUMENT", "The text to replace was not found.");
    }
    let next = text.replacen(find, replace, 1);
    match std::fs::write(&path, &next) {
        Ok(()) => ok(json!({ "path": path, "replaced": true })),
        Err(err) => fail("PATH_NOT_ALLOWED", &err.to_string()),
    }
}

fn search(hub: &ControlHub, args: &Value) -> Value {
    let root = workspace_root(hub, args);
    let Some(query) = args.get("query").and_then(Value::as_str) else {
        return fail("INVALID_ARGUMENT", "query is required");
    };
    let max = args
        .get("max_results")
        .and_then(Value::as_u64)
        .unwrap_or(30)
        .clamp(1, 100) as usize;
    let mut hits = Vec::new();
    walk_search(&root, &root, query, max, &mut hits);
    ok(json!({ "matches": hits }))
}

fn walk_search(root: &Path, dir: &Path, query: &str, max: usize, hits: &mut Vec<Value>) {
    if hits.len() >= max {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if hits.len() >= max {
            return;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if matches!(
            name.as_str(),
            ".git"
                | "node_modules"
                | "target"
                | "dist"
                | "build"
                | ".cache"
                | ".next"
                | ".nuxt"
                | "venv"
                | ".venv"
        ) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            walk_search(root, &path, query, max, hits);
            continue;
        }
        if path.metadata().map(|m| m.len() > 200_000).unwrap_or(true) || is_secret_path(&path) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(index) = text.find(query) {
            let line = text[..index].chars().filter(|ch| *ch == '\n').count() + 1;
            hits.push(json!({ "path": path.strip_prefix(root).unwrap_or(&path), "line": line }));
        }
    }
}

pub fn resolve_path(
    root: &Path,
    requested: &str,
    allow_outside: bool,
) -> Result<PathBuf, &'static str> {
    if requested.contains('\0') {
        return Err("The path is not allowed.");
    }
    let joined = if Path::new(requested).is_absolute() {
        PathBuf::from(requested)
    } else {
        root.join(requested)
    };
    let root_real = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let candidate = if joined.exists() {
        joined
            .canonicalize()
            .map_err(|_| "The path is not allowed.")?
    } else if let Some(parent) = joined.parent() {
        let parent_real = if parent.as_os_str().is_empty() {
            root_real.clone()
        } else if parent.exists() {
            parent
                .canonicalize()
                .map_err(|_| "The path is not allowed.")?
        } else {
            parent.to_path_buf()
        };
        match joined.file_name() {
            Some(name) => parent_real.join(name),
            None => parent_real,
        }
    } else {
        joined
    };
    let candidate = lexical_normal(&candidate);
    let root_check = lexical_normal(&root_real);
    if candidate.as_os_str().is_empty() {
        return Err("The path is not allowed.");
    }
    if is_secret_path(&candidate) {
        return Err("This file is treated as a secret and is not available to AI.");
    }
    if !allow_outside && !candidate.starts_with(&root_check) {
        return Err("The path is outside the workspace root.");
    }
    Ok(candidate)
}

fn lexical_normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            other => out.push(other),
        }
    }
    out
}

pub fn is_secret_path(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if name == ".env"
        || name.starts_with(".env.")
        || name == "id_rsa"
        || name == "id_ed25519"
        || name == "id_ecdsa"
        || name == "cookies.sqlite"
    {
        return true;
    }
    let text = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    text.contains("/.ssh/")
        || text.contains("/.gnupg/")
        || text.ends_with(".pem") && text.contains("private")
}

pub fn redact(text: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let rest: String = chars[index..].iter().collect();
        if rest.starts_with("-----BEGIN ") && rest.contains("PRIVATE KEY-----") {
            if let Some(end) = rest.find("-----END ") {
                let after = rest[end..]
                    .find('\n')
                    .map(|n| end + n + 1)
                    .unwrap_or(rest.len());
                out.push_str("[REDACTED]");
                index += after;
                continue;
            }
        }
        if rest.starts_with("sk-") || rest.starts_with("AKIA") {
            let take = rest.chars().take(24).count();
            if take >= 12
                && rest
                    .chars()
                    .take(take)
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
            {
                out.push_str("[REDACTED]");
                index += take;
                continue;
            }
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn system_info(hub: &ControlHub) -> Value {
    let host = host_identity();
    let shell = crate::system::detect_shell(None).unwrap_or_else(|_| "unavailable".into());
    ok(json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "family": std::env::consts::FAMILY,
        "hostname": host.server_name,
        "computerName": host.computer_name,
        "shell": shell,
        "user": std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "user".into()),
        "workspaces": hub.terminals.list().len(),
    }))
}

fn cpu_status() -> Value {
    let load = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    ok(
        json!({ "cores": cores, "load": load.split_whitespace().take(3).collect::<Vec<_>>().join(" ") }),
    )
}

fn memory_status() -> Value {
    let text = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let kb = |key: &str| {
        text.lines().find_map(|line| {
            let rest = line.strip_prefix(key)?.trim();
            rest.split_whitespace().next()?.parse::<u64>().ok()
        })
    };
    ok(json!({
        "totalKb": kb("MemTotal:"),
        "availableKb": kb("MemAvailable:"),
    }))
}

fn disk_status(hub: &ControlHub, args: &Value) -> Value {
    let root = workspace_root(hub, args);
    unix_disk(&root).unwrap_or_else(|| ok(json!({ "path": root, "available": false })))
}

#[cfg(unix)]
fn unix_disk(root: &Path) -> Option<Value> {
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let text = root.to_string_lossy();
    let path = std::ffi::CString::new(text.as_bytes()).ok()?;
    let rc = unsafe { libc::statvfs(path.as_ptr(), &mut stat) };
    if rc != 0 {
        return None;
    }
    let block = stat.f_frsize as u64;
    Some(ok(json!({
        "path": root,
        "total": stat.f_blocks as u64 * block,
        "available": stat.f_bavail as u64 * block,
    })))
}

#[cfg(not(unix))]
fn unix_disk(_root: &Path) -> Option<Value> {
    None
}

fn prysel_status(hub: &ControlHub) -> Value {
    let snap = hub.connection.snapshot();
    let session = hub
        .auth
        .restore()
        .unwrap_or_else(|_| crate::auth::SessionView::none());
    let user = session.user.as_ref().map(|user| {
        json!({
            "name": user.name,
            "username": user.username,
            "email": user.email,
            "picture": user.picture,
        })
    });
    ok(json!({
        "authenticated": session.authenticated,
        "expired": session.expired,
        "source": session.source,
        "prysel": snap.prysel,
        "internet": snap.internet,
        "configured": hub.config.configured(),
        "user": user,
    }))
}

fn port_check(args: &Value) -> Value {
    let port = args.get("port").and_then(Value::as_u64).unwrap_or(0);
    if port == 0 || port > 65535 {
        return fail("INVALID_ARGUMENT", "port must be between 1 and 65535");
    }
    let address = format!("127.0.0.1:{port}");
    let open = std::net::TcpStream::connect_timeout(
        &address
            .parse()
            .unwrap_or_else(|_| std::net::SocketAddr::from(([127, 0, 0, 1], 9))),
        Duration::from_millis(200),
    )
    .is_ok();
    ok(json!({ "port": port, "open": open, "host": "127.0.0.1" }))
}

fn docker_cmd(args: &[&str]) -> Value {
    let output = std::process::Command::new("docker").args(args).output();
    match output {
        Ok(output) if output.status.success() => ok(
            json!({ "available": true, "output": String::from_utf8_lossy(&output.stdout).trim().to_string() }),
        ),
        Ok(_) => ok(json!({ "available": false, "output": "" })),
        Err(_) => ok(json!({ "available": false, "output": "" })),
    }
}

fn gated_feature(hub: &ControlHub, name: &str) -> Value {
    let settings = hub.storage.settings();
    let allowed = if name.contains("clipboard") {
        settings.ai_clipboard
    } else {
        settings.ai_screenshots
    };
    if !allowed {
        return fail(
            "PERMISSION_DENIED",
            "The user has not enabled this capture. Unit Agent does not read it silently.",
        );
    }
    fail("PERMISSION_DENIED", "Capture stays off in this build even after the switch is enabled, so the desktop is not grabbed in the background.")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hub() -> Arc<ControlHub> {
        let dir = std::env::temp_dir().join(format!("unit-agent-control-{}", uuid::Uuid::new_v4()));
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
    fn catalog_includes_execute_and_workspaces() {
        let names: Vec<&str> = TOOLS.iter().map(|tool| tool.name).collect();
        assert!(names.contains(&"unit_agent_execute"));
        assert!(names.contains(&"unit_agent_workspace_create"));
        assert!(names.contains(&"unit_agent_git_push"));
        assert_eq!(command_effect("echo rm -rf /"), Effect::Read);
        assert_eq!(command_effect("rm -rf /"), Effect::Dangerous);
        assert_eq!(command_effect("git status"), Effect::Read);
        assert_eq!(command_effect("git push"), Effect::Dangerous);
        assert_eq!(authorize(Level::ReadOnly, Effect::Mutate), Decision::Deny);
        assert_eq!(authorize(Level::Safe, Effect::Develop), Decision::Allow);
        let hub = hub();
        hub.set_level(Level::Full);
        let session = hub.open_session("Claude");
        let missing = hub.call_tool(&session, "unit_agent_execute", &json!({ "command": "pwd" }));
        assert_eq!(missing["code"], "WORKSPACE_NOT_FOUND");
    }

    #[test]
    fn blocks_secret_paths_and_traversal() {
        let root = std::env::temp_dir().join(format!("unit-agent-root-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        assert!(is_secret_path(Path::new("/home/ada/.ssh/id_ed25519")));
        assert!(is_secret_path(Path::new("/tmp/project/.env")));
        assert!(resolve_path(&root, "../etc/passwd", false).is_err());
        assert!(resolve_path(&root, "src/main.rs", false).is_ok());
        assert!(redact("token sk-abcdefghijklmnopqrstuvwxyz").contains("[REDACTED]"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn execute_uses_the_same_pty_and_denied_commands_do_not_run() {
        if !std::path::Path::new("/dev/pts").exists() {
            return;
        }
        let hub = hub();
        hub.set_level(Level::Full);
        let session = hub.open_session("Claude");
        let created = hub.call_tool(
            &session,
            "unit_agent_workspace_create",
            &json!({ "name": "AI", "directory": "/tmp" }),
        );
        assert_eq!(created["success"], true, "{created}");
        let id = created["workspace"]["id"].as_str().unwrap().to_string();
        let echoed = hub.call_tool(
            &session,
            "unit_agent_execute",
            &json!({ "workspace_id": id, "command": "echo Unit Agent" }),
        );
        assert_eq!(echoed["success"], true, "{echoed}");
        assert!(
            echoed["output"]
                .as_str()
                .unwrap_or("")
                .contains("Unit Agent"),
            "{echoed}"
        );
        let listed = hub.call_tool(&session, "unit_agent_workspace_list", &json!({}));
        assert!(listed["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == id));

        let hub_deny = hub.clone();
        hub.set_listener(Arc::new(move |event| {
            if let ControlEvent::Confirm { id, .. } = event {
                hub_deny.resolve_confirm(&id, false);
            }
        }));
        let before = hub.terminals.snapshot(&id).unwrap().output;
        let denied = hub.call_tool(
            &session,
            "unit_agent_execute",
            &json!({ "workspace_id": id, "command": "rm -rf /" }),
        );
        assert_eq!(denied["success"], false, "{denied}");
        assert_eq!(denied["code"], "CONFIRMATION_REQUIRED");
        let after = hub.terminals.snapshot(&id).unwrap().output;
        assert_eq!(before, after);
        hub.disconnect_session(&session);
        assert!(
            hub.terminals.snapshot(&id).unwrap().info.running
                || hub.terminals.list().iter().any(|item| item.id == id)
        );
    }

    #[test]
    fn emergency_stop_blocks_commands_and_leaves_the_shell() {
        if !std::path::Path::new("/dev/pts").exists() {
            return;
        }
        let hub = hub();
        hub.set_level(Level::Full);
        let session = hub.open_session("Codex");
        let created = hub.call_tool(
            &session,
            "unit_agent_workspace_create",
            &json!({ "directory": "/tmp" }),
        );
        let id = created["workspace"]["id"].as_str().unwrap().to_string();
        hub.emergency_stop();
        let blocked = hub.call_tool(
            &session,
            "unit_agent_execute",
            &json!({ "workspace_id": id, "command": "echo AFTER" }),
        );
        assert_eq!(blocked["code"], "AI_DISABLED");
        let status = hub.call_tool("none", "unit_agent_app_status", &json!({}));
        assert_eq!(status["emergencyStop"], true);
        assert!(hub.terminals.list().iter().any(|item| item.id == id));
    }
}
