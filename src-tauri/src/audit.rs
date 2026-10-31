use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

use chrono::Utc;
use serde_json::{Map, Value};

pub struct AuditLogger {
    path: PathBuf,
    lock: Mutex<()>,
}

impl AuditLogger {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join("audit.jsonl"),
            lock: Mutex::new(()),
        }
    }

    pub fn record(
        &self,
        action: &str,
        host_id: &str,
        host_alias: &str,
        success: bool,
        details: Map<String, Value>,
    ) {
        let mut payload = Map::new();
        payload.insert("timestamp".into(), Value::String(Utc::now().to_rfc3339()));
        payload.insert("action".into(), Value::String(action.into()));
        payload.insert("host_id".into(), Value::String(host_id.into()));
        payload.insert("host_alias".into(), Value::String(host_alias.into()));
        payload.insert("success".into(), Value::Bool(success));
        payload.extend(details);

        let Ok(line) = serde_json::to_string(&payload) else {
            tracing::error!("failed to serialize audit event");
            return;
        };
        let _guard = self.lock.lock().ok();
        if let Err(err) = fs::create_dir_all(self.path.parent().unwrap_or(Path::new("."))) {
            tracing::error!("failed to create audit directory: {err}");
            return;
        }
        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(mut file) => {
                if let Err(err) = writeln!(file, "{line}") {
                    tracing::error!("failed to append audit event: {err}");
                }
            }
            Err(err) => tracing::error!("failed to open audit log: {err}"),
        }
    }
}
