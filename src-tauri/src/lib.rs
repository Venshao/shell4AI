use std::sync::Arc;

use tauri::{Manager, RunEvent};

mod audit;
mod autostart;
mod commands;
mod crypto;
mod domain;
mod error;
mod logging;
mod mcp;
mod repository;
mod security;
mod service;
mod settings;
mod ssh;
mod state;
mod storage;
mod tray;

use state::AppState;

pub fn run() {
    let state = AppState::create().expect("failed to initialize SSH MCP state");
    state.start_mcp();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .manage(state.clone())
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let hide_window = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = hide_window.hide();
                    }
                });
                if !std::env::args().any(|arg| arg == "--autostart") {
                    window.show()?;
                    window.set_focus()?;
                }
            }
            tray::init(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_hosts,
            commands::save_host,
            commands::delete_host,
            commands::probe_host,
            commands::trust_host,
            commands::test_host,
            commands::get_app_state,
            commands::get_autostart,
            commands::set_autostart,
            commands::quit_app,
        ]);

    let app = builder
        .build(tauri::generate_context!())
        .expect("failed to build SSH MCP application");

    app.run(move |_app_handle, event| match event {
        RunEvent::ExitRequested { .. } => state.request_shutdown(),
        RunEvent::Exit => state.shutdown_blocking(),
        _ => {}
    });
}
