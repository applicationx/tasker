use serde_json::{Map, Value, json};
use std::io;
use thiserror::Error;

#[derive(Debug, Error)]
#[error("{message}")]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub category: ErrorCategory,
    pub details: Box<Map<String, Value>>,
}

#[derive(Debug, Clone, Copy)]
pub enum ErrorCategory {
    Input,
    NotFound,
    Conflict,
    Validation,
    Cycle,
    Project,
    Io,
}

impl AppError {
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        category: ErrorCategory,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            category,
            details: Box::default(),
        }
    }
    pub fn detail(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.details.insert(key.into(), value.into());
        self
    }
    pub fn input(message: impl Into<String>) -> Self {
        Self::new("invalid_input", message, ErrorCategory::Input)
    }
    pub fn project(message: impl Into<String>) -> Self {
        Self::new("invalid_project_config", message, ErrorCategory::Project)
    }
    pub fn io(error: io::Error, context: impl AsRef<str>) -> Self {
        Self::new(
            "filesystem_error",
            format!("{}: {error}", context.as_ref()),
            ErrorCategory::Io,
        )
    }
    pub fn exit_code(&self) -> u8 {
        match self.category {
            ErrorCategory::Input => 2,
            ErrorCategory::NotFound => 3,
            ErrorCategory::Conflict => 4,
            ErrorCategory::Validation => 5,
            ErrorCategory::Cycle => 6,
            ErrorCategory::Project => 7,
            ErrorCategory::Io => 8,
        }
    }
    pub fn value(&self) -> Value {
        let mut inner = Map::new();
        inner.insert("code".into(), json!(self.code));
        inner.insert("message".into(), json!(self.message));
        inner.extend((*self.details).clone());
        json!({"error": inner})
    }
}
pub type Result<T> = std::result::Result<T, AppError>;
