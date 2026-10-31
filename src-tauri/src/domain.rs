use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

fn clean_required(value: &str, field: &str) -> AppResult<String> {
    let cleaned = value.trim();
    if cleaned.is_empty() {
        return Err(AppError::Validation(format!("{field} 不能为空")));
    }
    if cleaned.chars().any(char::is_whitespace) {
        return Err(AppError::Validation(format!("{field} 不能包含空白字符")));
    }
    Ok(cleaned.to_owned())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostPayload {
    pub host_id: Option<String>,
    pub alias: String,
    pub hostname: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct HostInput {
    pub alias: String,
    pub hostname: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    pub enabled: bool,
}

impl HostPayload {
    pub fn validate(self) -> AppResult<HostInput> {
        let alias = clean_required(&self.alias, "别名")?;
        if alias.chars().count() > 64 {
            return Err(AppError::Validation("别名不能超过 64 个字符".into()));
        }
        let hostname = clean_required(&self.hostname, "主机名")?;
        if hostname.chars().count() > 253 {
            return Err(AppError::Validation("主机名不能超过 253 个字符".into()));
        }
        let username = clean_required(&self.username, "用户名")?;
        if username.chars().count() > 128 {
            return Err(AppError::Validation("用户名不能超过 128 个字符".into()));
        }
        if self.port == 0 {
            return Err(AppError::Validation("端口必须在 1 到 65535 之间".into()));
        }
        if let Some(password) = self.password.as_ref() {
            if password.is_empty() {
                return Err(AppError::Validation("密码不能为空".into()));
            }
            if password.len() > 4096 {
                return Err(AppError::Validation("密码不能超过 4096 字节".into()));
            }
        }
        Ok(HostInput {
            alias,
            hostname,
            port: self.port,
            username,
            password: self.password,
            enabled: self.enabled,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostRecord {
    pub id: String,
    pub alias: String,
    pub hostname: String,
    pub port: u16,
    pub username: String,
    pub enabled: bool,
    pub password_configured: bool,
    pub host_key_fingerprint: Option<String>,
    pub host_key_line: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl HostRecord {
    pub fn new(input: &HostInput) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            alias: input.alias.clone(),
            hostname: input.hostname.clone(),
            port: input.port,
            username: input.username.clone(),
            enabled: input.enabled,
            password_configured: input.password.is_some(),
            host_key_fingerprint: None,
            host_key_line: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn view(&self) -> HostView {
        HostView {
            id: self.id.clone(),
            alias: self.alias.clone(),
            hostname: self.hostname.clone(),
            port: self.port,
            username: self.username.clone(),
            enabled: self.enabled,
            password_configured: self.password_configured,
            host_key_fingerprint: self.host_key_fingerprint.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostView {
    pub id: String,
    pub alias: String,
    pub hostname: String,
    pub port: u16,
    pub username: String,
    pub enabled: bool,
    pub password_configured: bool,
    pub host_key_fingerprint: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostEntry {
    pub record: HostRecord,
    pub secret: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostDocument {
    pub schema_version: u32,
    pub hosts: Vec<HostEntry>,
}

impl Default for HostDocument {
    fn default() -> Self {
        Self {
            schema_version: 1,
            hosts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProbeResult {
    pub host_id: String,
    pub alias: String,
    pub fingerprint: String,
    pub host_key_line: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> HostPayload {
        HostPayload {
            host_id: None,
            alias: "  prod  ".into(),
            hostname: " 10.0.0.10 ".into(),
            port: 22,
            username: " root ".into(),
            password: Some("secret".into()),
            enabled: true,
        }
    }

    #[test]
    fn host_payload_normalizes_text() {
        let value = payload().validate().expect("valid host");
        assert_eq!(value.alias, "prod");
        assert_eq!(value.hostname, "10.0.0.10");
        assert_eq!(value.username, "root");
    }

    #[test]
    fn host_payload_rejects_whitespace_in_identity_fields() {
        let mut value = payload();
        value.alias = "prod web".into();
        assert!(value.validate().is_err());
    }

    #[test]
    fn host_payload_rejects_zero_port() {
        let mut value = payload();
        value.port = 0;
        assert!(value.validate().is_err());
    }
}
