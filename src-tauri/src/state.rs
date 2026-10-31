use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, RwLock,
};

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::{
    audit::AuditLogger,
    autostart::AutostartManager,
    crypto::{DpapiMasterKeyStore, SecretCipher},
    error::AppResult,
    mcp::build_mcp_router,
    repository::HostRepository,
    security::{default_data_dir, prepare_data_directory},
    service::HostService,
    settings::{AppSettingsStore, RuntimeSettings},
    ssh::{SshExecutor, SshManager},
};

pub struct McpRuntimeState {
    running: AtomicBool,
    error: RwLock<Option<String>>,
}

impl Default for McpRuntimeState {
    fn default() -> Self {
        Self {
            running: AtomicBool::new(false),
            error: RwLock::new(None),
        }
    }
}

impl McpRuntimeState {
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn error(&self) -> Option<String> {
        self.error.read().ok().and_then(|value| value.clone())
    }

    fn set_running(&self, running: bool, error: Option<String>) {
        self.running.store(running, Ordering::SeqCst);
        if let Ok(mut slot) = self.error.write() {
            *slot = error;
        }
    }
}

pub struct AppState {
    pub data_dir: std::path::PathBuf,
    pub repository: Arc<HostRepository>,
    pub service: Arc<HostService>,
    pub ssh: Arc<SshManager>,
    pub settings: RuntimeSettings,
    pub autostart: AutostartManager,
    pub mcp: McpRuntimeState,
    cancellation: CancellationToken,
}

impl AppState {
    pub fn create() -> AppResult<Arc<Self>> {
        let data_dir = prepare_data_directory(&default_data_dir())?;
        crate::logging::init(&data_dir);

        let key_store = DpapiMasterKeyStore::new(data_dir.join("master.key.dpapi"));
        let key = key_store.load_or_create()?;
        let cipher = SecretCipher::new(&key)?;
        let settings_store = AppSettingsStore::new(&data_dir, cipher);
        let settings = settings_store.load_or_create()?;
        let cipher = SecretCipher::new(&key)?;
        let repository = Arc::new(HostRepository::open(&data_dir, cipher)?);
        let ssh = Arc::new(SshManager::new());
        let audit = Arc::new(AuditLogger::new(&data_dir));
        let service = Arc::new(HostService::new(
            repository.clone(),
            ssh.clone(),
            Some(audit),
        ));
        let autostart = AutostartManager::current();

        tracing::info!(data_dir = %data_dir.display(), "SSH MCP starting");
        Ok(Arc::new(Self {
            data_dir,
            repository,
            service,
            ssh,
            settings,
            autostart,
            mcp: McpRuntimeState::default(),
            cancellation: CancellationToken::new(),
        }))
    }

    pub fn start_mcp(self: &Arc<Self>) {
        let state = self.clone();
        let token = self.settings.mcp_token.clone();
        let cancellation = self.cancellation.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = run_mcp_server(state.clone(), token, cancellation).await {
                tracing::error!(%error, "MCP server stopped");
                state.mcp.set_running(false, Some(error.to_string()));
            }
        });
    }

    pub fn request_shutdown(&self) {
        self.cancellation.cancel();
        self.mcp.set_running(false, None);
        tracing::info!("SSH MCP shutdown requested");
    }

    pub async fn shutdown(&self) {
        self.request_shutdown();
        self.ssh.close_all().await;
    }

    pub fn shutdown_blocking(&self) {
        let _ = tauri::async_runtime::block_on(self.shutdown());
    }
}

async fn run_mcp_server(
    state: Arc<AppState>,
    token: String,
    cancellation: CancellationToken,
) -> AppResult<()> {
    let address = format!("{}:{}", state.settings.mcp_host, state.settings.mcp_port);
    let listener = TcpListener::bind(&address)
        .await
        .map_err(|err| crate::error::AppError::Connection(format!("MCP 监听失败 {address}: {err}")))?;
    let router = build_mcp_router(state.service.clone(), token, cancellation.clone());
    state.mcp.set_running(true, None);
    tracing::info!(%address, "MCP server listening");
    axum::serve(listener, router)
        .with_graceful_shutdown(async move { cancellation.cancelled_owned().await })
        .await
        .map_err(|err| crate::error::AppError::Connection(format!("MCP 服务异常退出: {err}")))?;
    state.mcp.set_running(false, None);
    Ok(())
}
