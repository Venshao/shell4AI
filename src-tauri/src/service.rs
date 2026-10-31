use std::sync::Arc;

use serde_json::{Map, Value};
#[cfg(test)]
use serde_json::json;
use zeroize::Zeroizing;

use crate::{
    audit::AuditLogger,
    domain::{HostRecord, HostView},
    error::{AppError, AppResult},
    repository::HostRepository,
    ssh::{ServerHostKey, SshExecutor},
};

pub struct HostService {
    repository: Arc<HostRepository>,
    executor: Arc<dyn SshExecutor>,
    audit: Option<Arc<AuditLogger>>,
}

impl HostService {
    pub fn new(
        repository: Arc<HostRepository>,
        executor: Arc<dyn SshExecutor>,
        audit: Option<Arc<AuditLogger>>,
    ) -> Self {
        Self {
            repository,
            executor,
            audit,
        }
    }

    pub fn list_hosts(&self) -> AppResult<Vec<HostView>> {
        self.repository.list_views()
    }

    pub async fn probe_host(&self, selector: &str) -> AppResult<ServerHostKey> {
        let record = self.repository.get(selector)?;
        self.executor.probe(&record).await
    }

    pub async fn trust_host(
        &self,
        host_id: &str,
        fingerprint: &str,
        host_key_line: &str,
    ) -> AppResult<HostView> {
        let record = self.repository.get_by_id(host_id)?;
        let current = self.executor.probe(&record).await?;
        if current.fingerprint != fingerprint || current.host_key_line != host_key_line {
            return Err(AppError::Security(
                "服务器指纹在确认期间发生变化，请重新检查".into(),
            ));
        }
        let updated = self.repository.set_host_key(
            host_id,
            &current.fingerprint,
            &current.host_key_line,
        )?;
        Ok(updated.view())
    }

    pub async fn test_connection(&self, selector: &str) -> AppResult<()> {
        let (record, password) = self.ready_host(selector)?;
        match self
            .executor
            .test_connection(&record, password.as_str())
            .await
        {
            Ok(()) => {
                self.audit(&record, "test_connection", true, Map::new());
                Ok(())
            }
            Err(error) => {
                self.audit_error(&record, "test_connection", &error);
                Err(error)
            }
        }
    }

    pub async fn exec(
        &self,
        selector: &str,
        command: &str,
        timeout_seconds: f64,
        max_output_bytes: usize,
    ) -> AppResult<Value> {
        if command.trim().is_empty() {
            return Err(AppError::Validation("命令不能为空".into()));
        }
        if !(1.0..=1800.0).contains(&timeout_seconds) {
            return Err(AppError::Validation(
                "timeout_seconds 必须在 1 到 1800 之间".into(),
            ));
        }
        if !(1024..=4 * 1024 * 1024).contains(&max_output_bytes) {
            return Err(AppError::Validation(
                "max_output_bytes 必须在 1024 到 4194304 之间".into(),
            ));
        }

        let (record, password) = self.ready_host(selector)?;
        match self
            .executor
            .exec(
                &record,
                password.as_str(),
                command,
                timeout_seconds,
                max_output_bytes,
            )
            .await
        {
            Ok(result) => {
                let mut details = Map::new();
                details.insert("exit_code".into(), result["exit_code"].clone());
                details.insert("elapsed_ms".into(), result["elapsed_ms"].clone());
                details.insert(
                    "command_length".into(),
                    Value::Number(command.len().into()),
                );
                self.audit(
                    &record,
                    "exec",
                    result["exit_code"].as_i64() == Some(0),
                    details,
                );
                Ok(result)
            }
            Err(error) => {
                self.audit_error(&record, "exec", &error);
                Err(error)
            }
        }
    }

    pub async fn read_file(
        &self,
        selector: &str,
        path: &str,
        max_bytes: usize,
    ) -> AppResult<Value> {
        if !(1..=2 * 1024 * 1024).contains(&max_bytes) {
            return Err(AppError::Validation(
                "max_bytes 必须在 1 到 2097152 之间".into(),
            ));
        }
        let (record, password) = self.ready_host(selector)?;
        match self
            .executor
            .read_text_file(&record, password.as_str(), path, max_bytes)
            .await
        {
            Ok(result) => {
                let mut details = Map::new();
                details.insert("bytes".into(), result["size"].clone());
                self.audit(&record, "read_file", true, details);
                Ok(result)
            }
            Err(error) => {
                self.audit_error(&record, "read_file", &error);
                Err(error)
            }
        }
    }

    pub async fn list_dir(&self, selector: &str, path: &str) -> AppResult<Value> {
        let (record, password) = self.ready_host(selector)?;
        match self
            .executor
            .list_dir(&record, password.as_str(), path)
            .await
        {
            Ok(result) => {
                let mut details = Map::new();
                details.insert(
                    "entries".into(),
                    Value::Number(
                        result["entries"]
                            .as_array()
                            .map(Vec::len)
                            .unwrap_or_default()
                            .into(),
                    ),
                );
                self.audit(&record, "list_dir", true, details);
                Ok(result)
            }
            Err(error) => {
                self.audit_error(&record, "list_dir", &error);
                Err(error)
            }
        }
    }

    fn ready_host(&self, selector: &str) -> AppResult<(HostRecord, Zeroizing<String>)> {
        let record = self.repository.get(selector)?;
        if !record.enabled {
            return Err(AppError::Validation(format!("主机 {} 已禁用", record.alias)));
        }
        if record.host_key_fingerprint.is_none() || record.host_key_line.is_none() {
            return Err(AppError::Security(format!(
                "主机 {} 尚未确认 SSH 主机指纹",
                record.alias
            )));
        }
        if !record.password_configured {
            return Err(AppError::Validation(format!(
                "主机 {} 未配置密码",
                record.alias
            )));
        }
        let password = self.repository.password_for(&record.id)?;
        Ok((record, password))
    }

    fn audit(
        &self,
        record: &HostRecord,
        action: &str,
        success: bool,
        details: Map<String, Value>,
    ) {
        if let Some(audit) = &self.audit {
            audit.record(action, &record.id, &record.alias, success, details);
        }
    }

    fn audit_error(&self, record: &HostRecord, action: &str, error: &AppError) {
        let mut details = Map::new();
        details.insert(
            "error_type".into(),
            Value::String(error_type(error).into()),
        );
        self.audit(record, action, false, details);
    }
}

fn error_type(error: &AppError) -> &'static str {
    match error {
        AppError::Validation(_) => "ValidationError",
        AppError::NotFound(_) => "NotFoundError",
        AppError::Conflict(_) => "ConflictError",
        AppError::Security(_) => "SecurityError",
        AppError::Connection(_) => "ConnectionError",
        AppError::Data(_) => "DataError",
        AppError::Io(_) => "IOError",
        AppError::Json(_) => "JSONError",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::*;
    use crate::{crypto::SecretCipher, domain::HostInput};

    #[derive(Default)]
    struct FakeExecutor {
        calls: Mutex<Vec<(String, String, String)>>,
    }

    #[async_trait]
    impl SshExecutor for FakeExecutor {
        async fn probe(&self, _record: &HostRecord) -> AppResult<ServerHostKey> {
            Ok(ServerHostKey {
                fingerprint: "SHA256:new".into(),
                host_key_line: "ssh-ed25519 AAAA".into(),
            })
        }

        async fn test_connection(&self, record: &HostRecord, password: &str) -> AppResult<()> {
            self.calls
                .lock()
                .unwrap()
                .push((record.alias.clone(), password.into(), "test".into()));
            Ok(())
        }

        async fn exec(
            &self,
            record: &HostRecord,
            password: &str,
            command: &str,
            _timeout_seconds: f64,
            _max_output_bytes: usize,
        ) -> AppResult<Value> {
            self.calls
                .lock()
                .unwrap()
                .push((record.alias.clone(), password.into(), command.into()));
            Ok(json!({
                "exit_code": 0,
                "stdout": "ok\n",
                "stderr": "",
                "truncated": false,
                "elapsed_ms": 1,
            }))
        }

        async fn read_text_file(
            &self,
            _record: &HostRecord,
            _password: &str,
            path: &str,
            _max_bytes: usize,
        ) -> AppResult<Value> {
            Ok(json!({"path": path, "content": "hello", "truncated": false, "size": 5}))
        }

        async fn list_dir(
            &self,
            _record: &HostRecord,
            _password: &str,
            path: &str,
        ) -> AppResult<Value> {
            Ok(json!({"path": path, "entries": []}))
        }

        async fn disconnect(&self, _host_id: &str) {}

        async fn close_all(&self) {}
    }

    fn setup() -> (tempfile::TempDir, Arc<HostRepository>, Arc<FakeExecutor>, HostService) {
        let dir = tempfile::tempdir().expect("tempdir");
        let cipher = SecretCipher::new(&[5_u8; 32]).expect("cipher");
        let repository = Arc::new(HostRepository::open(dir.path(), cipher).expect("repo"));
        let executor = Arc::new(FakeExecutor::default());
        let service = HostService::new(repository.clone(), executor.clone(), None);
        (dir, repository, executor, service)
    }

    fn input() -> HostInput {
        HostInput {
            alias: "prod".into(),
            hostname: "10.0.0.10".into(),
            port: 22,
            username: "root".into(),
            password: Some("secret".into()),
            enabled: true,
        }
    }

    #[tokio::test]
    async fn rejects_host_until_fingerprint_is_trusted() {
        let (_dir, repository, _, service) = setup();
        repository.create(&input()).expect("host");
        assert!(service.exec("prod", "hostname", 10.0, 1024).await.is_err());
    }

    #[tokio::test]
    async fn resolves_password_and_selector_for_exec() {
        let (_dir, repository, executor, service) = setup();
        let record = repository.create(&input()).expect("host");
        repository
            .set_host_key(&record.id, "SHA256:test", "ssh-ed25519 AAAA")
            .expect("trust");

        let result = service.exec("PROD", "hostname", 10.0, 1024).await.expect("exec");
        assert_eq!(result["stdout"], "ok\n");
        assert_eq!(
            executor.calls.lock().unwrap().as_slice(),
            &[("prod".into(), "secret".into(), "hostname".into())]
        );
    }

    #[tokio::test]
    async fn trust_rejects_fingerprint_changed_during_confirmation() {
        let (_dir, repository, _, service) = setup();
        let record = repository.create(&input()).expect("host");
        assert!(service
            .trust_host(&record.id, "SHA256:old", "ssh-ed25519 AAAA")
            .await
            .is_err());
    }
}
