use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

use crate::error::{AppError, AppResult};

pub fn default_data_dir() -> PathBuf {
    if let Some(value) = env::var_os("SSH_MCP_DATA_DIR") {
        return PathBuf::from(value);
    }
    let appdata = env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join("AppData")
                .join("Roaming")
        });
    appdata.join("SSH-MCP")
}

pub fn prepare_data_directory(path: &Path) -> AppResult<PathBuf> {
    std::fs::create_dir_all(path)?;
    let path = dunce::canonicalize(path)?;
    let sid = current_user_sid()?;
    run_icacls(&path, &sid)?;
    Ok(path)
}

fn current_user_sid() -> AppResult<String> {
    let output = hidden_command("whoami")
        .args(["/user", "/fo", "csv", "/nh"])
        .output()?;
    if !output.status.success() {
        return Err(AppError::Security(format!(
            "无法读取当前 Windows 用户 SID: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().next().unwrap_or_default();
    let sid = line
        .split(',')
        .nth(1)
        .map(|value| value.trim().trim_matches('"'))
        .unwrap_or_default();
    if !sid.starts_with("S-1-")
        || !sid
            .chars()
            .all(|value| value.is_ascii_digit() || value == '-' || value == 'S')
    {
        return Err(AppError::Security("当前 Windows 用户 SID 格式无效".into()));
    }
    Ok(sid.to_owned())
}

fn run_icacls(path: &Path, sid: &str) -> AppResult<()> {
    let output = hidden_command("icacls")
        .arg(path)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg(format!("*{sid}:(OI)(CI)F"))
        .arg("*S-1-5-18:(OI)(CI)F")
        .arg("*S-1-5-32-544:(OI)(CI)F")
        .output()?;
    if !output.status.success() {
        return Err(AppError::Security(format!(
            "无法限制数据目录权限: {} {}",
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

#[cfg(windows)]
fn hidden_command(program: &str) -> Command {
    use std::os::windows::process::CommandExt;
    let mut command = Command::new(program);
    command.creation_flags(0x0800_0000);
    command
}

#[cfg(not(windows))]
fn hidden_command(program: &str) -> Command {
    Command::new(program)
}

#[cfg(windows)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardens_data_directory_for_current_user() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("private");
        let secured = prepare_data_directory(&target).expect("secure directory");
        assert!(secured.is_dir());

        let dump = dir.path().join("acl.sddl");
        let output = hidden_command("icacls")
            .arg(&secured)
            .arg("/save")
            .arg(&dump)
            .output()
            .expect("save acl");
        assert!(output.status.success());
        let sddl = String::from_utf16_lossy(
            &std::fs::read(&dump)
                .expect("read acl")
                .chunks_exact(2)
                .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
                .collect::<Vec<_>>(),
        );
        assert!(sddl.contains("SY"));
        assert!(sddl.contains("BA"));
    }
}
