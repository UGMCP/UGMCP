mod agent;
mod auth;
mod commands;
mod config;
mod connection;
mod control;
mod error;
mod mcp;
mod mcp_host;
mod safety;
mod server;
mod state;
mod storage;
mod system;
mod terminal;
mod util;

pub use mcp_host::{cli, mcp_main, CLI_HELP};
pub use server::serve;

use std::time::Duration;

use tauri::{Emitter, Manager, RunEvent};

use crate::state::{restore_window, AppState};

fn install_brand_icon(app: &tauri::App) {
    // Official Unit Agent mark: public/unit_agent.png, also shipped as favicon.ico.
    let bytes = include_bytes!("../icons/icon.png");
    match tauri::image::Image::from_bytes(bytes) {
        Ok(icon) => {
            if let Some(window) = app.get_webview_window("main") {
                if let Err(err) = window.set_icon(icon) {
                    log::warn!("could not set the Unit Agent window icon: {err}");
                }
            }
        }
        Err(err) => log::warn!("could not decode the Unit Agent icon: {err}"),
    }
}

pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .init();

    tauri::Builder::default()
        .setup(|app| {
            install_brand_icon(app);
            let state = AppState::initialize(app.handle().clone());
            restore_window(app, &state.storage.settings());
            let connection = state.connection.clone();
            let config = state.config.clone();
            let handle = app.handle().clone();
            app.manage(state);
            tauri::async_runtime::spawn(async move {
                loop {
                    let snapshot = connection.check(&config).await;
                    let _ = handle.emit("connection-changed", &snapshot);
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::session_restore,
            commands::auth_login,
            commands::auth_cancel,
            commands::auth_logout,
            commands::connection_snapshot,
            commands::connection_refresh,
            commands::terminal_create,
            commands::terminal_write,
            commands::terminal_resize,
            commands::terminal_close,
            commands::terminal_restart,
            commands::terminal_list,
            commands::terminal_snapshot,
            commands::terminal_confirm,
            commands::terminal_set_cwd,
            commands::settings_get,
            commands::settings_update,
            commands::mcp_list,
            commands::mcp_connect,
            commands::mcp_start,
            commands::mcp_disconnect,
            commands::mcp_forget,
            commands::mcp_call_tool,
            commands::app_info,
            commands::debug_info,
            commands::agent_status,
            commands::host_identity,
            commands::ai_status,
            commands::ai_activity,
            commands::ai_confirm,
            commands::ai_emergency_stop,
            commands::ai_resume_control,
        ])
        .build(tauri::generate_context!())
        .expect("failed to start Unit Agent")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                if let Some(state) = app.try_state::<AppState>() {
                    state.shutdown();
                }
            }
        });
}
