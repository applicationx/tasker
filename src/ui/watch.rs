use crate::error::{AppError, ErrorCategory, Result};
use notify_debouncer_mini::notify::{
    Config as NotifyConfig, PollWatcher, RecommendedWatcher, RecursiveMode, Watcher,
};
use notify_debouncer_mini::{
    Config, DebounceEventResult, Debouncer, new_debouncer, new_debouncer_opt,
};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

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

pub enum WatchGuard {
    Native {
        _watcher: Debouncer<RecommendedWatcher>,
    },
    Poll {
        _watcher: Debouncer<PollWatcher>,
    },
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
    let root_path = root.to_path_buf();
    let native_changes = Arc::clone(&changes);
    let native_shutdown = Arc::clone(&shutdown);
    let handler = move |result: DebounceEventResult| {
        handle_events(result, &root_path, &native_changes, &native_shutdown);
    };
    match new_debouncer(Duration::from_millis(250), handler) {
        Ok(mut debouncer) => match register(debouncer.watcher(), root) {
            Ok(()) => Ok((
                WatchGuard::Native {
                    _watcher: debouncer,
                },
                Vec::new(),
            )),
            Err(native_error) => poll_fallback(root, native_error, changes, shutdown),
        },
        Err(error) => poll_fallback(root, error.to_string(), changes, shutdown),
    }
}

fn poll_fallback(
    root: &Path,
    native_error: String,
    changes: Arc<ChangeSignal>,
    shutdown: Arc<AtomicBool>,
) -> Result<(WatchGuard, Vec<String>)> {
    let root_path = root.to_path_buf();
    let config = Config::default()
        .with_timeout(Duration::from_millis(250))
        .with_notify_config(NotifyConfig::default().with_poll_interval(Duration::from_secs(5)));
    let mut debouncer = new_debouncer_opt::<_, PollWatcher>(config, move |result| {
        handle_events(result, &root_path, &changes, &shutdown);
    })
    .map_err(|error| {
        AppError::new(
            "ui_start_failed",
            format!("Cannot initialize native or polling filesystem watcher: {error}"),
            ErrorCategory::Io,
        )
    })?;
    register(debouncer.watcher(), root).map_err(|error| {
        AppError::new(
            "ui_start_failed",
            format!("Cannot register polling filesystem watcher: {error}"),
            ErrorCategory::Io,
        )
    })?;
    Ok((
        WatchGuard::Poll {
            _watcher: debouncer,
        },
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

fn handle_events(
    result: DebounceEventResult,
    root: &Path,
    changes: &ChangeSignal,
    shutdown: &AtomicBool,
) {
    if !root.is_dir() {
        shutdown.store(true, Ordering::Release);
        changes.wake_all();
        return;
    }
    match result {
        Ok(events) => {
            let mut relevant = events
                .into_iter()
                .filter(|event| relevant_path(root, &event.path))
                .collect::<Vec<_>>();
            if relevant.is_empty() {
                return;
            }
            relevant.sort_by(|left, right| left.path.cmp(&right.path));
            let prefixes = relevant
                .iter()
                .filter_map(|event| canonical_project_prefix(root, &event.path))
                .collect::<BTreeSet<_>>();
            changes.notify(
                (prefixes.len() == 1).then(|| prefixes.into_iter().next().unwrap()),
                "filesystem",
            );
        }
        Err(_) => changes.notify(None, "watcher_error"),
    }
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
