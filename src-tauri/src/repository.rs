use std::{
    path::{Path, PathBuf},
    sync::RwLock,
};

use chrono::Utc;
use zeroize::Zeroizing;

use crate::{
    crypto::SecretCipher,
    domain::{HostDocument, HostEntry, HostInput, HostRecord, HostView},
    error::{AppError, AppResult},
    storage::{atomic_write_json, read_json},
};

pub struct HostRepository {
    path: PathBuf,
    cipher: SecretCipher,
    document: RwLock<HostDocument>,
}

impl HostRepository {
    pub fn open(data_dir: &Path, cipher: SecretCipher) -> AppResult<Self> {
        let path = data_dir.join("store.json");
        let document = read_json::<HostDocument>(&path)?.unwrap_or_default();
        Ok(Self {
            path,
            cipher,
            document: RwLock::new(document),
        })
    }

    pub fn list_hosts(&self) -> AppResult<Vec<HostRecord>> {
        let document = self
            .document
            .read()
            .map_err(|_| AppError::Data("主机存储锁已损坏".into()))?;
        Ok(document
            .hosts
            .iter()
            .map(|entry| entry.record.clone())
            .collect())
    }

    pub fn list_views(&self) -> AppResult<Vec<HostView>> {
        Ok(self
            .list_hosts()?
            .into_iter()
            .map(|record| record.view())
            .collect())
    }

    pub fn get(&self, selector: &str) -> AppResult<HostRecord> {
        let document = self
            .document
            .read()
            .map_err(|_| AppError::Data("主机存储锁已损坏".into()))?;
        let index = find_by_selector(&document, selector)?;
        Ok(document.hosts[index].record.clone())
    }

    pub fn get_by_id(&self, host_id: &str) -> AppResult<HostRecord> {
        let document = self
            .document
            .read()
            .map_err(|_| AppError::Data("主机存储锁已损坏".into()))?;
        let index = find_by_id(&document, host_id)?;
        Ok(document.hosts[index].record.clone())
    }

    pub fn create(&self, input: &HostInput) -> AppResult<HostRecord> {
        let password = input
            .password
            .as_ref()
            .ok_or_else(|| AppError::Validation("新建主机必须提供密码".into()))?;
        let mut document = self
            .document
            .write()
            .map_err(|_| AppError::Data("主机存储锁已损坏".into()))?;
        ensure_unique_alias(&document, &input.alias, None)?;
        let mut record = HostRecord::new(input);
        record.password_configured = true;
        let secret = self
            .cipher
            .encrypt(password, &password_context(&record.id))?;
        document.hosts.push(HostEntry {
            record: record.clone(),
            secret: Some(secret),
        });
        self.save(&document)?;
        Ok(record)
    }

    pub fn update(&self, host_id: &str, input: &HostInput) -> AppResult<HostRecord> {
        let mut document = self
            .document
            .write()
            .map_err(|_| AppError::Data("主机存储锁已损坏".into()))?;
        let index = find_by_id(&document, host_id)?;
        ensure_unique_alias(&document, &input.alias, Some(host_id))?;
        let entry = &mut document.hosts[index];
        let identity_changed =
            entry.record.hostname != input.hostname || entry.record.port != input.port;
        entry.record.alias = input.alias.clone();
        entry.record.hostname = input.hostname.clone();
        entry.record.port = input.port;
        entry.record.username = input.username.clone();
        entry.record.enabled = input.enabled;
        entry.record.updated_at = Utc::now();
        if identity_changed {
            entry.record.host_key_fingerprint = None;
            entry.record.host_key_line = None;
        }
        if let Some(password) = input.password.as_ref() {
            entry.secret = Some(
                self.cipher
                    .encrypt(password, &password_context(host_id))?,
            );
            entry.record.password_configured = true;
        }
        let record = entry.record.clone();
        self.save(&document)?;
        Ok(record)
    }

    pub fn delete(&self, host_id: &str) -> AppResult<()> {
        let mut document = self
            .document
            .write()
            .map_err(|_| AppError::Data("主机存储锁已损坏".into()))?;
        let index = find_by_id(&document, host_id)?;
        document.hosts.remove(index);
        self.save(&document)
    }

    pub fn password_for(&self, host_id: &str) -> AppResult<Zeroizing<String>> {
        let document = self
            .document
            .read()
            .map_err(|_| AppError::Data("主机存储锁已损坏".into()))?;
        let index = find_by_id(&document, host_id)?;
        let entry = &document.hosts[index];
        let secret = entry
            .secret
            .as_deref()
            .ok_or_else(|| AppError::Validation(format!("主机 {} 未配置密码", entry.record.alias)))?;
        Ok(Zeroizing::new(
            self.cipher.decrypt(secret, &password_context(host_id))?,
        ))
    }

    pub fn set_host_key(
        &self,
        host_id: &str,
        fingerprint: &str,
        host_key_line: &str,
    ) -> AppResult<HostRecord> {
        let mut document = self
            .document
            .write()
            .map_err(|_| AppError::Data("主机存储锁已损坏".into()))?;
        let index = find_by_id(&document, host_id)?;
        let entry = &mut document.hosts[index];
        entry.record.host_key_fingerprint = Some(fingerprint.to_owned());
        entry.record.host_key_line = Some(host_key_line.to_owned());
        entry.record.updated_at = Utc::now();
        let record = entry.record.clone();
        self.save(&document)?;
        Ok(record)
    }

    fn save(&self, document: &HostDocument) -> AppResult<()> {
        atomic_write_json(&self.path, document)
    }
}

fn password_context(host_id: &str) -> String {
    format!("host:{host_id}:password")
}

fn find_by_id(document: &HostDocument, host_id: &str) -> AppResult<usize> {
    document
        .hosts
        .iter()
        .position(|entry| entry.record.id == host_id)
        .ok_or_else(|| AppError::NotFound(format!("未知主机 ID: {host_id}")))
}

fn find_by_selector(document: &HostDocument, selector: &str) -> AppResult<usize> {
    let cleaned = selector.trim();
    if cleaned.is_empty() {
        return Err(AppError::NotFound("主机标识不能为空".into()));
    }
    let normalized = cleaned.to_lowercase();
    let alias_matches: Vec<usize> = document
        .hosts
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.record.alias.to_lowercase() == normalized)
        .map(|(index, _)| index)
        .collect();
    if alias_matches.len() == 1 {
        return Ok(alias_matches[0]);
    }
    let host_matches: Vec<usize> = document
        .hosts
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.record.hostname.to_lowercase() == normalized)
        .map(|(index, _)| index)
        .collect();
    match host_matches.as_slice() {
        [index] => Ok(*index),
        [] => Err(AppError::NotFound(format!("未登记的主机: {cleaned}"))),
        _ => {
            let aliases = host_matches
                .iter()
                .map(|index| document.hosts[*index].record.alias.as_str())
                .collect::<Vec<_>>()
                .join("、");
            Err(AppError::NotFound(format!(
                "多个主机匹配 {cleaned}：{aliases}，请使用别名"
            )))
        }
    }
}

fn ensure_unique_alias(
    document: &HostDocument,
    alias: &str,
    exclude_id: Option<&str>,
) -> AppResult<()> {
    let normalized = alias.to_lowercase();
    if document.hosts.iter().any(|entry| {
        Some(entry.record.id.as_str()) != exclude_id
            && entry.record.alias.to_lowercase() == normalized
    }) {
        return Err(AppError::Conflict(format!("主机别名已存在: {alias}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> (tempfile::TempDir, HostRepository) {
        let dir = tempfile::tempdir().expect("tempdir");
        let cipher = SecretCipher::new(&[9_u8; 32]).expect("cipher");
        let repo = HostRepository::open(dir.path(), cipher).expect("repository");
        (dir, repo)
    }

    fn input(alias: &str, hostname: &str, password: &str) -> HostInput {
        HostInput {
            alias: alias.into(),
            hostname: hostname.into(),
            port: 22,
            username: "root".into(),
            password: Some(password.into()),
            enabled: true,
        }
    }

    #[test]
    fn persists_encrypted_password() {
        let (dir, repo) = repo();
        let record = repo.create(&input("prod", "10.0.0.10", "top-secret-value")).expect("create");

        let text = std::fs::read_to_string(dir.path().join("store.json")).expect("store");
        assert!(!text.contains("top-secret-value"));
        assert_eq!(repo.password_for(&record.id).expect("password").as_str(), "top-secret-value");

        let cipher = SecretCipher::new(&[9_u8; 32]).expect("cipher");
        let reloaded = HostRepository::open(dir.path(), cipher).expect("reload");
        assert_eq!(reloaded.get("PROD").expect("host").hostname, "10.0.0.10");
    }

    #[test]
    fn update_without_password_keeps_secret() {
        let (_, repo) = repo();
        let record = repo.create(&input("prod", "10.0.0.10", "old")).expect("create");
        let mut update = input("renamed", "10.0.0.11", "ignored");
        update.password = None;
        repo.update(&record.id, &update).expect("update");

        assert_eq!(repo.password_for(&record.id).expect("password").as_str(), "old");
    }

    #[test]
    fn changing_address_revokes_old_host_key() {
        let (_, repo) = repo();
        let record = repo.create(&input("prod", "10.0.0.10", "top-secret-value")).expect("create");
        repo.set_host_key(&record.id, "SHA256:old", "ssh-ed25519 AAAA")
            .expect("trust");

        let mut update = input("prod", "10.0.0.11", "ignored");
        update.password = None;
        let updated = repo.update(&record.id, &update).expect("update");

        assert!(updated.host_key_fingerprint.is_none());
        assert!(updated.host_key_line.is_none());
    }

    #[test]
    fn duplicate_alias_is_case_insensitive() {
        let (_, repo) = repo();
        repo.create(&input("prod", "10.0.0.10", "a")).expect("create");
        assert!(repo.create(&input("PROD", "10.0.0.11", "b")).is_err());
    }

    #[test]
    fn ambiguous_hostname_is_rejected() {
        let (_, repo) = repo();
        let first = repo.create(&input("prod-a", "10.0.0.10", "a")).expect("create");
        repo.create(&input("prod-b", "10.0.0.10", "b")).expect("create");
        assert_eq!(repo.get("PROD-A").expect("alias").id, first.id);
        assert!(repo.get("10.0.0.10").is_err());
    }
}
