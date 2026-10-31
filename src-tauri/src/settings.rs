use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    crypto::{random_token, SecretCipher},
    error::{AppError, AppResult},
    storage::{atomic_write_json, read_json},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeSettings {
    pub mcp_host: String,
    pub mcp_port: u16,
    pub mcp_token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsDocument {
    schema_version: u32,
    mcp_host: String,
    mcp_port: u16,
    encrypted_mcp_token: String,
}

pub struct AppSettingsStore {
    path: PathBuf,
    cipher: SecretCipher,
}

impl AppSettingsStore {
    pub fn new(data_dir: &Path, cipher: SecretCipher) -> Self {
        Self {
            path: data_dir.join("settings.json"),
            cipher,
        }
    }

    pub fn load_or_create(&self) -> AppResult<RuntimeSettings> {
        if let Some(document) = read_json::<SettingsDocument>(&self.path)? {
            validate_listener(&document.mcp_host, document.mcp_port)?;
            return Ok(RuntimeSettings {
                mcp_host: document.mcp_host,
                mcp_port: document.mcp_port,
                mcp_token: self
                    .cipher
                    .decrypt(&document.encrypted_mcp_token, "settings:mcp_token")?,
            });
        }

        let settings = RuntimeSettings {
            mcp_host: "127.0.0.1".into(),
            mcp_port: 8765,
            mcp_token: random_token()?,
        };
        self.save(&settings)?;
        Ok(settings)
    }

    fn save(&self, settings: &RuntimeSettings) -> AppResult<()> {
        validate_listener(&settings.mcp_host, settings.mcp_port)?;
        let document = SettingsDocument {
            schema_version: 1,
            mcp_host: settings.mcp_host.clone(),
            mcp_port: settings.mcp_port,
            encrypted_mcp_token: self
                .cipher
                .encrypt(&settings.mcp_token, "settings:mcp_token")?,
        };
        atomic_write_json(&self.path, &document)
    }
}

fn validate_listener(host: &str, port: u16) -> AppResult<()> {
    if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return Err(AppError::Security(
            "MCP 服务只允许监听本机回环地址".into(),
        ));
    }
    if port < 1024 {
        return Err(AppError::Security("MCP 端口必须大于等于 1024".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_encrypted_token_and_reloads_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cipher = SecretCipher::new(&[3_u8; 32]).expect("cipher");
        let store = AppSettingsStore::new(dir.path(), cipher);
        let first = store.load_or_create().expect("create");
        let second = store.load_or_create().expect("reload");

        assert_eq!(first.mcp_token, second.mcp_token);
        let text = std::fs::read_to_string(dir.path().join("settings.json")).expect("settings");
        assert!(!text.contains(&first.mcp_token));
    }
}
