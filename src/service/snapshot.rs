use crate::app;
use crate::error::{AppError, ErrorCategory, Result};
use crate::knowledge::model::{DecisionMeta, ResourceMeta};
use crate::model::{Event, RelationType, StatusView, Task, User, Workflow};
use crate::storage::{self, Project};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct ProjectListItem {
    pub name: String,
    pub prefix: String,
    pub path: String,
    pub stats: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectView {
    pub name: String,
    pub prefix: String,
    pub path: String,
    pub workflow: Workflow,
    pub relations: BTreeMap<String, RelationType>,
}

#[derive(Debug, Serialize)]
pub struct TaskView {
    pub task: Task,
    pub status: StatusView,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResourceSummary {
    #[serde(flatten)]
    pub metadata: ResourceMeta,
    pub source_type: ResourceSourceType,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceSourceType {
    Managed,
    Referenced,
}

#[derive(Debug, Clone, Serialize)]
pub struct DecisionSummary {
    #[serde(flatten)]
    pub metadata: DecisionMeta,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReverseReference {
    pub task_id: String,
    pub role: crate::model::ContextReferenceRole,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<Value>,
    pub warnings: Vec<Value>,
}

#[derive(Debug, Serialize)]
pub struct ProjectSnapshot {
    pub project: ProjectView,
    pub tasks: Vec<TaskView>,
    pub users: Vec<User>,
    pub resources: Vec<ResourceSummary>,
    pub decisions: Vec<DecisionSummary>,
    pub reverse_context_refs: BTreeMap<String, Vec<ReverseReference>>,
    pub recent_events: Vec<Event>,
    pub validation: ValidationResult,
    pub change_generation: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResourceView {
    #[serde(flatten)]
    pub metadata: ResourceMeta,
    pub source_type: ResourceSourceType,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DecisionView {
    #[serde(flatten)]
    pub metadata: DecisionMeta,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectBriefView {
    pub path: String,
    pub content: String,
}

pub fn canonical_projects_root(root: &Path) -> Result<PathBuf> {
    let canonical = fs::canonicalize(root).map_err(|error| {
        AppError::new(
            "ui_root_unavailable",
            format!("Projects root {} is unavailable: {error}", root.display()),
            ErrorCategory::Project,
        )
        .detail("projects_root", root.to_string_lossy().to_string())
    })?;
    let metadata = fs::symlink_metadata(&canonical).map_err(|error| {
        AppError::new(
            "ui_root_unavailable",
            format!(
                "Cannot inspect projects root {}: {error}",
                canonical.display()
            ),
            ErrorCategory::Project,
        )
    })?;
    if !metadata.file_type().is_dir() || storage::is_symlink_or_reparse(&metadata) {
        return Err(AppError::new(
            "ui_root_unavailable",
            "Projects root must be a real, non-reparse directory",
            ErrorCategory::Project,
        )
        .detail("projects_root", canonical.to_string_lossy().to_string()));
    }
    if canonical.to_str().is_none() {
        return Err(AppError::new(
            "ui_root_unavailable",
            "The canonical projects root cannot be represented as UTF-8 for display",
            ErrorCategory::Project,
        ));
    }
    Ok(canonical)
}

fn direct_projects(root: &Path) -> Result<Vec<Project>> {
    let root = canonical_projects_root(root)?;
    let mut projects = Vec::new();
    for entry in fs::read_dir(&root)
        .map_err(|error| AppError::io(error, format!("cannot scan {}", root.display())))?
    {
        let path = entry
            .map_err(|error| AppError::io(error, "cannot read projects root entry"))?
            .path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.file_type().is_dir()
            || storage::is_symlink_or_reparse(&metadata)
            || !path.join("tasker.yaml").is_file()
        {
            continue;
        }
        let Ok(canonical) = fs::canonicalize(&path) else {
            continue;
        };
        if canonical.parent() != Some(root.as_path()) {
            continue;
        }
        if let Ok(project) = storage::load_project(&canonical) {
            projects.push(project);
        }
    }
    projects.sort_by(|left, right| {
        left.config
            .name
            .to_ascii_lowercase()
            .cmp(&right.config.name.to_ascii_lowercase())
            .then_with(|| left.config.prefix.cmp(&right.config.prefix))
    });
    Ok(projects)
}

pub(crate) fn resolve_project(root: &Path, prefix: &str) -> Result<Project> {
    if prefix.is_empty()
        || !prefix
            .chars()
            .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit())
    {
        return Err(AppError::input(
            "UI project selectors must be uppercase project prefixes",
        ));
    }
    direct_projects(root)?
        .into_iter()
        .find(|project| project.config.prefix == prefix)
        .ok_or_else(|| {
            AppError::new(
                "project_not_found",
                format!("Project {prefix} does not exist beneath this UI projects root"),
                ErrorCategory::NotFound,
            )
            .detail("project", prefix.to_string())
        })
}

pub fn list_projects(root: &Path) -> Result<Vec<ProjectListItem>> {
    direct_projects(root)?
        .into_iter()
        .map(|project| {
            let (_lock, project) = lock_and_reload(project)?;
            let mut stats = BTreeMap::new();
            for task in storage::read_tasks(&project)? {
                *stats.entry(task.state).or_insert(0) += 1;
            }
            Ok(ProjectListItem {
                name: project.config.name,
                prefix: project.config.prefix,
                path: project.path.to_string_lossy().to_string(),
                stats,
            })
        })
        .collect()
}

fn lock_and_reload(project: Project) -> Result<(storage::ProjectLock, Project)> {
    let lock = storage::lock_project(&project.path)?;
    let project = storage::reload_project(&project)?;
    crate::transaction::recover(&project)?;
    Ok((lock, project))
}

pub fn project_snapshot(root: &Path, prefix: &str, generation: u64) -> Result<ProjectSnapshot> {
    let project = resolve_project(root, prefix)?;
    let (_lock, project) = lock_and_reload(project)?;
    let tasks = storage::read_tasks(&project)?;
    let task_map: HashMap<String, Task> = tasks
        .iter()
        .cloned()
        .map(|task| (task.id.clone(), task))
        .collect();
    let task_views = tasks
        .iter()
        .cloned()
        .map(|task| TaskView {
            status: app::status_for(&task, &task_map, &project.config),
            task,
        })
        .collect();
    let users = storage::read_users(&project)?;
    let resources = crate::knowledge::read_resources(&project)?
        .into_iter()
        .map(|document| ResourceSummary {
            source_type: if document.metadata.source_path.is_some() {
                ResourceSourceType::Referenced
            } else {
                ResourceSourceType::Managed
            },
            metadata: document.metadata,
        })
        .collect();
    let decisions = crate::knowledge::read_decisions(&project)?
        .into_iter()
        .map(|document| DecisionSummary {
            metadata: document.metadata,
        })
        .collect();
    let mut reverse_context_refs: BTreeMap<String, Vec<ReverseReference>> = BTreeMap::new();
    for task in &tasks {
        for reference in &task.context_refs {
            reverse_context_refs
                .entry(reference.id.clone())
                .or_default()
                .push(ReverseReference {
                    task_id: task.id.clone(),
                    role: reference.role,
                });
        }
    }
    let validation_value = match app::validate_loaded_project(&project) {
        Ok(value) => value,
        Err(error) if error.code == "validation_failed" => error
            .details
            .get("result")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({"valid":false,"errors":[],"warnings":[]})),
        Err(error) => return Err(error),
    };
    let validation: ValidationResult = serde_json::from_value(validation_value)
        .map_err(|error| AppError::input(format!("cannot serialize validation result: {error}")))?;
    let recent_events = storage::read_events(&project)?
        .into_iter()
        .take(50)
        .collect();
    Ok(ProjectSnapshot {
        project: ProjectView {
            name: project.config.name.clone(),
            prefix: project.config.prefix.clone(),
            path: project.path.to_string_lossy().to_string(),
            workflow: project.config.workflow,
            relations: project.config.relations,
        },
        tasks: task_views,
        users,
        resources,
        decisions,
        reverse_context_refs,
        recent_events,
        validation,
        change_generation: generation,
    })
}

pub fn read_task(root: &Path, prefix: &str, id: &str) -> Result<TaskView> {
    let project = resolve_project(root, prefix)?;
    let (_lock, project) = lock_and_reload(project)?;
    if !id.starts_with(&format!("{}-", project.config.prefix)) {
        return Err(AppError::new(
            "task_project_mismatch",
            format!("Task {id} does not belong to project {prefix}"),
            ErrorCategory::Validation,
        ));
    }
    let task = storage::read_task(&project, id)?;
    let tasks: HashMap<String, Task> = storage::read_tasks(&project)?
        .into_iter()
        .map(|task| (task.id.clone(), task))
        .collect();
    Ok(TaskView {
        status: app::status_for(&task, &tasks, &project.config),
        task,
    })
}

pub fn read_resource(root: &Path, prefix: &str, id: &str) -> Result<ResourceView> {
    let project = resolve_project(root, prefix)?;
    let (_lock, project) = lock_and_reload(project)?;
    let id = crate::knowledge::canonical_id(&project, id, 'R')?;
    let document = crate::knowledge::read_resources(&project)?
        .into_iter()
        .find(|document| document.metadata.id == id)
        .ok_or_else(|| {
            AppError::new(
                "resource_not_found",
                format!("Resource {id} does not exist"),
                ErrorCategory::NotFound,
            )
        })?;
    let (source_type, content) = if let Some(path) = &document.metadata.source_path {
        (
            ResourceSourceType::Referenced,
            crate::knowledge::safe_path::read(&project.path, path)?,
        )
    } else {
        (ResourceSourceType::Managed, document.body)
    };
    Ok(ResourceView {
        metadata: document.metadata,
        source_type,
        content,
    })
}

pub fn read_decision(root: &Path, prefix: &str, id: &str) -> Result<DecisionView> {
    let project = resolve_project(root, prefix)?;
    let (_lock, project) = lock_and_reload(project)?;
    let id = crate::knowledge::canonical_id(&project, id, 'D')?;
    let document = crate::knowledge::read_decisions(&project)?
        .into_iter()
        .find(|document| document.metadata.id == id)
        .ok_or_else(|| {
            AppError::new(
                "decision_not_found",
                format!("Decision {id} does not exist"),
                ErrorCategory::NotFound,
            )
        })?;
    Ok(DecisionView {
        metadata: document.metadata,
        content: document.body,
    })
}

pub fn project_brief(root: &Path, prefix: &str) -> Result<ProjectBriefView> {
    let project = resolve_project(root, prefix)?;
    let (_lock, project) = lock_and_reload(project)?;
    let path = project.path.join(crate::knowledge::PROJECT_BRIEF_PATH);
    let content = fs::read_to_string(&path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            AppError::new(
                "project_brief_not_found",
                "PROJECT.md does not exist",
                ErrorCategory::NotFound,
            )
        } else {
            AppError::io(error, format!("cannot read {}", path.display()))
        }
    })?;
    Ok(ProjectBriefView {
        path: crate::knowledge::PROJECT_BRIEF_PATH.to_string(),
        content,
    })
}

pub fn brief(
    root: &Path,
    prefix: &str,
    task_id: Option<String>,
    max_bytes: usize,
    format: crate::cli::OutputFormat,
) -> Result<Value> {
    if !(256..=10_000_000).contains(&max_bytes) {
        return Err(AppError::input(
            "Brief max_bytes must be between 256 and 10000000",
        ));
    }
    let project = resolve_project(root, prefix)?;
    if let Some(task_id) = &task_id
        && !task_id.starts_with(&format!("{prefix}-"))
    {
        return Err(AppError::new(
            "task_project_mismatch",
            format!("Task {task_id} does not belong to project {prefix}"),
            ErrorCategory::Validation,
        ));
    }
    let args = crate::cli::BriefArgs {
        task: task_id,
        max_bytes,
        include_archived: false,
    };
    let value = crate::knowledge::commands::brief_for_ui(project, &args, format)?;
    match format {
        crate::cli::OutputFormat::Human | crate::cli::OutputFormat::Markdown => {
            Ok(serde_json::json!({
                "content": format!("{}\n", crate::render_brief_markdown(&value).trim_end()),
                "truncation": value["truncation"]
            }))
        }
        crate::cli::OutputFormat::Yaml => Ok(serde_json::json!({
            "yaml": format!("{}\n", crate::render_success(&value, format, true).trim_end()),
            "truncation": value["truncation"]
        })),
        crate::cli::OutputFormat::Json => Ok(value),
    }
}

pub fn events(root: &Path, prefix: &str, offset: usize, limit: usize) -> Result<Vec<Event>> {
    if limit > 500 {
        return Err(AppError::input("Event limit cannot exceed 500"));
    }
    let project = resolve_project(root, prefix)?;
    let (_lock, project) = lock_and_reload(project)?;
    Ok(storage::read_events(&project)?
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect())
}
