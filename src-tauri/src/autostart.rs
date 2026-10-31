use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "SSH MCP";

#[derive(Debug, Clone)]
pub struct AutostartManager {
    executable: PathBuf,
}

impl AutostartManager {
    pub fn new(executable: PathBuf) -> Self {
        Self { executable }
    }

    pub fn current() -> Self {
        Self::new(std::env::current_exe().unwrap_or_else(|_| PathBuf::from("ssh-mcp.exe")))
    }

    pub fn command(&self) -> String {
        build_autostart_command(&self.executable)
    }

    #[cfg(windows)]
    pub fn is_enabled(&self) -> bool {
        use winreg::{enums::HKEY_CURRENT_USER, RegKey};

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let Ok(key) = hkcu.open_subkey(RUN_KEY) else {
            return false;
        };
        let value: Result<String, _> = key.get_value(VALUE_NAME);
        value.is_ok_and(|command| command == self.command())
    }

    #[cfg(not(windows))]
    pub fn is_enabled(&self) -> bool {
        false
    }

    #[cfg(windows)]
    pub fn set_enabled(&self, enabled: bool) -> AppResult<bool> {
        use winreg::{enums::HKEY_CURRENT_USER, RegKey};

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu
            .create_subkey(RUN_KEY)
            .map_err(|err| AppError::Security(format!("无法打开登录启动项: {err}")))?;
        if enabled {
            key.set_value(VALUE_NAME, &self.command())
                .map_err(|err| AppError::Security(format!("无法写入登录启动项: {err}")))?;
        } else {
            match key.delete_value(VALUE_NAME) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(AppError::Security(format!(
                        "无法删除登录启动项: {err}"
                    )))
                }
            }
        }
        Ok(self.is_enabled())
    }

    #[cfg(not(windows))]
    pub fn set_enabled(&self, _enabled: bool) -> AppResult<bool> {
        Err(AppError::Security("登录自启动仅支持 Windows".into()))
    }
}

pub fn build_autostart_command(executable: &Path) -> String {
    format!(r#""{}" --autostart"#, executable.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_uses_quoted_current_executable() {
        let command = build_autostart_command(Path::new(r"C:\Apps\SSH MCP\ssh-mcp.exe"));
        assert_eq!(command, r#""C:\Apps\SSH MCP\ssh-mcp.exe" --autostart"#);
    }
}
