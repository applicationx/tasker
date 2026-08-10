use crate::cli::OutputFormat;
use crate::error::{AppError, ErrorCategory, Result};
use crate::model::{Event, GlobalConfig, Meta, ProjectConfig, Task, User};
use atomic_write_file::AtomicWriteFile;
use directories::{BaseDirs, ProjectDirs};
use fs4::fs_std::FileExt;
use serde::{Serialize, de::DeserializeOwned};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Project {
    pub path: PathBuf,
    pub config: ProjectConfig,
}
pub struct ProjectLock {
    file: File,
}

pub fn is_symlink_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

impl Drop for ProjectLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}
pub fn lock_project(path: &Path) -> Result<ProjectLock> {
    let p = path.join(".tasker.lock");
    let f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&p)
        .map_err(|e| AppError::io(e, format!("cannot open {}", p.display())))?;
    f.lock_exclusive()
        .map_err(|e| AppError::io(e, "cannot acquire project lock"))?;
    Ok(ProjectLock { file: f })
}

pub fn lock_projects_root() -> Result<ProjectLock> {
    let root = projects_root();
    fs::create_dir_all(&root)
        .map_err(|e| AppError::io(e, format!("cannot create {}", root.display())))?;
    let path = root.join(".tasker-root.lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| AppError::io(e, format!("cannot open {}", path.display())))?;
    file.lock_exclusive()
        .map_err(|e| AppError::io(e, "cannot acquire projects root lock"))?;
    Ok(ProjectLock { file })
}

pub fn expand_path(value: &str) -> PathBuf {
    if (value == "~" || value.starts_with("~/") || value.starts_with("~\\"))
        && let Some(b) = BaseDirs::new()
    {
        return b.home_dir().join(
            value
                .trim_start_matches('~')
                .trim_start_matches(['/', '\\']),
        );
    }
    PathBuf::from(value)
}
fn config_path() -> PathBuf {
    ProjectDirs::from("dev", "Tasker", "tasker")
        .map(|d| d.config_dir().join("config.toml"))
        .unwrap_or_else(|| PathBuf::from(".tasker-config.toml"))
}
pub fn load_global_config() -> GlobalConfig {
    fs::read_to_string(config_path())
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}
pub fn save_global_config(c: &GlobalConfig) -> Result<()> {
    let p = config_path();
    if let Some(d) = p.parent() {
        fs::create_dir_all(d)
            .map_err(|e| AppError::io(e, "cannot create configuration directory"))?;
    }
    let s = toml::to_string_pretty(c)
        .map_err(|e| AppError::input(format!("cannot serialize configuration: {e}")))?;
    atomic_write(&p, format!("{s}\n").as_bytes())
}
pub fn configured_output() -> Option<OutputFormat> {
    load_global_config()
        .default_output
        .and_then(|s| s.parse().ok())
}
pub fn projects_root() -> PathBuf {
    if let Ok(s) = std::env::var("TASKER_ROOT") {
        return expand_path(&s);
    }
    if let Some(s) = load_global_config().projects_root {
        return expand_path(&s);
    }
    BaseDirs::new()
        .map(|b| b.home_dir().join("tasker"))
        .unwrap_or_else(|| PathBuf::from("tasker"))
}
pub fn effective_config_value() -> serde_json::Value {
    let c = load_global_config();
    serde_json::json!({"projects_root":projects_root().to_string_lossy(),"default_output":std::env::var("TASKER_OUTPUT").ok().or(c.default_output).unwrap_or_else(||"human".into()),"config_file":config_path().to_string_lossy()})
}

pub fn discover_projects() -> Result<Vec<Project>> {
    let root = projects_root();
    if !root.exists() {
        return Ok(vec![]);
    }
    let mut out = vec![];
    for e in fs::read_dir(&root)
        .map_err(|e| AppError::io(e, format!("cannot scan {}", root.display())))?
    {
        let p = e
            .map_err(|e| AppError::io(e, "cannot read project root entry"))?
            .path();
        if p.is_dir()
            && p.join("tasker.yaml").is_file()
            && let Ok(config) = read_yaml(&p.join("tasker.yaml"))
        {
            out.push(Project { path: p, config });
        }
    }
    out.sort_by(|a, b| {
        a.config
            .name
            .to_lowercase()
            .cmp(&b.config.name.to_lowercase())
    });
    Ok(out)
}
pub fn resolve_project(selector: Option<&str>, task_id: Option<&str>) -> Result<Project> {
    if let Some(s) = selector {
        return resolve_selector(s);
    }
    if let Some(id) = task_id {
        let upper = id.to_ascii_uppercase();
        let prefix = upper
            .split_once("-R")
            .or_else(|| upper.split_once("-D"))
            .map(|(prefix, _)| prefix)
            .or_else(|| upper.rsplit_once('-').map(|(prefix, _)| prefix));
        if let Some(prefix) = prefix
            && let Ok(project) = resolve_selector(prefix)
        {
            return Ok(project);
        }
    }
    if let Ok(s) = std::env::var("TASKER_PROJECT") {
        return resolve_selector(&s);
    }
    let mut cwd = std::env::current_dir()
        .map_err(|e| AppError::io(e, "cannot determine current directory"))?;
    loop {
        if cwd.join("tasker.yaml").is_file() {
            return load_project(&cwd);
        }
        if !cwd.pop() {
            break;
        }
    }
    Err(AppError::new(
        "project_required",
        "No project selected; use --project, a task ID, TASKER_PROJECT, or run inside a project",
        ErrorCategory::Project,
    ))
}
pub fn resolve_selector(s: &str) -> Result<Project> {
    let expanded = expand_path(s);
    if expanded.join("tasker.yaml").is_file() {
        return load_project(&expanded);
    }
    if expanded.is_absolute() || s.contains('/') || s.contains('\\') {
        return Err(project_not_found(s));
    }
    let direct = projects_root().join(s);
    if direct.join("tasker.yaml").is_file() {
        return load_project(&direct);
    }
    let low = s.to_lowercase();
    let found: Vec<_> = discover_projects()?
        .into_iter()
        .filter(|p| {
            p.config.prefix.eq_ignore_ascii_case(s)
                || p.config.name.to_lowercase() == low
                || p.path
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().to_lowercase() == low)
        })
        .collect();
    if found.is_empty() {
        // A partially readable YAML document can still identify its project for
        // `tasker validate`, which then reports the full typed-config error.
        let root = projects_root();
        if let Ok(entries) = fs::read_dir(root) {
            for path in entries.filter_map(|entry| entry.ok().map(|entry| entry.path())) {
                let config_path = path.join("tasker.yaml");
                let partial = fs::read_to_string(&config_path)
                    .ok()
                    .and_then(|text| serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&text).ok());
                let matches = partial.as_ref().is_some_and(|value| {
                    value
                        .get("prefix")
                        .and_then(serde_yaml_ng::Value::as_str)
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(s))
                        || value
                            .get("name")
                            .and_then(serde_yaml_ng::Value::as_str)
                            .is_some_and(|name| name.eq_ignore_ascii_case(s))
                });
                if matches {
                    return load_project(&path);
                }
            }
        }
    }
    match found.len() {
        0 => Err(project_not_found(s)),
        1 => Ok(found.into_iter().next().unwrap()),
        _ => Err(AppError::new(
            "project_ambiguous",
            format!("Project selector {s} is ambiguous"),
            ErrorCategory::Conflict,
        )
        .detail("project", s.to_string())),
    }
}
fn project_not_found(s: &str) -> AppError {
    AppError::new(
        "project_not_found",
        format!("Project {s} does not exist"),
        ErrorCategory::NotFound,
    )
    .detail("project", s.to_string())
}
pub fn load_project(path: &Path) -> Result<Project> {
    let p = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let config = read_yaml(&p.join("tasker.yaml")).map_err(|e| {
        AppError::new("invalid_project_config", e.message, ErrorCategory::Project)
            .detail("path", p.join("tasker.yaml").to_string_lossy().to_string())
    })?;
    Ok(Project { path: p, config })
}
pub fn reload_project(p: &Project) -> Result<Project> {
    load_project(&p.path)
}

pub fn read_task(p: &Project, id: &str) -> Result<Task> {
    let path = p.path.join("tasks").join(format!("{id}.json"));
    if !path.is_file() {
        return Err(AppError::new(
            "task_not_found",
            format!("Task {id} does not exist"),
            ErrorCategory::NotFound,
        )
        .detail("task_id", id.to_string()));
    }
    let mut task: Task = read_json(&path).map_err(|e| {
        AppError::project(format!("Malformed task {}: {}", path.display(), e.message))
    })?;
    task.normalize();
    Ok(task)
}
pub fn read_tasks(p: &Project) -> Result<Vec<Task>> {
    let mut out: Vec<Task> = vec![];
    for path in json_files(&p.path.join("tasks"))? {
        let mut task: Task = read_json(&path).map_err(|e| {
            AppError::project(format!("Malformed task {}: {}", path.display(), e.message))
        })?;
        task.normalize();
        out.push(task);
    }
    out.sort_by(|a, b| crate::model::task_id_cmp(&a.id, &b.id));
    Ok(out)
}
pub fn write_task(p: &Project, t: &Task) -> Result<()> {
    write_json(&p.path.join("tasks").join(format!("{}.json", t.id)), t)
}
pub fn read_meta(p: &Project) -> Result<Meta> {
    read_json(&p.path.join("meta.json"))
        .map_err(|e| AppError::project(format!("Invalid meta.json: {}", e.message)))
}
pub fn write_meta(p: &Project, m: &Meta) -> Result<()> {
    write_json(&p.path.join("meta.json"), m)
}
pub fn read_users(p: &Project) -> Result<Vec<User>> {
    let mut out: Vec<User> = vec![];
    for path in json_files(&p.path.join("users"))? {
        out.push(read_json(&path).map_err(|e| {
            AppError::project(format!("Malformed user {}: {}", path.display(), e.message))
        })?)
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}
pub fn read_user_file(p: &Project, id: &str) -> Result<User> {
    let path = p.path.join("users").join(format!("{id}.json"));
    if !path.is_file() {
        return Err(AppError::new(
            "user_not_found",
            format!("User {id} does not exist"),
            ErrorCategory::NotFound,
        )
        .detail("user", id.to_string()));
    }
    read_json(&path)
        .map_err(|e| AppError::project(format!("Malformed user {}: {}", path.display(), e.message)))
}
pub fn write_user(p: &Project, u: &User) -> Result<()> {
    write_json(&p.path.join("users").join(format!("{}.json", u.id)), u)
}
pub fn read_events(p: &Project) -> Result<Vec<Event>> {
    let mut out: Vec<Event> = vec![];
    for path in json_files(&p.path.join("changelog"))? {
        out.push(read_json(&path).map_err(|e| {
            AppError::project(format!(
                "Malformed changelog event {}: {}",
                path.display(),
                e.message
            ))
        })?)
    }
    out.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    Ok(out)
}
pub fn write_event(p: &Project, e: &Event) -> Result<()> {
    let ts: String = e
        .timestamp
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    write_json(
        &p.path
            .join("changelog")
            .join(format!("{}_{}.json", ts, Uuid::new_v4())),
        e,
    )
}

pub fn json_files(dir: &Path) -> Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut out = vec![];
    for e in
        fs::read_dir(dir).map_err(|e| AppError::io(e, format!("cannot scan {}", dir.display())))?
    {
        let p = e
            .map_err(|e| AppError::io(e, "cannot read directory entry"))?
            .path();
        if p.extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("json"))
        {
            out.push(p)
        }
    }
    out.sort();
    Ok(out)
}
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let s = fs::read_to_string(path)
        .map_err(|e| AppError::io(e, format!("cannot read {}", path.display())))?;
    serde_json::from_str(&s).map_err(|e| AppError::project(format!("{}: {e}", path.display())))
}
pub fn read_yaml<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let s = fs::read_to_string(path)
        .map_err(|e| AppError::io(e, format!("cannot read {}", path.display())))?;
    serde_yaml_ng::from_str(&s).map_err(|e| AppError::project(format!("{}: {e}", path.display())))
}
pub fn write_json<T: Serialize>(path: &Path, v: &T) -> Result<()> {
    let mut b = serde_json::to_vec_pretty(v)
        .map_err(|e| AppError::input(format!("cannot serialize JSON: {e}")))?;
    b.push(b'\n');
    write_if_changed(path, &b)
}
pub fn write_yaml<T: Serialize>(path: &Path, v: &T) -> Result<()> {
    let s = serde_yaml_ng::to_string(v)
        .map_err(|e| AppError::input(format!("cannot serialize YAML: {e}")))?;
    write_if_changed(path, s.as_bytes())
}
fn write_if_changed(path: &Path, b: &[u8]) -> Result<()> {
    if fs::read(path).ok().as_deref() == Some(b) {
        return Ok(());
    }
    if let Some(d) = path.parent() {
        fs::create_dir_all(d)
            .map_err(|e| AppError::io(e, format!("cannot create {}", d.display())))?
    }
    atomic_write(path, b)
}
pub fn atomic_write(path: &Path, b: &[u8]) -> Result<()> {
    let mut f = AtomicWriteFile::options().open(path).map_err(|e| {
        AppError::io(
            e,
            format!("cannot open atomic writer for {}", path.display()),
        )
    })?;
    f.write_all(b)
        .map_err(|e| AppError::io(e, format!("cannot write {}", path.display())))?;
    f.commit()
        .map_err(|e| AppError::io(e, format!("cannot commit {}", path.display())))
}
pub fn read_input(path: Option<&str>) -> Result<String> {
    if let Some(p) = path {
        return fs::read_to_string(expand_path(p))
            .map_err(|e| AppError::io(e, format!("cannot read input file {p}")));
    }
    let mut s = String::new();
    std::io::stdin()
        .read_to_string(&mut s)
        .map_err(|e| AppError::io(e, "cannot read stdin"))?;
    Ok(s)
}
