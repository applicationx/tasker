use crate::error::{AppError, ErrorCategory, Result};
use crate::knowledge::frontmatter;
use crate::knowledge::model::{ResourceKind, ResourceStatus, normalize_tags};
use crate::model::{AcceptanceCriterion, ContextKind, Task, TaskContextEntry, User};
use crate::storage::{self, Project};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::fs;
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskPatch {
    pub header: Option<String>,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "operation", deny_unknown_fields)]
pub enum UiMutation {
    #[serde(rename = "task.update")]
    TaskUpdate {
        actor: String,
        task_id: String,
        if_revision: u64,
        patch: TaskPatch,
    },
    #[serde(rename = "task.transition")]
    TaskTransition {
        actor: String,
        task_id: String,
        if_revision: u64,
        #[serde(alias = "target_state")]
        state: String,
    },
    #[serde(rename = "task.assign")]
    TaskAssign {
        actor: String,
        task_id: String,
        if_revision: u64,
        assignee: String,
    },
    #[serde(rename = "task.unassign")]
    TaskUnassign {
        actor: String,
        task_id: String,
        if_revision: u64,
    },
    #[serde(rename = "task.acceptance.add")]
    AcceptanceAdd {
        actor: String,
        task_id: String,
        if_revision: u64,
        text: String,
    },
    #[serde(rename = "task.acceptance.remove")]
    AcceptanceRemove {
        actor: String,
        task_id: String,
        if_revision: u64,
        #[serde(alias = "criterion_id")]
        acceptance_id: String,
    },
    #[serde(rename = "task.acceptance.set")]
    AcceptanceSet {
        actor: String,
        task_id: String,
        if_revision: u64,
        #[serde(alias = "criterion_id")]
        acceptance_id: String,
        completed: bool,
    },
    #[serde(rename = "task.context.append")]
    ContextAppend {
        actor: String,
        task_id: String,
        if_revision: u64,
        kind: ContextKind,
        message: String,
    },
    #[serde(rename = "research.update")]
    ResearchUpdate {
        actor: String,
        resource_id: String,
        if_revision: u64,
        patch: ResearchPatch,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResearchPatch {
    pub title: Option<String>,
    pub status: Option<ResourceStatus>,
    pub tags: Option<Vec<String>>,
    pub include_in_project_brief: Option<bool>,
    #[serde(alias = "content")]
    pub body: Option<String>,
}

#[derive(Debug, Clone)]
pub enum TaskMutation {
    Update(TaskPatch),
    Transition {
        state: String,
    },
    Assign {
        assignee: String,
    },
    Unassign,
    AcceptanceAdd {
        text: String,
    },
    AcceptanceRemove {
        acceptance_id: String,
    },
    AcceptanceSet {
        acceptance_id: String,
        completed: bool,
    },
    ContextAppend {
        kind: ContextKind,
        message: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "entity", rename_all = "snake_case")]
pub enum UiMutationResult {
    Task {
        task: Task,
    },
    Research {
        resource: crate::service::snapshot::ResourceView,
    },
}

pub(crate) fn mutate_task_record<A, F>(
    project: Project,
    id: &str,
    revision: Option<u64>,
    actor: A,
    operation: F,
) -> Result<Task>
where
    A: FnOnce(&Project) -> Result<User>,
    F: FnOnce(&Project, &User, &mut Task) -> Result<(&'static str, Value, bool)>,
{
    if !id.starts_with(&format!("{}-", project.config.prefix)) {
        return Err(AppError::new(
            "task_project_mismatch",
            format!(
                "Task {id} does not belong to project {}",
                project.config.prefix
            ),
            ErrorCategory::Validation,
        )
        .detail("task_id", id.to_string())
        .detail("project_prefix", project.config.prefix));
    }
    let _lock = storage::lock_project(&project.path)?;
    let project = storage::reload_project(&project)?;
    crate::transaction::recover(&project)?;
    let mut task = storage::read_task(&project, id)?;
    check_task_revision(&task, revision)?;
    let actor = actor(&project)?;
    let (action, changes, changed) = operation(&project, &actor, &mut task)?;
    if changed {
        task.revision += 1;
        task.updated_at = crate::app::now();
        task.updated_by = actor.id.clone();
        task.normalize();
        storage::write_task(&project, &task)?;
        crate::app::event(
            &project,
            &actor,
            action,
            Some(&task.id),
            Some(task.revision),
            changes,
        )?;
    }
    Ok(task)
}

fn check_task_revision(task: &Task, expected: Option<u64>) -> Result<()> {
    if let Some(expected) = expected
        && task.revision != expected
    {
        return Err(AppError::new(
            "revision_conflict",
            format!(
                "Task {} is at revision {}, not {}",
                task.id, task.revision, expected
            ),
            ErrorCategory::Conflict,
        )
        .detail("task_id", task.id.clone())
        .detail("expected_revision", expected)
        .detail("current_revision", task.revision));
    }
    Ok(())
}

fn check_revision(id: &str, current: u64, expected: Option<u64>) -> Result<()> {
    if let Some(expected) = expected
        && current != expected
    {
        return Err(AppError::new(
            "revision_conflict",
            format!("{id} is at revision {current}"),
            ErrorCategory::Conflict,
        )
        .detail("id", id.to_string())
        .detail("current_revision", current)
        .detail("expected_revision", expected));
    }
    Ok(())
}

fn existing_user(project: &Project, selector: &str) -> Result<User> {
    storage::read_users(project)?
        .into_iter()
        .find(|user| user.id == selector || user.name.eq_ignore_ascii_case(selector))
        .ok_or_else(|| {
            AppError::new(
                "user_not_found",
                format!("User {selector} does not exist; the UI only uses existing users"),
                ErrorCategory::NotFound,
            )
            .detail("user", selector.to_string())
        })
}

fn apply_task_mutation(
    project: Project,
    task_id: &str,
    actor_selector: &str,
    revision: u64,
    mutation: TaskMutation,
) -> Result<Task> {
    let actor_selector = actor_selector.to_string();
    mutate_task_record(
        project,
        task_id,
        Some(revision),
        move |project| existing_user(project, &actor_selector),
        move |project, actor, task| match mutation {
            TaskMutation::Update(patch) => update_task(task, patch),
            TaskMutation::Transition { state } => transition_task(project, task, state),
            TaskMutation::Assign { assignee } => {
                let user = existing_user(project, &assignee)?;
                let previous = task.assignee.clone();
                if previous.as_deref() == Some(user.id.as_str()) {
                    return Ok(("task.assigned", json!({}), false));
                }
                task.assignee = Some(user.id.clone());
                Ok((
                    "task.assigned",
                    json!({"assignee":{"from":previous,"to":user.id}}),
                    true,
                ))
            }
            TaskMutation::Unassign => {
                let previous = task.assignee.take();
                let changed = previous.is_some();
                Ok((
                    "task.unassigned",
                    json!({"assignee":{"from":previous,"to":null}}),
                    changed,
                ))
            }
            TaskMutation::AcceptanceAdd { text } => acceptance_add(task, text),
            TaskMutation::AcceptanceRemove { acceptance_id } => {
                acceptance_remove(task, &acceptance_id)
            }
            TaskMutation::AcceptanceSet {
                acceptance_id,
                completed,
            } => acceptance_set(task, actor, &acceptance_id, completed),
            TaskMutation::ContextAppend { kind, message } => {
                context_append(task, actor, kind, message)
            }
        },
    )
}

fn update_task(task: &mut Task, patch: TaskPatch) -> Result<(&'static str, Value, bool)> {
    if patch.header.is_none() && patch.description.is_none() && patch.tags.is_none() {
        return Err(AppError::input("No mutable task fields were supplied"));
    }
    let mut changes = Map::new();
    if let Some(header) = patch.header {
        let header = header.trim().to_string();
        if header.is_empty() {
            return Err(AppError::input("Task header cannot be empty"));
        }
        if task.header != header {
            changes.insert("header".into(), json!({"from":task.header,"to":header}));
            task.header = header;
        }
    }
    if let Some(description) = patch.description
        && task.description != description
    {
        changes.insert(
            "description".into(),
            json!({"from":task.description,"to":description}),
        );
        task.description = description;
    }
    if let Some(tags) = patch.tags {
        let previous = task.tags.clone();
        task.tags = tags;
        task.normalize();
        if previous != task.tags {
            changes.insert("tags".into(), json!({"from":previous,"to":task.tags}));
        }
    }
    let changed = !changes.is_empty();
    Ok(("task.updated", Value::Object(changes), changed))
}

fn transition_task(
    project: &Project,
    task: &mut Task,
    state: String,
) -> Result<(&'static str, Value, bool)> {
    crate::app::verify_transition(&project.config, &task.state, &state)?;
    let remaining: Vec<String> = task
        .acceptance_criteria
        .iter()
        .filter(|criterion| !criterion.completed)
        .map(|criterion| criterion.id.clone())
        .collect();
    if !remaining.is_empty()
        && project
            .config
            .workflow
            .states
            .get(&state)
            .is_some_and(|config| config.dependency_satisfied)
    {
        return Err(AppError::new(
            "acceptance_criteria_incomplete",
            "All acceptance criteria must be completed before entering a dependency-satisfying state",
            ErrorCategory::Validation,
        )
        .detail("remaining_acceptance_criteria", json!(remaining)));
    }
    let previous = std::mem::replace(&mut task.state, state.clone());
    Ok((
        "task.transitioned",
        json!({"state":{"from":previous,"to":state}}),
        true,
    ))
}

fn acceptance_add(task: &mut Task, text: String) -> Result<(&'static str, Value, bool)> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err(AppError::input("Acceptance criterion cannot be empty"));
    }
    if task
        .acceptance_criteria
        .iter()
        .any(|criterion| criterion.text.eq_ignore_ascii_case(&text))
    {
        return Err(AppError::new(
            "acceptance_criterion_exists",
            "An equivalent acceptance criterion already exists",
            ErrorCategory::Conflict,
        ));
    }
    let maximum = task
        .acceptance_criteria
        .iter()
        .filter_map(|criterion| criterion.id.strip_prefix("AC-")?.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    let next = task.next_acceptance_number.max(maximum + 1);
    task.next_acceptance_number = next + 1;
    let criterion = AcceptanceCriterion {
        id: format!("AC-{next}"),
        text,
        completed: false,
        completed_at: None,
        completed_by: None,
    };
    task.acceptance_criteria.push(criterion.clone());
    Ok((
        "task.acceptance_added",
        json!({"acceptance_criterion":{"from":null,"to":criterion}}),
        true,
    ))
}

fn acceptance_remove(task: &mut Task, acceptance_id: &str) -> Result<(&'static str, Value, bool)> {
    let id = acceptance_id.to_ascii_uppercase();
    let position = task
        .acceptance_criteria
        .iter()
        .position(|criterion| criterion.id == id)
        .ok_or_else(|| {
            AppError::new(
                "acceptance_criterion_not_found",
                format!("Acceptance criterion {id} does not exist"),
                ErrorCategory::NotFound,
            )
        })?;
    let removed = task.acceptance_criteria.remove(position);
    Ok((
        "task.acceptance_removed",
        json!({"acceptance_criterion":{"from":removed,"to":null}}),
        true,
    ))
}

fn acceptance_set(
    task: &mut Task,
    actor: &User,
    acceptance_id: &str,
    completed: bool,
) -> Result<(&'static str, Value, bool)> {
    let id = acceptance_id.to_ascii_uppercase();
    let criterion = task
        .acceptance_criteria
        .iter_mut()
        .find(|criterion| criterion.id == id)
        .ok_or_else(|| {
            AppError::new(
                "acceptance_criterion_not_found",
                format!("Acceptance criterion {id} does not exist"),
                ErrorCategory::NotFound,
            )
        })?;
    let action = if completed {
        "task.acceptance_completed"
    } else {
        "task.acceptance_reopened"
    };
    if criterion.completed == completed {
        return Ok((action, json!({}), false));
    }
    let previous = criterion.clone();
    criterion.completed = completed;
    criterion.completed_at = completed.then(crate::app::now);
    criterion.completed_by = completed.then(|| actor.id.clone());
    Ok((
        action,
        json!({"acceptance_criterion":{"from":previous,"to":criterion}}),
        true,
    ))
}

fn context_append(
    task: &mut Task,
    actor: &User,
    kind: ContextKind,
    message: String,
) -> Result<(&'static str, Value, bool)> {
    let message = message.trim().to_string();
    if message.is_empty() {
        return Err(AppError::input("Context message cannot be empty"));
    }
    let next = task
        .context
        .iter()
        .filter_map(|entry| entry.id.strip_prefix("CTX-")?.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    let entry = TaskContextEntry {
        id: format!("CTX-{next}"),
        kind,
        message,
        created_at: crate::app::now(),
        created_by: actor.id.clone(),
    };
    task.context.push(entry.clone());
    Ok((
        "task.context_added",
        json!({"context":{"from":null,"to":entry}}),
        true,
    ))
}

pub fn apply_mutation(root: &Path, prefix: &str, mutation: UiMutation) -> Result<UiMutationResult> {
    let project = crate::service::snapshot::resolve_project(root, prefix)?;
    match mutation {
        UiMutation::TaskUpdate {
            actor,
            task_id,
            if_revision,
            patch,
        } => task_result(
            project,
            &task_id,
            &actor,
            if_revision,
            TaskMutation::Update(patch),
        ),
        UiMutation::TaskTransition {
            actor,
            task_id,
            if_revision,
            state,
        } => task_result(
            project,
            &task_id,
            &actor,
            if_revision,
            TaskMutation::Transition { state },
        ),
        UiMutation::TaskAssign {
            actor,
            task_id,
            if_revision,
            assignee,
        } => task_result(
            project,
            &task_id,
            &actor,
            if_revision,
            TaskMutation::Assign { assignee },
        ),
        UiMutation::TaskUnassign {
            actor,
            task_id,
            if_revision,
        } => task_result(
            project,
            &task_id,
            &actor,
            if_revision,
            TaskMutation::Unassign,
        ),
        UiMutation::AcceptanceAdd {
            actor,
            task_id,
            if_revision,
            text,
        } => task_result(
            project,
            &task_id,
            &actor,
            if_revision,
            TaskMutation::AcceptanceAdd { text },
        ),
        UiMutation::AcceptanceRemove {
            actor,
            task_id,
            if_revision,
            acceptance_id,
        } => task_result(
            project,
            &task_id,
            &actor,
            if_revision,
            TaskMutation::AcceptanceRemove { acceptance_id },
        ),
        UiMutation::AcceptanceSet {
            actor,
            task_id,
            if_revision,
            acceptance_id,
            completed,
        } => task_result(
            project,
            &task_id,
            &actor,
            if_revision,
            TaskMutation::AcceptanceSet {
                acceptance_id,
                completed,
            },
        ),
        UiMutation::ContextAppend {
            actor,
            task_id,
            if_revision,
            kind,
            message,
        } => task_result(
            project,
            &task_id,
            &actor,
            if_revision,
            TaskMutation::ContextAppend { kind, message },
        ),
        UiMutation::ResearchUpdate {
            actor,
            resource_id,
            if_revision,
            patch,
        } => update_research(project, &resource_id, &actor, if_revision, patch),
    }
}

fn task_result(
    project: Project,
    task_id: &str,
    actor: &str,
    revision: u64,
    mutation: TaskMutation,
) -> Result<UiMutationResult> {
    Ok(UiMutationResult::Task {
        task: apply_task_mutation(project, task_id, actor, revision, mutation)?,
    })
}

fn update_research(
    project: Project,
    resource_id: &str,
    actor_selector: &str,
    revision: u64,
    patch: ResearchPatch,
) -> Result<UiMutationResult> {
    if patch.title.is_none()
        && patch.status.is_none()
        && patch.tags.is_none()
        && patch.include_in_project_brief.is_none()
        && patch.body.is_none()
    {
        return Err(AppError::input("No mutable research fields were supplied"));
    }
    if patch
        .body
        .as_ref()
        .is_some_and(|body| body.len() > 1024 * 1024)
    {
        return Err(AppError::input("Managed research body cannot exceed 1 MiB"));
    }
    if patch.status == Some(ResourceStatus::Archived) {
        return Err(AppError::new(
            "research_archive_read_only",
            "The UI cannot archive research resources",
            ErrorCategory::Validation,
        ));
    }
    let _lock = storage::lock_project(&project.path)?;
    let project = storage::reload_project(&project)?;
    crate::transaction::recover(&project)?;
    let id = crate::knowledge::canonical_id(&project, resource_id, 'R')?;
    let mut document = crate::knowledge::read_resources(&project)?
        .into_iter()
        .find(|document| document.metadata.id == id)
        .ok_or_else(|| {
            AppError::new(
                "resource_not_found",
                format!("Resource {id} does not exist"),
                ErrorCategory::NotFound,
            )
        })?;
    check_revision(&id, document.metadata.revision, Some(revision))?;
    if document.metadata.kind != ResourceKind::Research
        || document.metadata.source_path.is_some()
        || document.metadata.status == ResourceStatus::Archived
    {
        return Err(AppError::new(
            "research_not_editable",
            "Only existing managed, non-archived research resources are editable in the UI",
            ErrorCategory::Validation,
        )
        .detail("resource_id", id));
    }
    let actor = existing_user(&project, actor_selector)?;
    let before = fs::read(&document.path)
        .map_err(|error| AppError::io(error, "cannot read research resource before update"))?;
    let before_metadata = document.metadata.clone();
    if let Some(title) = patch.title {
        let title = title.trim().to_string();
        if title.is_empty() {
            return Err(AppError::input("Research title cannot be empty"));
        }
        document.metadata.title = title;
    }
    if let Some(status) = patch.status {
        document.metadata.status = status;
    }
    if let Some(mut tags) = patch.tags {
        normalize_tags(&mut tags);
        document.metadata.tags = tags;
    }
    if let Some(include) = patch.include_in_project_brief {
        document.metadata.include_in_project_brief = include;
    }
    if let Some(body) = patch.body {
        document.body = frontmatter::normalized_body(&body);
    }
    let comparison = frontmatter::serialize(&document.metadata, &document.body)?;
    if comparison == before {
        return Ok(UiMutationResult::Research {
            resource: crate::service::snapshot::ResourceView {
                metadata: document.metadata,
                source_type: crate::service::snapshot::ResourceSourceType::Managed,
                content: document.body,
            },
        });
    }
    document.metadata.revision += 1;
    document.metadata.updated_at = crate::app::now();
    document.metadata.updated_by = actor.id.clone();
    let after = frontmatter::serialize(&document.metadata, &document.body)?;
    crate::knowledge::validate_managed_file(&project, "resources", &document.path, false)?;
    storage::atomic_write(&document.path, &after)?;
    let transaction_id = Uuid::new_v4().to_string();
    if let Err(error) = crate::app::event(
        &project,
        &actor,
        "resource.updated",
        None,
        None,
        json!({
            "resource_id":id,
            "path":document.path.strip_prefix(&project.path).unwrap_or(&document.path).to_string_lossy().replace('\\', "/"),
            "revision":document.metadata.revision,
            "metadata":{"from":before_metadata,"to":document.metadata},
            "before_sha256":crate::knowledge::sha256(&before),
            "after_sha256":crate::knowledge::sha256(&after),
            "before_bytes":before.len(),
            "after_bytes":after.len(),
            "transaction_id":transaction_id
        }),
    ) {
        crate::knowledge::validate_managed_file(&project, "resources", &document.path, false)?;
        storage::atomic_write(&document.path, &before)?;
        return Err(error);
    }
    Ok(UiMutationResult::Research {
        resource: crate::service::snapshot::ResourceView {
            metadata: document.metadata,
            source_type: crate::service::snapshot::ResourceSourceType::Managed,
            content: document.body,
        },
    })
}
