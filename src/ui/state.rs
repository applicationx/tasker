use crate::error::{AppError, ErrorCategory, Result};
use crate::storage;
use directories::{BaseDirs, ProjectDirs};
use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

pub const ADDRESS: &str = "127.0.0.1";
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
pub struct RuntimePaths {
    pub lifecycle_lock: PathBuf,
    pub instance_lock: PathBuf,
    pub state: PathBuf,
    pub launch_error: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeState {
    pub schema_version: u32,
    pub pid: u32,
    pub address: String,
    pub port: u16,
    pub url: String,
    pub projects_root: String,
    pub root_fingerprint: String,
    pub started_at: String,
    pub server_version: String,
    pub instance_id: String,
    pub launch_id: String,
    pub executable: String,
    pub watch_mode: String,
    pub warnings: Vec<String>,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchError {
    pub launch_id: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleResult {
    pub schema_version: u32,
    pub operation: String,
    pub changed: bool,
    pub running: bool,
    pub pid: Option<u32>,
    pub address: Option<String>,
    pub port: Option<u16>,
    pub url: Option<String>,
    pub projects_root: String,
    pub started_at: Option<String>,
    pub server_version: Option<String>,
    pub instance_id: Option<String>,
    pub watch_mode: Option<String>,
    pub open_requested: bool,
    pub browser_opened: Option<bool>,
    pub warnings: Vec<String>,
}

impl LifecycleResult {
    pub fn running(
        operation: &str,
        changed: bool,
        state: &RuntimeState,
        open_requested: bool,
    ) -> Self {
        let mut warnings = state.warnings.clone();
        if state.server_version != SERVER_VERSION {
            warnings.push(format!(
                "The running UI child is version {}; this Tasker executable is version {SERVER_VERSION}",
                state.server_version
            ));
        }
        Self {
            schema_version: 1,
            operation: operation.to_string(),
            changed,
            running: true,
            pid: Some(state.pid),
            address: Some(state.address.clone()),
            port: Some(state.port),
            url: Some(state.url.clone()),
            projects_root: state.projects_root.clone(),
            started_at: Some(state.started_at.clone()),
            server_version: Some(state.server_version.clone()),
            instance_id: Some(state.instance_id.clone()),
            watch_mode: Some(state.watch_mode.clone()),
            open_requested,
            browser_opened: None,
            warnings,
        }
    }

    pub fn stopped(operation: &str, changed: bool, root: &Path) -> Self {
        Self {
            schema_version: 1,
            operation: operation.to_string(),
            changed,
            running: false,
            pid: None,
            address: None,
            port: None,
            url: None,
            projects_root: root
                .to_str()
                .expect("UI roots are validated as UTF-8")
                .to_string(),
            started_at: None,
            server_version: None,
            instance_id: None,
            watch_mode: None,
            open_requested: false,
            browser_opened: None,
            warnings: Vec::new(),
        }
    }
}

pub struct RuntimeLock {
    file: File,
}

impl Drop for RuntimeLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

impl RuntimePaths {
    pub fn prepare(root: &Path) -> Result<Self> {
        if root.to_str().is_none() {
            return Err(AppError::new(
                "ui_root_unavailable",
                "The canonical projects root cannot be represented as UTF-8 for display",
                ErrorCategory::Project,
            ));
        }
        let base = runtime_base()?.join("tasker").join("ui");
        create_secure_directory(&base)?;
        let directory = base.join(root_fingerprint(root));
        create_secure_directory(&directory)?;
        verify_secure_directory(&directory)?;
        let paths = Self {
            lifecycle_lock: directory.join("lifecycle.lock"),
            instance_lock: directory.join("instance.lock"),
            state: directory.join("state.json"),
            launch_error: directory.join("launch-error.json"),
        };
        paths.ensure_lock_files()?;
        Ok(paths)
    }

    fn ensure_lock_files(&self) -> Result<()> {
        for path in [&self.lifecycle_lock, &self.instance_lock] {
            open_lock(path)?;
        }
        Ok(())
    }

    pub fn lock_lifecycle(&self) -> Result<RuntimeLock> {
        let file = open_lock(&self.lifecycle_lock)?;
        file.lock_exclusive().map_err(|error| {
            AppError::new(
                "ui_runtime_insecure",
                format!("Cannot acquire UI lifecycle lock: {error}"),
                ErrorCategory::Io,
            )
        })?;
        Ok(RuntimeLock { file })
    }

    pub fn try_lock_instance(&self) -> Result<Option<RuntimeLock>> {
        let file = open_lock(&self.instance_lock)?;
        match file.try_lock_exclusive() {
            Ok(true) => Ok(Some(RuntimeLock { file })),
            Ok(false) => Ok(None),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(AppError::new(
                "ui_runtime_insecure",
                format!("Cannot inspect UI instance lock: {error}"),
                ErrorCategory::Io,
            )),
        }
    }

    pub fn read_state(&self) -> Result<Option<RuntimeState>> {
        let bytes = match fs::read(&self.state) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(AppError::new(
                    "ui_state_corrupt",
                    format!("Cannot read UI runtime state: {error}"),
                    ErrorCategory::Conflict,
                ));
            }
        };
        let state: RuntimeState = serde_json::from_slice(&bytes).map_err(|error| {
            AppError::new(
                "ui_state_corrupt",
                format!("UI runtime state is malformed: {error}"),
                ErrorCategory::Conflict,
            )
        })?;
        validate_state(&state)?;
        Ok(Some(state))
    }

    pub fn write_state(&self, state: &RuntimeState) -> Result<()> {
        let mut bytes = serde_json::to_vec_pretty(state)
            .map_err(|error| AppError::input(format!("cannot serialize UI state: {error}")))?;
        bytes.push(b'\n');
        storage::atomic_write(&self.state, &bytes)
    }

    pub fn read_launch_error(&self, launch_id: &str) -> Option<LaunchError> {
        let error: LaunchError =
            serde_json::from_slice(&fs::read(&self.launch_error).ok()?).ok()?;
        (error.launch_id == launch_id).then_some(error)
    }

    pub fn write_launch_error(&self, launch_id: &str, error: &AppError) {
        let report = LaunchError {
            launch_id: launch_id.to_string(),
            code: error.code.clone(),
            message: error.message.clone(),
        };
        if let Ok(mut bytes) = serde_json::to_vec_pretty(&report) {
            bytes.push(b'\n');
            let _ = storage::atomic_write(&self.launch_error, &bytes);
        }
    }

    pub fn clear_transient(&self) {
        remove_if_file(&self.state);
        remove_if_file(&self.launch_error);
    }

    pub fn remove_state_if_instance(&self, instance_id: &str) {
        if self
            .read_state()
            .ok()
            .flatten()
            .is_some_and(|state| state.instance_id == instance_id)
        {
            remove_if_file(&self.state);
        }
    }
}

pub fn root_fingerprint(root: &Path) -> String {
    let mut hasher = Sha256::new();
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        hasher.update(b"unix\0");
        hasher.update(root.as_os_str().as_bytes());
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        hasher.update(b"windows-utf16\0");
        for unit in root.as_os_str().encode_wide() {
            hasher.update(unit.to_le_bytes());
        }
    }
    format!("{:x}", hasher.finalize())
}

fn runtime_base() -> Result<PathBuf> {
    if let Some(base) = std::env::var_os("TASKER_UI_RUNTIME") {
        return Ok(PathBuf::from(base));
    }
    if let Some(base) = BaseDirs::new().and_then(|dirs| dirs.runtime_dir().map(Path::to_path_buf)) {
        return Ok(base);
    }
    ProjectDirs::from("dev", "Tasker", "tasker")
        .map(|dirs| dirs.data_local_dir().join("runtime"))
        .ok_or_else(|| {
            AppError::new(
                "ui_runtime_insecure",
                "Cannot determine a per-user UI runtime directory",
                ErrorCategory::Io,
            )
        })
}

fn create_secure_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|error| {
        AppError::new(
            "ui_runtime_insecure",
            format!(
                "Cannot create UI runtime directory {}: {error}",
                path.display()
            ),
            ErrorCategory::Io,
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
            AppError::new(
                "ui_runtime_insecure",
                format!(
                    "Cannot secure UI runtime directory {}: {error}",
                    path.display()
                ),
                ErrorCategory::Io,
            )
        })?;
    }
    verify_secure_directory(path)
}

fn verify_secure_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        AppError::new(
            "ui_runtime_insecure",
            format!(
                "Cannot inspect UI runtime directory {}: {error}",
                path.display()
            ),
            ErrorCategory::Io,
        )
    })?;
    if !metadata.file_type().is_dir() || storage::is_symlink_or_reparse(&metadata) {
        return Err(AppError::new(
            "ui_runtime_insecure",
            "UI runtime paths must be real, non-reparse directories",
            ErrorCategory::Io,
        )
        .detail("path", path.to_string_lossy().to_string()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(AppError::new(
                "ui_runtime_insecure",
                "UI runtime directory is accessible by other users",
                ErrorCategory::Io,
            )
            .detail("path", path.to_string_lossy().to_string()));
        }
    }
    Ok(())
}

fn open_lock(path: &Path) -> Result<File> {
    let metadata = fs::symlink_metadata(path).ok();
    if metadata
        .as_ref()
        .is_some_and(|metadata| storage::is_symlink_or_reparse(metadata) || !metadata.is_file())
    {
        return Err(AppError::new(
            "ui_runtime_insecure",
            "UI lock paths must be regular non-reparse files",
            ErrorCategory::Io,
        )
        .detail("path", path.to_string_lossy().to_string()));
    }
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|error| {
        AppError::new(
            "ui_runtime_insecure",
            format!("Cannot open UI lock {}: {error}", path.display()),
            ErrorCategory::Io,
        )
    })
}

fn remove_if_file(path: &Path) {
    if fs::symlink_metadata(path).ok().is_some_and(|metadata| {
        metadata.file_type().is_file() && !storage::is_symlink_or_reparse(&metadata)
    }) {
        let _ = fs::remove_file(path);
    }
}

fn validate_state(state: &RuntimeState) -> Result<()> {
    let valid_token = state.token.len() == 64
        && state
            .token
            .chars()
            .all(|character| character.is_ascii_hexdigit());
    if state.schema_version != 1
        || state.address != ADDRESS
        || state.port == 0
        || state.url != format!("http://{}:{}/", ADDRESS, state.port)
        || state.instance_id.is_empty()
        || state.launch_id.is_empty()
        || state.root_fingerprint.is_empty()
        || !valid_token
    {
        return Err(AppError::new(
            "ui_state_corrupt",
            "UI runtime state failed structural validation",
            ErrorCategory::Conflict,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::root_fingerprint;
    use std::path::Path;

    #[test]
    fn root_fingerprint_preserves_native_path_identity() {
        assert_ne!(
            root_fingerprint(Path::new("CaseSensitiveRoot")),
            root_fingerprint(Path::new("casesensitiveroot"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn root_fingerprint_uses_non_utf8_bytes_without_loss() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        assert_ne!(
            root_fingerprint(Path::new(OsStr::from_bytes(b"root-\xff"))),
            root_fingerprint(Path::new(OsStr::from_bytes(b"root-\xfe")))
        );
    }
}
