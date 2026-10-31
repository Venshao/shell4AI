use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Security(String),
    #[error("{0}")]
    Connection(String),
    #[error("数据文件损坏: {0}")]
    Data(String),
    #[error("文件操作失败: {0}")]
    Io(#[from] io::Error),
    #[error("JSON 数据处理失败: {0}")]
    Json(#[from] serde_json::Error),
}

pub type AppResult<T> = Result<T, AppError>;
