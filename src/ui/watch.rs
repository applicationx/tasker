use crate::error::{AppError, ErrorCategory, Result};
use notify_debouncer_mini::notify::{
    self, Config as NotifyConfig, Event, EventKind, PollWatcher, RecommendedWatcher, RecursiveMode,
    Watcher,
};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Quiet period that closes a batch of filesystem changes.
const DEBOUNCE: Duration = Duration::from_millis(250);
/// Upper bound on how long a continuously changing batch is held back.
const MAX_BATCH_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Serialize)]
pub struct ChangeHint {
    pub generation: u64,
    pub project_prefix: Option<String>,
    pub reason: String,
}

pub struct ChangeSignal {
    generation: AtomicU64,
    latest: Mutex<ChangeHint>,
    changed: Condvar,
}

impl ChangeSignal {
    pub fn new() -> Self {
        Self {
            generation: AtomicU64::new(0),
            latest: Mutex::new(ChangeHint {
                generation: 0,
                project_prefix: None,
                reason: "ready".to_string(),
            }),
            changed: Condvar::new(),
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub fn notify(&self, project_prefix: Option<String>, reason: impl Into<String>) {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        if let Ok(mut latest) = self.latest.lock() {
            *latest = ChangeHint {
                generation,
                project_prefix,
                reason: reason.into(),
            };
            self.changed.notify_all();
        }
    }

    pub fn wake_all(&self) {
        self.changed.notify_all();
    }

    pub fn wait(&self, since: u64, timeout: Duration, shutdown: &AtomicBool) -> ChangeHint {
        let mut latest = self
            .latest
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        while latest.generation <= since && !shutdown.load(Ordering::Acquire) {
            let result = self.changed.wait_timeout(latest, timeout);
            match result {
                Ok((guard, timed_out)) => {
                    latest = guard;
                    if timed_out.timed_out() {
                        break;
                    }
                }
                Err(poison) => {
                    latest = poison.into_inner().0;
                    break;
                }
            }
        }
        latest.clone()
    }
}

/// Owns the watcher; dropping it drops the event sender, which ends the
/// debounce thread.
pub enum WatchGuard {
    Native { _watcher: RecommendedWatcher },
    Poll { _watcher: PollWatcher },
}

impl WatchGuard {
    pub fn mode(&self) -> &'static str {
        match self {
            Self::Native { .. } => "native",
            Self::Poll { .. } => "poll",
        }
    }
}

pub fn start(
    root: &Path,
    changes: Arc<ChangeSignal>,
    shutdown: Arc<AtomicBool>,
) -> Result<(WatchGuard, Vec<String>)> {
    let events = spawn_debouncer(root, Arc::clone(&changes), Arc::clone(&shutdown))?;
    match RecommendedWatcher::new(events, NotifyConfig::default()) {
        Ok(mut watcher) => match register(&mut watcher, root) {
            Ok(()) => Ok((WatchGuard::Native { _watcher: watcher }, Vec::new())),
            Err(native_error) => poll_fallback(root, native_error, changes, shutdown),
        },
        Err(error) => poll_fallback(root, error.to_string(), changes, shutdown),
    }
}

fn spawn_debouncer(
    root: &Path,
    changes: Arc<ChangeSignal>,
    shutdown: Arc<AtomicBool>,
) -> Result<Sender<notify::Result<Event>>> {
    let (sender, receiver) = mpsc::channel();
    let root = root.to_path_buf();
    thread::Builder::new()
        .name("tasker-ui-watch".to_string())
        .spawn(move || debounce(receiver, &root, &changes, &shutdown))
        .map_err(|error| {
            AppError::new(
                "ui_start_failed",
                format!("Cannot start filesystem watcher thread: {error}"),
                ErrorCategory::Io,
            )
        })?;
    Ok(sender)
}

/// Batches changed paths until DEBOUNCE passes without a further change (or
/// MAX_BATCH_DELAY elapses). Access events are dropped before batching:
/// inotify reports every open/read, so serving the UI, or loading a project
/// to name a change, would otherwise keep signalling changes forever.
fn debounce(
    receiver: Receiver<notify::Result<Event>>,
    root: &Path,
    changes: &ChangeSignal,
    shutdown: &AtomicBool,
) {
    let mut pending = BTreeSet::<PathBuf>::new();
    let mut first = Instant::now();
    let mut last = first;
    loop {
        let received = if pending.is_empty() {
            receiver.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            let deadline = (last + DEBOUNCE).min(first + MAX_BATCH_DELAY);
            receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        };
        match received {
            Ok(Ok(event)) if matches!(event.kind, EventKind::Access(_)) => {}
            Ok(Ok(event)) => {
                let now = Instant::now();
                if pending.is_empty() {
                    first = now;
                }
                last = now;
                pending.extend(event.paths);
            }
            Ok(Err(_)) => changes.notify(None, "watcher_error"),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let now = Instant::now();
        if !pending.is_empty() && (now >= last + DEBOUNCE || now >= first + MAX_BATCH_DELAY) {
            let batch = std::mem::take(&mut pending);
            if !handle_paths(batch, root, changes, shutdown) {
                return;
            }
        }
    }
}

fn poll_fallback(
    root: &Path,
    native_error: String,
    changes: Arc<ChangeSignal>,
    shutdown: Arc<AtomicBool>,
) -> Result<(WatchGuard, Vec<String>)> {
    let events = spawn_debouncer(root, changes, shutdown)?;
    let config = NotifyConfig::default().with_poll_interval(Duration::from_secs(5));
    let mut watcher = PollWatcher::new(events, config).map_err(|error| {
        AppError::new(
            "ui_start_failed",
            format!("Cannot initialize native or polling filesystem watcher: {error}"),
            ErrorCategory::Io,
        )
    })?;
    register(&mut watcher, root).map_err(|error| {
        AppError::new(
            "ui_start_failed",
            format!("Cannot register polling filesystem watcher: {error}"),
            ErrorCategory::Io,
        )
    })?;
    Ok((
        WatchGuard::Poll { _watcher: watcher },
        vec![format!(
            "Native filesystem watcher unavailable; using low-frequency polling: {native_error}"
        )],
    ))
}

fn register(watcher: &mut dyn Watcher, root: &Path) -> std::result::Result<(), String> {
    watcher
        .watch(root, RecursiveMode::Recursive)
        .map_err(|error| format!("{}: {error}", root.display()))
}

/// Signals one change for a batch of paths. Returns false once the root is
/// gone, which shuts the UI down.
fn handle_paths(
    paths: BTreeSet<PathBuf>,
    root: &Path,
    changes: &ChangeSignal,
    shutdown: &AtomicBool,
) -> bool {
    if !root.is_dir() {
        shutdown.store(true, Ordering::Release);
        changes.wake_all();
        return false;
    }
    let relevant = paths
        .into_iter()
        .filter(|path| relevant_path(root, path))
        .collect::<Vec<_>>();
    if relevant.is_empty() {
        return true;
    }
    let prefixes = relevant
        .iter()
        .filter_map(|path| canonical_project_prefix(root, path))
        .collect::<BTreeSet<_>>();
    changes.notify(
        (prefixes.len() == 1).then(|| prefixes.into_iter().next().unwrap()),
        "filesystem",
    );
    true
}

fn canonical_project_prefix(root: &Path, changed: &Path) -> Option<String> {
    let directory = changed.strip_prefix(root).ok()?.components().next()?;
    let project_path = root.join(directory.as_os_str());
    crate::storage::load_project(&project_path)
        .ok()
        .map(|project| project.config.prefix)
}

fn relevant_path(root: &Path, path: &Path) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    !relative.components().any(|component| {
        matches!(
            component.as_os_str().to_str(),
            Some(".tasker.lock" | ".tasker-root.lock" | ".tasker-transactions")
        )
    })
}
