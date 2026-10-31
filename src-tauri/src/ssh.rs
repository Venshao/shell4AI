use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use russh::{
    client::{self, Handle},
    keys::{HashAlg, PublicKeyOrCertificate},
    ChannelMsg, Disconnect, Sig,
};
use russh_sftp::client::SftpSession;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::{
    io::AsyncReadExt,
    sync::Mutex,
    time::timeout,
};

use crate::{
    domain::HostRecord,
    error::{AppError, AppResult},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerHostKey {
    pub fingerprint: String,
    pub host_key_line: String,
}

#[async_trait]
pub trait SshExecutor: Send + Sync {
    async fn probe(&self, record: &HostRecord) -> AppResult<ServerHostKey>;
    async fn test_connection(&self, record: &HostRecord, password: &str) -> AppResult<()>;
    async fn exec(
        &self,
        record: &HostRecord,
        password: &str,
        command: &str,
        timeout_seconds: f64,
        max_output_bytes: usize,
    ) -> AppResult<Value>;
    async fn read_text_file(
        &self,
        record: &HostRecord,
        password: &str,
        path: &str,
        max_bytes: usize,
    ) -> AppResult<Value>;
    async fn list_dir(
        &self,
        record: &HostRecord,
        password: &str,
        path: &str,
    ) -> AppResult<Value>;
    async fn disconnect(&self, host_id: &str);
    async fn close_all(&self);
}

struct ClientHandler {
    expected_fingerprint: Option<String>,
    captured: Arc<StdMutex<Option<ServerHostKey>>>,
    fingerprint_mismatch: Arc<AtomicBool>,
}

impl client::Handler for ClientHandler {
    type Error = anyhow::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = server_public_key.public_key();
        let captured = ServerHostKey {
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
            host_key_line: key.to_openssh()?,
        };
        if let Ok(mut slot) = self.captured.lock() {
            *slot = Some(captured.clone());
        }
        let accepted = self
            .expected_fingerprint
            .as_ref()
            .is_none_or(|expected| expected == &captured.fingerprint);
        if !accepted {
            self.fingerprint_mismatch.store(true, Ordering::SeqCst);
        }
        Ok(accepted)
    }
}

pub struct SshManager {
    config: Arc<client::Config>,
    connections: Mutex<HashMap<String, Arc<Handle<ClientHandler>>>>,
    connect_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl SshManager {
    pub fn new() -> Self {
        let config = client::Config {
            inactivity_timeout: Some(Duration::from_secs(120)),
            keepalive_interval: Some(Duration::from_secs(30)),
            keepalive_max: 3,
            nodelay: true,
            ..Default::default()
        };
        Self {
            config: Arc::new(config),
            connections: Mutex::new(HashMap::new()),
            connect_locks: Mutex::new(HashMap::new()),
        }
    }

    async fn connection(
        &self,
        record: &HostRecord,
        password: &str,
    ) -> AppResult<Arc<Handle<ClientHandler>>> {
        if let Some(handle) = self.connections.lock().await.get(&record.id).cloned() {
            if !handle.is_closed() {
                return Ok(handle);
            }
        }

        let host_lock = {
            let mut locks = self.connect_locks.lock().await;
            locks
                .entry(record.id.clone())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _guard = host_lock.lock().await;

        if let Some(handle) = self.connections.lock().await.get(&record.id).cloned() {
            if !handle.is_closed() {
                return Ok(handle);
            }
        }
        self.connections.lock().await.remove(&record.id);

        let expected = record
            .host_key_fingerprint
            .clone()
            .ok_or_else(|| AppError::Security("SSH 主机指纹尚未确认".into()))?;
        let captured = Arc::new(StdMutex::new(None));
        let mismatch = Arc::new(AtomicBool::new(false));
        let handler = ClientHandler {
            expected_fingerprint: Some(expected),
            captured: captured.clone(),
            fingerprint_mismatch: mismatch.clone(),
        };
        let address = (record.hostname.as_str(), record.port);
        let mut handle = timeout(
            Duration::from_secs(25),
            client::connect(self.config.clone(), address, handler),
        )
        .await
        .map_err(|_| AppError::Connection("SSH 连接超时".into()))?
        .map_err(|err| {
            if mismatch.load(Ordering::SeqCst) {
                AppError::Security("SSH 主机指纹不匹配，连接已拒绝".into())
            } else {
                AppError::Connection(format!("SSH 握手失败: {err}"))
            }
        })?;

        let auth = timeout(
            Duration::from_secs(20),
            handle.authenticate_password(record.username.clone(), password.to_owned()),
        )
        .await
        .map_err(|_| AppError::Connection("SSH 登录超时".into()))?
        .map_err(|err| AppError::Connection(format!("SSH 登录失败: {err}")))?;
        if !auth.success() {
            return Err(AppError::Connection("SSH 用户名或密码错误".into()));
        }
        let handle = Arc::new(handle);
        self.connections
            .lock()
            .await
            .insert(record.id.clone(), handle.clone());
        Ok(handle)
    }

    async fn execute_once(
        &self,
        handle: &Handle<ClientHandler>,
        command: &str,
        timeout_seconds: f64,
        max_output_bytes: usize,
        started: Instant,
    ) -> AppResult<Value> {
        let mut channel = handle
            .channel_open_session()
            .await
            .map_err(|err| AppError::Connection(format!("无法打开 SSH 通道: {err}")))?;
        channel
            .exec(true, command.as_bytes().to_vec())
            .await
            .map_err(|err| AppError::Connection(format!("无法执行 SSH 命令: {err}")))?;

        let collect = async {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let mut exit_code = -1_i32;
            let mut truncated = false;
            while let Some(message) = channel.wait().await {
                match message {
                    ChannelMsg::Data { data } => {
                        append_limited(&mut stdout, &data, max_output_bytes, &mut truncated);
                    }
                    ChannelMsg::ExtendedData { data, ext: 1 } => {
                        append_limited(&mut stderr, &data, max_output_bytes, &mut truncated);
                    }
                    ChannelMsg::ExitStatus { exit_status } => {
                        exit_code = exit_status as i32;
                    }
                    ChannelMsg::Eof | ChannelMsg::Close => {}
                    _ => {}
                }
            }
            (stdout, stderr, exit_code, truncated)
        };

        match timeout(Duration::from_secs_f64(timeout_seconds), collect).await {
            Ok((stdout, stderr, exit_code, truncated)) => Ok(json!({
                "exit_code": exit_code,
                "stdout": String::from_utf8_lossy(&stdout),
                "stderr": String::from_utf8_lossy(&stderr),
                "truncated": truncated,
                "elapsed_ms": started.elapsed().as_millis() as u64,
            })),
            Err(_) => {
                let _ = channel.signal(Sig::TERM).await;
                let _ = channel.close().await;
                Err(AppError::Connection(format!(
                    "命令超时 ({timeout_seconds}s)"
                )))
            }
        }
    }
}

#[async_trait]
impl SshExecutor for SshManager {
    async fn probe(&self, record: &HostRecord) -> AppResult<ServerHostKey> {
        let captured = Arc::new(StdMutex::new(None));
        let handler = ClientHandler {
            expected_fingerprint: None,
            captured: captured.clone(),
            fingerprint_mismatch: Arc::new(AtomicBool::new(false)),
        };
        let address = (record.hostname.as_str(), record.port);
        let handle = timeout(
            Duration::from_secs(15),
            client::connect(self.config.clone(), address, handler),
        )
        .await
        .map_err(|_| AppError::Connection("读取 SSH 主机密钥超时".into()))?
        .map_err(|err| AppError::Connection(format!("SSH 主机密钥读取失败: {err}")))?;
        let _ = handle
            .disconnect(Disconnect::ByApplication, "", "en")
            .await;
        let key = captured
            .lock()
            .map_err(|_| AppError::Connection("SSH 主机密钥状态锁已损坏".into()))?
            .clone()
            .ok_or_else(|| AppError::Connection("远程服务器未返回 SSH 主机密钥".into()))?;
        Ok(key)
    }

    async fn test_connection(&self, record: &HostRecord, password: &str) -> AppResult<()> {
        let result = self
            .exec(record, password, "printf ssh-mcp-ok", 20.0, 1024)
            .await?;
        if result["exit_code"] != 0 || !result["stdout"].as_str().unwrap_or_default().contains("ssh-mcp-ok") {
            return Err(AppError::Connection("SSH 登录成功，但测试命令执行失败".into()));
        }
        Ok(())
    }

    async fn exec(
        &self,
        record: &HostRecord,
        password: &str,
        command: &str,
        timeout_seconds: f64,
        max_output_bytes: usize,
    ) -> AppResult<Value> {
        let started = Instant::now();
        let handle = self.connection(record, password).await?;
        match self
            .execute_once(&handle, command, timeout_seconds, max_output_bytes, started)
            .await
        {
            Ok(result) => Ok(result),
            Err(first_error) => {
                self.disconnect(&record.id).await;
                let handle = self.connection(record, password).await?;
                self.execute_once(&handle, command, timeout_seconds, max_output_bytes, started)
                    .await
                    .map_err(|_| first_error)
            }
        }
    }

    async fn read_text_file(
        &self,
        record: &HostRecord,
        password: &str,
        path: &str,
        max_bytes: usize,
    ) -> AppResult<Value> {
        let handle = self.connection(record, password).await?;
        let channel = handle
            .channel_open_session()
            .await
            .map_err(|err| AppError::Connection(format!("无法打开 SFTP 通道: {err}")))?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|err| AppError::Connection(format!("无法启动 SFTP: {err}")))?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(|err| AppError::Connection(format!("SFTP 初始化失败: {err}")))?;
        let file = sftp
            .open(path)
            .await
            .map_err(|err| AppError::Connection(format!("无法打开远程文件: {err}")))?;
        let mut data = Vec::new();
        file.take(max_bytes as u64 + 1)
            .read_to_end(&mut data)
            .await
            .map_err(|err| AppError::Connection(format!("读取远程文件失败: {err}")))?;
        let truncated = data.len() > max_bytes;
        data.truncate(max_bytes);
        Ok(json!({
            "path": path,
            "content": String::from_utf8_lossy(&data),
            "truncated": truncated,
            "size": data.len(),
        }))
    }

    async fn list_dir(
        &self,
        record: &HostRecord,
        password: &str,
        path: &str,
    ) -> AppResult<Value> {
        let handle = self.connection(record, password).await?;
        let channel = handle
            .channel_open_session()
            .await
            .map_err(|err| AppError::Connection(format!("无法打开 SFTP 通道: {err}")))?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|err| AppError::Connection(format!("无法启动 SFTP: {err}")))?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(|err| AppError::Connection(format!("SFTP 初始化失败: {err}")))?;
        let entries = sftp
            .read_dir(path)
            .await
            .map_err(|err| AppError::Connection(format!("无法读取远程目录: {err}")))?;
        let mut items = entries
            .map(|entry| {
                let metadata = entry.metadata();
                let file_type = entry.file_type();
                json!({
                    "name": entry.file_name(),
                    "size": metadata.size,
                    "mtime": metadata.mtime,
                    "mode": metadata.permissions.map(|mode| format!("0o{mode:o}")),
                    "is_dir": file_type.is_dir(),
                })
            })
            .collect::<Vec<_>>();
        items.sort_by(|left, right| {
            let left_dir = left["is_dir"].as_bool().unwrap_or(false);
            let right_dir = right["is_dir"].as_bool().unwrap_or(false);
            right_dir
                .cmp(&left_dir)
                .then_with(|| left["name"].as_str().cmp(&right["name"].as_str()))
        });
        Ok(json!({"path": path, "entries": items}))
    }

    async fn disconnect(&self, host_id: &str) {
        let handle = self.connections.lock().await.remove(host_id);
        if let Some(handle) = handle {
            let _ = handle
                .disconnect(Disconnect::ByApplication, "", "en")
                .await;
        }
    }

    async fn close_all(&self) {
        let handles = {
            let mut connections = self.connections.lock().await;
            connections.drain().map(|(_, handle)| handle).collect::<Vec<_>>()
        };
        for handle in handles {
            let _ = handle
                .disconnect(Disconnect::ByApplication, "", "en")
                .await;
        }
    }
}

fn append_limited(target: &mut Vec<u8>, data: &[u8], limit: usize, truncated: &mut bool) {
    if target.len() >= limit {
        *truncated = true;
        return;
    }
    let remaining = limit - target.len();
    if data.len() > remaining {
        target.extend_from_slice(&data[..remaining]);
        *truncated = true;
    } else {
        target.extend_from_slice(data);
    }
}

pub fn error_json(error: &AppError) -> Value {
    let mut object = Map::new();
    object.insert("ok".into(), Value::Bool(false));
    object.insert("error".into(), Value::String(error.to_string()));
    object.insert(
        "error_type".into(),
        Value::String(match error {
            AppError::Validation(_) => "ValidationError",
            AppError::NotFound(_) => "NotFoundError",
            AppError::Conflict(_) => "ConflictError",
            AppError::Security(_) => "SecurityError",
            AppError::Connection(_) => "ConnectionError",
            AppError::Data(_) => "DataError",
            AppError::Io(_) => "IOError",
            AppError::Json(_) => "JSONError",
        }
        .into()),
    );
    Value::Object(object)
}
