use super::state::{
    ADDRESS, LifecycleResult, RuntimePaths, RuntimeState, SERVER_VERSION, root_fingerprint,
};
use crate::error::{AppError, ErrorCategory, Result};
use crate::service::snapshot::canonical_projects_root;
use serde::Deserialize;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

const LIFECYCLE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Deserialize)]
struct ApiEnvelope<T> {
    api_version: u32,
    ok: bool,
    data: T,
}

#[derive(Debug, Deserialize)]
struct Health {
    instance_id: String,
    launch_id: String,
    root_fingerprint: String,
    server_version: String,
}

pub fn start(open: bool) -> Result<LifecycleResult> {
    let root = canonical_projects_root(&crate::storage::projects_root())?;
    let paths = RuntimePaths::prepare(&root)?;
    let _lifecycle = paths.lock_lifecycle()?;
    start_locked(&root, &paths, "start", open)
}

pub fn status() -> Result<LifecycleResult> {
    let root = canonical_projects_root(&crate::storage::projects_root())?;
    let paths = RuntimePaths::prepare(&root)?;
    let _lifecycle = paths.lock_lifecycle()?;
    match inspect(&root, &paths)? {
        Some(state) => Ok(LifecycleResult::running("status", false, &state, false)),
        None => Ok(LifecycleResult::stopped("status", false, &root)),
    }
}

pub fn stop() -> Result<LifecycleResult> {
    let root = canonical_projects_root(&crate::storage::projects_root())?;
    let paths = RuntimePaths::prepare(&root)?;
    let _lifecycle = paths.lock_lifecycle()?;
    let changed = stop_locked(&root, &paths)?;
    Ok(LifecycleResult::stopped("stop", changed, &root))
}

pub fn restart(open: bool) -> Result<LifecycleResult> {
    let root = canonical_projects_root(&crate::storage::projects_root())?;
    let paths = RuntimePaths::prepare(&root)?;
    let _lifecycle = paths.lock_lifecycle()?;
    let _ = stop_locked(&root, &paths)?;
    start_locked(&root, &paths, "restart", open)
}

fn start_locked(
    root: &Path,
    paths: &RuntimePaths,
    operation: &str,
    open: bool,
) -> Result<LifecycleResult> {
    if let Some(state) = inspect(root, paths)? {
        if state.server_version == SERVER_VERSION {
            let mut result = LifecycleResult::running(operation, false, &state, open);
            maybe_open(&mut result);
            return Ok(result);
        }
        // A nonce-authenticated older child is safe to stop through its own
        // endpoint. Never fall back to killing the recorded PID.
        let _ = stop_locked(root, paths)?;
    }
    paths.clear_transient();
    let launch_id = Uuid::new_v4().to_string();
    let mut child = spawn_child(root, &launch_id).map_err(|error| {
        AppError::new(
            "ui_start_failed",
            format!("Cannot spawn Tasker UI child: {error}"),
            ErrorCategory::Io,
        )
    })?;
    let started = Instant::now();
    loop {
        if let Some(state) = paths.read_state()?
            && state.launch_id == launch_id
        {
            verify_state_root(root, &state)?;
            verify_health(&state)?;
            let mut result = LifecycleResult::running(operation, true, &state, open);
            maybe_open(&mut result);
            return Ok(result);
        }
        if let Some(status) = child.try_wait().map_err(|error| {
            AppError::new(
                "ui_start_failed",
                format!("Cannot inspect Tasker UI child: {error}"),
                ErrorCategory::Io,
            )
        })? {
            if let Some(report) = paths.read_launch_error(&launch_id) {
                return Err(AppError::new(
                    "ui_start_failed",
                    format!(
                        "Tasker UI child failed [{}]: {}",
                        report.code, report.message
                    ),
                    ErrorCategory::Io,
                )
                .detail("child_error_code", report.code));
            }
            return Err(AppError::new(
                "ui_start_failed",
                format!("Tasker UI child exited before readiness with {status}"),
                ErrorCategory::Io,
            ));
        }
        if started.elapsed() >= LIFECYCLE_TIMEOUT {
            return Err(AppError::new(
                "ui_start_timeout",
                "Timed out waiting for Tasker UI readiness",
                ErrorCategory::Io,
            )
            .detail("launch_id", launch_id));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn stop_locked(root: &Path, paths: &RuntimePaths) -> Result<bool> {
    let Some(state) = inspect(root, paths)? else {
        return Ok(false);
    };
    let response = http_request(&state, "POST", "/__shutdown", "")?;
    if response.0 != 200 {
        return Err(AppError::new(
            "ui_instance_unreachable",
            "Tasker UI rejected its authenticated shutdown request",
            ErrorCategory::Conflict,
        )
        .detail("http_status", response.0));
    }
    let started = Instant::now();
    loop {
        if let Some(instance) = paths.try_lock_instance()? {
            drop(instance);
            paths.clear_transient();
            return Ok(true);
        }
        if started.elapsed() >= LIFECYCLE_TIMEOUT {
            return Err(AppError::new(
                "ui_stop_timeout",
                "Timed out waiting for the Tasker UI instance lock to be released",
                ErrorCategory::Conflict,
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn inspect(root: &Path, paths: &RuntimePaths) -> Result<Option<RuntimeState>> {
    if let Some(instance) = paths.try_lock_instance()? {
        drop(instance);
        paths.clear_transient();
        return Ok(None);
    }
    let state = paths.read_state()?.ok_or_else(|| {
        AppError::new(
            "ui_instance_unreachable",
            "The UI instance lock is held but no runtime state is available",
            ErrorCategory::Conflict,
        )
    })?;
    verify_state_root(root, &state)?;
    verify_health(&state)?;
    Ok(Some(state))
}

fn verify_state_root(root: &Path, state: &RuntimeState) -> Result<()> {
    if state.root_fingerprint != root_fingerprint(root) || Path::new(&state.projects_root) != root {
        return Err(AppError::new(
            "ui_state_corrupt",
            "UI runtime state does not match this projects root",
            ErrorCategory::Conflict,
        ));
    }
    Ok(())
}

fn verify_health(state: &RuntimeState) -> Result<()> {
    let (status, body) = http_request(state, "GET", "/__health", "").map_err(|error| {
        AppError::new(
            "ui_instance_unreachable",
            format!("UI health handshake failed: {}", error.message),
            ErrorCategory::Conflict,
        )
    })?;
    if status != 200 {
        return Err(AppError::new(
            "ui_instance_unreachable",
            format!("UI health endpoint returned HTTP {status}"),
            ErrorCategory::Conflict,
        ));
    }
    let envelope: ApiEnvelope<Health> = serde_json::from_slice(&body).map_err(|error| {
        AppError::new(
            "ui_instance_unreachable",
            format!("UI health response is malformed: {error}"),
            ErrorCategory::Conflict,
        )
    })?;
    if envelope.api_version != 1
        || !envelope.ok
        || envelope.data.instance_id != state.instance_id
        || envelope.data.launch_id != state.launch_id
        || envelope.data.root_fingerprint != state.root_fingerprint
        || envelope.data.server_version.is_empty()
    {
        return Err(AppError::new(
            "ui_instance_unreachable",
            "UI health response does not match the recorded instance",
            ErrorCategory::Conflict,
        ));
    }
    Ok(())
}

fn spawn_child(root: &Path, launch_id: &str) -> std::io::Result<Child> {
    let executable = std::env::current_exe()?;
    let mut command = Command::new(executable);
    command
        .arg("ui")
        .arg("serve")
        .arg("--projects-root")
        .arg(root)
        .arg("--launch-id")
        .arg(launch_id)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW};
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is async-signal-safe and this hook performs no allocation.
        unsafe {
            command.pre_exec(|| {
                nix::unistd::setsid()
                    .map(|_| ())
                    .map_err(std::io::Error::other)
            });
        }
    }
    #[cfg(windows)]
    let inheritance = StandardHandleInheritance::disable()?;
    let child = command.spawn();
    #[cfg(windows)]
    drop(inheritance);
    child
}

#[cfg(windows)]
struct StandardHandleInheritance {
    handles: Vec<(windows_sys::Win32::Foundation::HANDLE, u32)>,
}

#[cfg(windows)]
impl StandardHandleInheritance {
    fn disable() -> std::io::Result<Self> {
        use windows_sys::Win32::Foundation::{
            GetHandleInformation, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation,
        };
        use windows_sys::Win32::System::Console::{
            GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        };
        let mut handles = Vec::new();
        for kind in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            // SAFETY: GetStdHandle returns a borrowed process handle; it is never closed here.
            let handle = unsafe { GetStdHandle(kind) };
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                continue;
            }
            let mut flags = 0;
            // SAFETY: flags points to initialized writable memory and handle remains live.
            if unsafe { GetHandleInformation(handle, &mut flags) } == 0 {
                continue;
            }
            // SAFETY: this updates only the inherit bit on a live process-owned handle.
            if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) } == 0 {
                let error = std::io::Error::last_os_error();
                for (handle, original) in handles.drain(..) {
                    // SAFETY: restoring the bit on handles obtained above.
                    unsafe {
                        SetHandleInformation(
                            handle,
                            HANDLE_FLAG_INHERIT,
                            original & HANDLE_FLAG_INHERIT,
                        );
                    }
                }
                return Err(error);
            }
            handles.push((handle, flags));
        }
        Ok(Self { handles })
    }
}

#[cfg(windows)]
impl Drop for StandardHandleInheritance {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation};
        for (handle, original) in self.handles.drain(..) {
            // SAFETY: restoring the bit on process-owned handles captured by disable.
            unsafe {
                SetHandleInformation(handle, HANDLE_FLAG_INHERIT, original & HANDLE_FLAG_INHERIT);
            }
        }
    }
}

fn maybe_open(result: &mut LifecycleResult) {
    if !result.open_requested {
        return;
    }
    let Some(url) = result.url.as_deref() else {
        return;
    };
    match webbrowser::open(url) {
        Ok(()) => result.browser_opened = Some(true),
        Err(error) => {
            result.browser_opened = Some(false);
            result
                .warnings
                .push(format!("Could not open the default browser: {error}"));
        }
    }
}

fn http_request(
    state: &RuntimeState,
    method: &str,
    path: &str,
    body: &str,
) -> Result<(u16, Vec<u8>)> {
    let started = Instant::now();
    loop {
        match http_request_once(state, method, path, body) {
            Ok(response) if response.0 != 503 => return Ok(response),
            Ok(response) if started.elapsed() >= Duration::from_secs(4) => return Ok(response),
            Ok(_) => {}
            Err(error) if started.elapsed() >= Duration::from_secs(4) => return Err(error),
            Err(_) => {}
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn http_request_once(
    state: &RuntimeState,
    method: &str,
    path: &str,
    body: &str,
) -> Result<(u16, Vec<u8>)> {
    let address = SocketAddr::from(([127, 0, 0, 1], state.port));
    let mut stream =
        TcpStream::connect_timeout(&address, Duration::from_millis(250)).map_err(|error| {
            AppError::new(
                "ui_instance_unreachable",
                format!("Cannot connect to Tasker UI: {error}"),
                ErrorCategory::Conflict,
            )
        })?;
    stream
        .set_read_timeout(Some(Duration::from_millis(900)))
        .and_then(|()| stream.set_write_timeout(Some(Duration::from_millis(900))))
        .map_err(|error| AppError::io(error, "cannot set UI lifecycle socket deadlines"))?;
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {ADDRESS}:{}\r\nX-Tasker-Token: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        state.port,
        state.token,
        body.len(),
        body
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|error| AppError::io(error, "cannot send UI lifecycle request"))?;
    let mut response = Vec::new();
    stream
        .take(128 * 1024)
        .read_to_end(&mut response)
        .map_err(|error| AppError::io(error, "cannot read UI lifecycle response"))?;
    parse_http_response(&response)
}

fn parse_http_response(response: &[u8]) -> Result<(u16, Vec<u8>)> {
    let separator = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| {
            AppError::new(
                "ui_instance_unreachable",
                "UI returned an incomplete HTTP response",
                ErrorCategory::Conflict,
            )
        })?;
    let headers = std::str::from_utf8(&response[..separator]).map_err(|_| {
        AppError::new(
            "ui_instance_unreachable",
            "UI returned non-UTF-8 HTTP headers",
            ErrorCategory::Conflict,
        )
    })?;
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| {
            AppError::new(
                "ui_instance_unreachable",
                "UI returned an invalid HTTP status line",
                ErrorCategory::Conflict,
            )
        })?;
    Ok((status, response[separator + 4..].to_vec()))
}

pub fn write_launch_failure(root: &Path, launch_id: &str, error: &AppError) {
    if let Ok(paths) = RuntimePaths::prepare(root) {
        paths.write_launch_error(launch_id, error);
    }
}
