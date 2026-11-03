//! Non-authoritative list of known projects roots ("boards").
//!
//! The registry only remembers where boards live so `tasker list` can show
//! all of them. Every entry is re-read from the filesystem on use; a missing
//! or unreadable entry is reported as a warning and never fails a command.

use crate::error::{AppError, Result};
use crate::storage;
use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    #[serde(default)]
    pub roots: Vec<RegistryRoot>,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryRoot {
    pub path: String,
}

pub fn registry_path() -> PathBuf {
    storage::config_dir()
        .map(|d| d.join("registry.toml"))
        .unwrap_or_else(|| PathBuf::from(".tasker-registry.toml"))
}

/// Canonical, display-friendly form of an existing path (no `\\?\` prefix).
pub fn display_path(path: &Path) -> String {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = canonical.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        text.into_owned()
    }
}

pub fn load() -> Result<Registry> {
    let path = registry_path();
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Registry::default()),
        Err(e) => return Err(AppError::io(e, format!("cannot read {}", path.display()))),
    };
    toml::from_str(&text).map_err(|e| {
        AppError::input(format!("malformed registry {}: {e}", path.display()))
            .detail("registry_file", path.to_string_lossy().to_string())
    })
}

/// Loads, edits, and atomically saves the registry under `registry.lock`.
/// Returns the edit's result; the file is untouched when nothing changed.
fn update<T>(edit: impl FnOnce(&mut Registry) -> T) -> Result<T> {
    let path = registry_path();
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    fs::create_dir_all(&dir)
        .map_err(|e| AppError::io(e, "cannot create configuration directory"))?;
    let lock_path = dir.join("registry.lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|e| AppError::io(e, format!("cannot open {}", lock_path.display())))?;
    lock.lock_exclusive()
        .map_err(|e| AppError::io(e, "cannot acquire registry lock"))?;
    let mut registry = load()?;
    let before = registry.roots.clone();
    let result = edit(&mut registry);
    registry.roots.sort();
    registry.roots.dedup();
    if registry.roots != before {
        let text = toml::to_string_pretty(&registry)
            .map_err(|e| AppError::input(format!("cannot serialize registry: {e}")))?;
        storage::atomic_write(&path, text.as_bytes())?;
    }
    let _ = lock.unlock();
    Ok(result)
}

/// Registers an existing directory. Returns its display path and whether it
/// was newly added.
pub fn add(path: &Path) -> Result<(String, bool)> {
    if !path.is_dir() {
        return Err(AppError::input(format!(
            "Board root {} is not an existing directory",
            path.display()
        ))
        .detail("path", path.to_string_lossy().to_string()));
    }
    let display = display_path(path);
    let added = update(|registry| {
        if registry.roots.iter().any(|root| root.path == display) {
            false
        } else {
            registry.roots.push(RegistryRoot {
                path: display.clone(),
            });
            true
        }
    })?;
    Ok((display, added))
}

/// Removes a registered root, matching either its canonical form or the
/// literal stored text so entries whose directory is gone can be pruned.
pub fn remove(path: &Path) -> Result<(String, bool)> {
    let display = display_path(path);
    let literal = path.to_string_lossy().to_string();
    let removed = update(|registry| {
        let before = registry.roots.len();
        registry
            .roots
            .retain(|root| root.path != display && root.path != literal);
        registry.roots.len() != before
    })?;
    Ok((display, removed))
}
