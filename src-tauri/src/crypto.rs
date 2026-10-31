use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

use crate::error::{AppError, AppResult};

const SECRET_VERSION: &str = "v1";
const NONCE_BYTES: usize = 12;
const KEY_BYTES: usize = 32;

pub struct SecretCipher {
    cipher: Aes256Gcm,
}

impl SecretCipher {
    pub fn new(key: &[u8]) -> AppResult<Self> {
        let key: [u8; KEY_BYTES] = key
            .try_into()
            .map_err(|_| AppError::Security("AES-256-GCM 密钥必须为 32 字节".into()))?;
        Ok(Self {
            cipher: Aes256Gcm::new_from_slice(&key)
                .map_err(|_| AppError::Security("无法初始化 AES-256-GCM".into()))?,
        })
    }

    pub fn encrypt(&self, plaintext: &str, context: &str) -> AppResult<String> {
        let mut nonce_bytes = [0_u8; NONCE_BYTES];
        getrandom::fill(&mut nonce_bytes)
            .map_err(|err| AppError::Security(format!("无法生成加密随机数: {err}")))?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = self
            .cipher
            .encrypt(
                nonce,
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: context.as_bytes(),
                },
            )
            .map_err(|_| AppError::Security("无法加密保存的凭据".into()))?;
        let mut payload = Vec::with_capacity(NONCE_BYTES + ciphertext.len());
        payload.extend_from_slice(&nonce_bytes);
        payload.extend_from_slice(&ciphertext);
        Ok(format!("{SECRET_VERSION}:{}", URL_SAFE_NO_PAD.encode(payload)))
    }

    pub fn decrypt(&self, encoded: &str, context: &str) -> AppResult<String> {
        let (version, payload) = encoded
            .split_once(':')
            .ok_or_else(|| AppError::Security("无法解密保存的凭据".into()))?;
        if version != SECRET_VERSION {
            return Err(AppError::Security("不支持的凭据版本".into()));
        }
        let raw = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| AppError::Security("无法解密保存的凭据".into()))?;
        if raw.len() <= NONCE_BYTES {
            return Err(AppError::Security("无法解密保存的凭据".into()));
        }
        let (nonce, ciphertext) = raw.split_at(NONCE_BYTES);
        let plaintext = self
            .cipher
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: context.as_bytes(),
                },
            )
            .map_err(|_| AppError::Security("无法解密保存的凭据".into()))?;
        String::from_utf8(plaintext).map_err(|_| AppError::Security("无法解密保存的凭据".into()))
    }
}

pub fn random_key() -> AppResult<[u8; KEY_BYTES]> {
    let mut key = [0_u8; KEY_BYTES];
    getrandom::fill(&mut key)
        .map_err(|err| AppError::Security(format!("无法生成安全随机密钥: {err}")))?;
    Ok(key)
}

pub fn random_token() -> AppResult<String> {
    let mut bytes = [0_u8; KEY_BYTES];
    getrandom::fill(&mut bytes)
        .map_err(|err| AppError::Security(format!("无法生成 MCP Token: {err}")))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

#[derive(Clone)]
pub struct DpapiMasterKeyStore {
    path: std::path::PathBuf,
}

impl DpapiMasterKeyStore {
    pub fn new(path: std::path::PathBuf) -> Self {
        Self { path }
    }

    pub fn load_or_create(&self) -> AppResult<[u8; KEY_BYTES]> {
        if self.path.exists() {
            let protected = std::fs::read(&self.path)?;
            let key = dpapi_unprotect(&protected)?;
            return key
                .try_into()
                .map_err(|_| AppError::Security("DPAPI 主密钥长度无效".into()));
        }

        let key = random_key()?;
        let protected = dpapi_protect(&key)?;
        crate::storage::atomic_write_bytes(&self.path, &protected)?;
        Ok(key)
    }
}

#[cfg(windows)]
fn dpapi_protect(value: &[u8]) -> AppResult<Vec<u8>> {
    use std::ptr;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB};

    let mut input = value.to_vec();
    let input_blob = CRYPT_INTEGER_BLOB {
        cbData: input.len() as u32,
        pbData: input.as_mut_ptr(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let success = unsafe {
        CryptProtectData(
            &input_blob,
            ptr::null(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            0,
            &mut output,
        )
    };
    if success == 0 {
        return Err(AppError::Security(format!(
            "无法使用 Windows DPAPI 保护主密钥: {}",
            std::io::Error::last_os_error()
        )));
    }
    let result = unsafe {
        std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec()
    };
    unsafe {
        LocalFree(output.pbData.cast());
    }
    Ok(result)
}

#[cfg(windows)]
fn dpapi_unprotect(value: &[u8]) -> AppResult<Vec<u8>> {
    use std::ptr;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};

    let mut input = value.to_vec();
    let input_blob = CRYPT_INTEGER_BLOB {
        cbData: input.len() as u32,
        pbData: input.as_mut_ptr(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let success = unsafe {
        CryptUnprotectData(
            &input_blob,
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            0,
            &mut output,
        )
    };
    if success == 0 {
        return Err(AppError::Security(
            "当前 Windows 用户无法解密主密钥".into(),
        ));
    }
    let result = unsafe {
        std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec()
    };
    unsafe {
        LocalFree(output.pbData.cast());
    }
    Ok(result)
}

#[cfg(not(windows))]
fn dpapi_protect(_value: &[u8]) -> AppResult<Vec<u8>> {
    Err(AppError::Security("DPAPI 仅支持 Windows".into()))
}

#[cfg(not(windows))]
fn dpapi_unprotect(_value: &[u8]) -> AppResult<Vec<u8>> {
    Err(AppError::Security("DPAPI 仅支持 Windows".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aes_gcm_round_trip_binds_context() {
        let cipher = SecretCipher::new(&[7_u8; KEY_BYTES]).expect("cipher");
        let encoded = cipher.encrypt("secret", "host:a").expect("encrypt");
        assert!(!encoded.contains("secret"));
        assert_eq!(cipher.decrypt(&encoded, "host:a").expect("decrypt"), "secret");
        assert!(cipher.decrypt(&encoded, "host:b").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn dpapi_master_key_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = DpapiMasterKeyStore::new(dir.path().join("master.key.dpapi"));
        let first = store.load_or_create().expect("create key");
        let second = store.load_or_create().expect("load key");
        assert_eq!(first, second);
    }
}
