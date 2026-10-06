use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager};

use crate::agent::PryselAgentService;
use crate::auth::AuthService;
use crate::config::PryselConfig;
use crate::connection::ConnectionService;
use crate::mcp::McpManager;
use crate::storage::Storage;
use crate::terminal::{TerminalEvent, TerminalManager};

pub struct AppState {
    pub handle: AppHandle,
    pub terminals: Arc<TerminalManager>,
    pub auth: Arc<AuthService>,
    pub connection: Arc<ConnectionService>,
    pub storage: Arc<Storage>,
    pub mcp: Arc<McpManager>,
    pub config: PryselConfig,
    pub prysel_agent: PryselAgentService,
}

impl AppState {
    pub fn initialize(handle: AppHandle) -> Self {
        let config = PryselConfig::load();
        let storage = Arc::new(Storage::open());
        let terminals = Arc::new(TerminalManager::new());
        let auth = Arc::new(AuthService::new(config.clone(), storage.clone()));
        let connection = Arc::new(ConnectionService::new());
        let mcp = Arc::new(McpManager::new(storage.clone()));
        let storage_for_events = storage.clone();
        let emit_handle = handle.clone();
        terminals.set_listener(Arc::new(move |event: TerminalEvent| {
            if let TerminalEvent::Cwd { id, cwd } = &event {
                storage_for_events.update_workspace_cwd(id, cwd);
            }
            let _ = emit_handle.emit("terminal-event", &event);
        }));
        Self {
            handle,
            terminals,
            auth,
            connection,
            storage,
            mcp,
            config,
            prysel_agent: PryselAgentService::new(),
        }
    }

    pub fn shutdown(&self) {
        self.auth.cancel();
        self.mcp.shutdown();
        self.terminals.shutdown();
    }
}

pub fn restore_window(app: &tauri::App, settings: &crate::storage::Settings) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if settings.window.maximized {
        let _ = window.maximize();
        return;
    }
    let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
        settings.window.width,
        settings.window.height,
    )));
    if let (Some(x), Some(y)) = (settings.window.x, settings.window.y) {
        let _ = window.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(x, y)));
    }
}
