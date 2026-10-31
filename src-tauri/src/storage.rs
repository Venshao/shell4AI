use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use serde::{de::DeserializeOwned, Serialize};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

pub fn atomic_write_bytes(path: &Path, content: &[u8]) -> AppResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, "路径没有父目录")))?;
    fs::create_dir_all(parent)?;

    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("data");
    let temp = parent.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    let result = (|| -> AppResult<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(content)?;
        file.flush()?;
        file.sync_all()?;

        if path.exists() {
            let backup = backup_path(path);
            fs::copy(path, backup)?;
        }
        replace_file(&temp, path)
    })();

    if temp.exists() {
        let _ = fs::remove_file(temp);
    }
    result
}

pub fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> AppResult<()> {
    let content = serde_json::to_vec_pretty(value)?;
    atomic_write_bytes(path, &content)
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> AppResult<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read(path)?;
    let value = serde_json::from_slice(&content)
        .map_err(|err| AppError::Data(format!("{}: {err}", path.display())))?;
    Ok(Some(value))
}

fn backup_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("data");
    path.with_file_name(format!("{name}.bak"))
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> AppResult<()> {
    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    let source = wide(source);
    let destination = wide(destination);
    let success = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if success == 0 {
        Err(AppError::Io(std::io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> AppResult<()> {
    fs::rename(source, destination)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_json_write_replaces_existing_file_and_keeps_backup() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("store.json");
        atomic_write_json(&path, &serde_json::json!({"value": 1})).expect("first write");
        atomic_write_json(&path, &serde_json::json!({"value": 2})).expect("second write");

        let current: serde_json::Value = read_json(&path).expect("read current").expect("exists");
        let backup: serde_json::Value =
            read_json(&backup_path(&path)).expect("read backup").expect("exists");
        assert_eq!(current["value"], 2);
        assert_eq!(backup["value"], 1);
    }
}
