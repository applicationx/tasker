use crate::error::{AppError, ErrorCategory, Result};
use cap_std::ambient_authority;
use cap_std::fs::Dir;
use std::io::Read;
use std::path::{Component, Path};

pub const MAX_REFERENCED_BYTES: u64 = 1_048_576;

pub fn normalize(value: &str) -> Result<String> {
    if value.is_empty() || value.contains('\0') {
        return Err(unsafe_path(value, "path is empty or contains NUL"));
    }
    if value.starts_with(['/', '\\'])
        || value.starts_with("//")
        || value.starts_with("\\\\")
        || value.contains(':')
    {
        return Err(unsafe_path(
            value,
            "absolute, rooted, drive, and UNC paths are forbidden",
        ));
    }
    let portable = value.replace('\\', "/");
    if portable
        .split('/')
        .any(|part| part.is_empty() || part == "..")
    {
        return Err(unsafe_path(
            value,
            "empty and parent path components are forbidden",
        ));
    }
    let path = Path::new(&portable);
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(unsafe_path(value, "path must remain inside the project"));
    }
    Ok(portable)
}

pub fn read(project: &Path, value: &str) -> Result<String> {
    let portable = normalize(value)?;
    let dir = Dir::open_ambient_dir(project, ambient_authority())
        .map_err(|error| AppError::io(error, "cannot open project directory capability"))?;
    let metadata = dir.metadata(&portable).map_err(|error| {
        AppError::new(
            "referenced_source_unavailable",
            format!("Cannot inspect referenced source {portable}: {error}"),
            ErrorCategory::Validation,
        )
        .detail("source_path", portable.clone())
    })?;
    if !metadata.is_file() {
        return Err(AppError::new(
            "referenced_source_not_file",
            format!("Referenced source {portable} is not a regular file"),
            ErrorCategory::Validation,
        ));
    }
    let mut file = dir.open(&portable).map_err(|error| {
        AppError::new(
            "referenced_source_unavailable",
            format!("Cannot open referenced source {portable}: {error}"),
            ErrorCategory::Validation,
        )
        .detail("source_path", portable.clone())
    })?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_REFERENCED_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            AppError::io(error, format!("cannot read referenced source {portable}"))
        })?;
    if bytes.len() as u64 > MAX_REFERENCED_BYTES {
        return Err(AppError::new(
            "referenced_source_too_large",
            format!("Referenced source {portable} exceeds 1 MiB"),
            ErrorCategory::Validation,
        ));
    }
    String::from_utf8(bytes).map_err(|_| {
        AppError::new(
            "referenced_source_not_utf8",
            format!("Referenced source {portable} is not UTF-8"),
            ErrorCategory::Validation,
        )
    })
}

fn unsafe_path(value: &str, message: &str) -> AppError {
    AppError::new("unsafe_resource_path", message, ErrorCategory::Validation)
        .detail("source_path", value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_portable_paths_and_rejects_escape_syntax() {
        assert_eq!(normalize("docs\\api.md").unwrap(), "docs/api.md");
        for path in ["", "../x", "a/../x", "/x", "C:\\x", "//host/x", "a//b"] {
            assert!(normalize(path).is_err(), "{path}");
        }
    }
}
