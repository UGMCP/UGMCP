use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use tauri::{Emitter, State};

use crate::agent::{AgentStatus, LocalAgentService};
use crate::auth::SessionView;
use crate::config::PublicConfig;
use crate::connection::ConnectionSnapshot;
use crate::error::AppError;
use crate::mcp::McpServerInfo;
use crate::state::AppState;
use crate::storage::{Settings, SettingsPatch};
use crate::system::detect_shell;
use crate::terminal::{CreateTerminalRequest, TerminalInfo, TerminalSnapshot, WriteResult};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpConnectRequest {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpCallRequest {
    pub id: String,
    pub name: String,
    pub arguments: Value,
    pub confirmed: bool,
    #[serde(default)]
    pub acknowledged_danger: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSnapshot {
    pub local: AgentStatus,
    pub prysel: AgentStatus,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub platform: &'static str,
    pub arch: &'static str,
    pub mode: &'static str,
    pub config: PublicConfig,
    pub shortcuts: Vec<Shortcut>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shortcut {
    pub keys: &'static str,
    pub action: &'static str,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugInfo {
    pub version: &'static str,
    pub connection: ConnectionSnapshot,
    pub auth_source: String,
    pub authenticated: bool,
    pub shell: String,
    pub terminals: Vec<TerminalInfo>,
    pub mcp: Vec<McpServerInfo>,
    pub app_id: String,
    pub secret_configured: bool,
}

#[tauri::command]
pub fn session_restore(state: State<'_, AppState>) -> Result<SessionView, AppError> {
    state.auth.restore()
}

#[tauri::command]
pub async fn auth_login(state: State<'_, AppState>) -> Result<SessionView, AppError> {
    state.auth.login().await
}

#[tauri::command]
pub fn auth_cancel(state: State<'_, AppState>) -> Result<(), AppError> {
    state.auth.cancel();
    Ok(())
}

#[tauri::command]
pub fn auth_logout(state: State<'_, AppState>) -> Result<SessionView, AppError> {
    state.auth.logout()
}

#[tauri::command]
pub fn connection_snapshot(state: State<'_, AppState>) -> ConnectionSnapshot {
    state.connection.snapshot()
}

#[tauri::command]
pub async fn connection_refresh(
    state: State<'_, AppState>,
) -> Result<ConnectionSnapshot, AppError> {
    let snapshot = state.connection.check(&state.config).await;
    let _ = state.handle.emit("connection-changed", &snapshot);
    Ok(snapshot)
}

#[tauri::command]
pub fn terminal_create(
    state: State<'_, AppState>,
    request: CreateTerminalRequest,
) -> Result<TerminalInfo, AppError> {
    let info = state.terminals.create(request)?;
    let _ = state
        .storage
        .save_workspace(crate::storage::SavedWorkspace {
            id: info.id.clone(),
            title: info.title.clone(),
            cwd: info.cwd.clone(),
            shell: info.shell.clone(),
            created_at: info.created_at,
        });
    Ok(info)
}

#[tauri::command]
pub fn terminal_write(
    state: State<'_, AppState>,
    id: String,
    data: String,
) -> Result<WriteResult, AppError> {
    state.terminals.write(&id, &data)
}

#[tauri::command]
pub fn terminal_resize(
    state: State<'_, AppState>,
    id: String,
    cols: u16,
    rows: u16,
) -> Result<(), AppError> {
    state.terminals.resize(&id, cols, rows)
}

#[tauri::command]
pub fn terminal_close(state: State<'_, AppState>, id: String, force: bool) -> Result<(), AppError> {
    state.terminals.close(&id, force)?;
    let _ = state.storage.remove_workspace(&id);
    Ok(())
}

#[tauri::command]
pub fn terminal_restart(state: State<'_, AppState>, id: String) -> Result<TerminalInfo, AppError> {
    let info = state.terminals.restart(&id)?;
    let _ = state
        .storage
        .save_workspace(crate::storage::SavedWorkspace {
            id: info.id.clone(),
            title: info.title.clone(),
            cwd: info.cwd.clone(),
            shell: info.shell.clone(),
            created_at: info.created_at,
        });
    Ok(info)
}

#[tauri::command]
pub fn terminal_list(state: State<'_, AppState>) -> Vec<TerminalInfo> {
    state.terminals.list()
}

#[tauri::command]
pub fn terminal_snapshot(
    state: State<'_, AppState>,
    id: String,
) -> Result<TerminalSnapshot, AppError> {
    state.terminals.snapshot(&id)
}

#[tauri::command]
pub fn terminal_confirm(
    state: State<'_, AppState>,
    id: String,
    approve: bool,
) -> Result<(), AppError> {
    state.terminals.confirm(&id, approve)
}

#[tauri::command]
pub fn terminal_set_cwd(
    state: State<'_, AppState>,
    id: String,
    cwd: String,
) -> Result<(), AppError> {
    state.terminals.set_cwd(&id, cwd.clone())?;
    state.storage.update_workspace_cwd(&id, &cwd);
    Ok(())
}

#[tauri::command]
pub fn settings_get(state: State<'_, AppState>) -> Settings {
    state.storage.settings()
}

#[tauri::command]
pub fn settings_update(
    state: State<'_, AppState>,
    patch: SettingsPatch,
) -> Result<Settings, AppError> {
    state.storage.update_settings(patch)
}

#[tauri::command]
pub fn mcp_list(state: State<'_, AppState>) -> Vec<McpServerInfo> {
    state.mcp.list()
}

#[tauri::command]
pub fn mcp_connect(
    state: State<'_, AppState>,
    request: McpConnectRequest,
) -> Result<McpServerInfo, AppError> {
    state
        .mcp
        .connect(request.name, request.command, request.args, request.env)
}

#[tauri::command]
pub fn mcp_start(state: State<'_, AppState>, id: String) -> Result<McpServerInfo, AppError> {
    state.mcp.start_saved(&id)
}

#[tauri::command]
pub fn mcp_disconnect(state: State<'_, AppState>, id: String) -> Result<(), AppError> {
    state.mcp.disconnect(&id)
}

#[tauri::command]
pub fn mcp_forget(state: State<'_, AppState>, id: String) -> Result<(), AppError> {
    state.mcp.forget(&id)
}

#[tauri::command]
pub fn mcp_call_tool(
    state: State<'_, AppState>,
    request: McpCallRequest,
) -> Result<Value, AppError> {
    state.mcp.call_tool(
        &request.id,
        &request.name,
        request.arguments,
        request.confirmed,
        request.acknowledged_danger,
    )
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        name: "Unit Agent",
        version: env!("CARGO_PKG_VERSION"),
        platform: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        mode: "desktop",
        config: PublicConfig::from(&state.config),
        shortcuts: vec![
            Shortcut {
                keys: "Ctrl+Shift+T",
                action: "New workspace",
            },
            Shortcut {
                keys: "Ctrl+Shift+N",
                action: "New workspace",
            },
            Shortcut {
                keys: "Ctrl+Shift+W",
                action: "Close active workspace",
            },
            Shortcut {
                keys: "Ctrl+Shift+L",
                action: "Toggle light and dark mode",
            },
            Shortcut {
                keys: "Ctrl+Shift+M",
                action: "MCP and services",
            },
            Shortcut {
                keys: "Ctrl+Shift+C",
                action: "Copy terminal selection",
            },
            Shortcut {
                keys: "Ctrl+Shift+V",
                action: "Paste into the terminal",
            },
        ],
    }
}

#[tauri::command]
pub fn debug_info(state: State<'_, AppState>) -> DebugInfo {
    let session = state.auth.restore().unwrap_or_else(|_| SessionView::none());
    DebugInfo {
        version: env!("CARGO_PKG_VERSION"),
        connection: state.connection.snapshot(),
        auth_source: session.source,
        authenticated: session.authenticated,
        shell: detect_shell(state.storage.settings().shell.as_deref())
            .unwrap_or_else(|_| "unavailable".into()),
        terminals: state.terminals.list(),
        mcp: state.mcp.list(),
        app_id: state.config.app_id.clone(),
        secret_configured: state.config.configured(),
    }
}

#[tauri::command]
pub async fn agent_status(state: State<'_, AppState>) -> Result<AgentSnapshot, AppError> {
    Ok(AgentSnapshot {
        local: LocalAgentService::status(),
        prysel: state.prysel_agent.status(&state.config).await,
    })
}
