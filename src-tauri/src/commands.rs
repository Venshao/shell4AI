use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::{
    domain::{HostPayload, HostView, ProbeResult},
    state::AppState,
    ssh::SshExecutor,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectorPayload {
    pub host: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostIdPayload {
    pub host_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TogglePayload {
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrustPayload {
    pub host_id: String,
    pub fingerprint: String,
    pub host_key_line: String,
}

#[derive(Debug, Serialize)]
pub struct McpAccessView {
    pub endpoint: String,
    pub token: String,
    pub running: bool,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AppStateView {
    pub data_dir: String,
    pub mcp: McpAccessView,
    pub startup_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AutostartState {
    pub enabled: bool,
    pub command: String,
}

#[tauri::command]
pub async fn list_hosts(state: State<'_, Arc<AppState>>) -> Result<Vec<HostView>, String> {
    state.service.list_hosts().map_err(error_text)
}

#[tauri::command]
pub async fn save_host(
    body: HostPayload,
    state: State<'_, Arc<AppState>>,
) -> Result<HostView, String> {
    let host_id = body.host_id.clone();
    let input = body.validate().map_err(error_text)?;
    let record = if let Some(host_id) = host_id {
        let record = state.repository.update(&host_id, &input).map_err(error_text)?;
        state.ssh.disconnect(&host_id).await;
        record
    } else {
        state.repository.create(&input).map_err(error_text)?
    };
    Ok(record.view())
}

#[tauri::command]
pub async fn delete_host(
    body: HostIdPayload,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    state.repository.delete(&body.host_id).map_err(error_text)?;
    state.ssh.disconnect(&body.host_id).await;
    Ok(())
}

#[tauri::command]
pub async fn probe_host(
    body: SelectorPayload,
    state: State<'_, Arc<AppState>>,
) -> Result<ProbeResult, String> {
    let record = state.repository.get(&body.host).map_err(error_text)?;
    let key = state.service.probe_host(&body.host).await.map_err(error_text)?;
    Ok(ProbeResult {
        host_id: record.id,
        alias: record.alias,
        fingerprint: key.fingerprint,
        host_key_line: key.host_key_line,
    })
}

#[tauri::command]
pub async fn trust_host(
    body: TrustPayload,
    state: State<'_, Arc<AppState>>,
) -> Result<HostView, String> {
    state
        .service
        .trust_host(&body.host_id, &body.fingerprint, &body.host_key_line)
        .await
        .map_err(error_text)
}

#[tauri::command]
pub async fn test_host(
    body: SelectorPayload,
    state: State<'_, Arc<AppState>>,
) -> Result<String, String> {
    state
        .service
        .test_connection(&body.host)
        .await
        .map_err(error_text)?;
    Ok("连接成功".into())
}

#[tauri::command]
pub async fn get_app_state(state: State<'_, Arc<AppState>>) -> Result<AppStateView, String> {
    Ok(AppStateView {
        data_dir: state.data_dir.display().to_string(),
        mcp: McpAccessView {
            endpoint: format!(
                "http://{}:{}/mcp",
                state.settings.mcp_host, state.settings.mcp_port
            ),
            token: state.settings.mcp_token.clone(),
            running: state.mcp.is_running(),
            error: state.mcp.error(),
        },
        startup_error: None,
    })
}

#[tauri::command]
pub fn get_autostart(state: State<'_, Arc<AppState>>) -> AutostartState {
    AutostartState {
        enabled: state.autostart.is_enabled(),
        command: state.autostart.command(),
    }
}

#[tauri::command]
pub fn set_autostart(
    body: TogglePayload,
    state: State<'_, Arc<AppState>>,
) -> Result<AutostartState, String> {
    let enabled = state
        .autostart
        .set_enabled(body.enabled)
        .map_err(error_text)?;
    Ok(AutostartState {
        enabled,
        command: state.autostart.command(),
    })
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    app.exit(0);
}

fn error_text(error: impl ToString) -> String {
    error.to_string()
}
