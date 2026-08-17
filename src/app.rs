use crate::cli::*;
use crate::error::{AppError, ErrorCategory, Result};
use crate::knowledge::model::{DecisionMeta, DecisionStatus};
use crate::model::*;
use crate::storage::{self, Project};
use chrono::{SecondsFormat, Utc};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs;

pub fn execute(cli: &Cli) -> Result<Value> {
    match &cli.command {
        Command::Ui { command } => crate::ui::command(cli, command),
        Command::Config { command } => config_command(command),
        Command::Create { command } => create_command(cli, command),
        Command::Project { command } => crate::knowledge::commands::project_command(cli, command),
        Command::Ensure { command } => ensure_command(cli, command),
        Command::List { command } => list_command(cli, command),
        Command::Get { command } => get_command(cli, command),
        Command::Update { command } => update_command(cli, command),
        Command::Archive { command } => match command {
            ArchiveCommand::Resource(args) => {
                crate::knowledge::commands::archive_resource(cli, args)
            }
        },
        Command::Decision { command } => {
            crate::knowledge::commands::decision_lifecycle(cli, command)
        }
        Command::ContextRef { command } => crate::knowledge::commands::context_ref(cli, command),
        Command::Brief(args) => crate::knowledge::commands::brief(cli, args),
        Command::Status(args) => status_command(cli, &args.task),
        Command::Assign(args) => assign_command(cli, args),
        Command::Unassign(args) => unassign_command(cli, args),
        Command::Transition(args) => transition_command(cli, args),
        Command::Claim(args) => claim_command(cli, args),
        Command::Next(args) => next_command(cli, args),
        Command::Dependency { command } => dependency_command(cli, command),
        Command::Acceptance { command } => acceptance_command(cli, command),
        Command::Context { command } => context_command(cli, command),
        Command::Relation { command } => relation_command(cli, command),
        Command::Search { command } => search_command(cli, command),
        Command::Changelog(args) => changelog_command(cli, args),
        Command::Validate => validate_command(cli),
    }
}

fn config_command(command: &ConfigCommand) -> Result<Value> {
    match command {
        ConfigCommand::Show => Ok(storage::effective_config_value()),
        ConfigCommand::Get { key } => {
            let config = storage::effective_config_value();
            let canonical = key.replace('-', "_");
            config
                .get(&canonical)
                .cloned()
                .ok_or_else(|| AppError::input(format!("Unknown configuration key {key}")))
        }
        ConfigCommand::Set { key, value } => {
            let mut config = storage::load_global_config();
            match key.as_str() {
                "projects-root" | "projects_root" => {
                    config.projects_root =
                        Some(storage::expand_path(value).to_string_lossy().to_string());
                }
                "default-output" | "default_output" => {
                    let _: OutputFormat = value.parse().map_err(|_| {
                        AppError::input("default-output must be human, markdown, json, or yaml")
                    })?;
                    config.default_output = Some(value.to_ascii_lowercase());
                }
                _ => return Err(AppError::input(format!("Unknown configuration key {key}"))),
            }
            storage::save_global_config(&config)?;
            Ok(storage::effective_config_value())
        }
    }
}

fn create_command(cli: &Cli, command: &CreateCommand) -> Result<Value> {
    match command {
        CreateCommand::Project(args) => create_project(cli, args),
        CreateCommand::Task(args) => create_task(cli, args),
        CreateCommand::User(args) => create_user_command(cli, args, false),
        CreateCommand::Resource(args) => crate::knowledge::commands::create_resource(cli, args),
        CreateCommand::Decision(args) => crate::knowledge::commands::create_decision(cli, args),
    }
}

fn ensure_command(cli: &Cli, command: &EnsureCommand) -> Result<Value> {
    match command {
        EnsureCommand::User(args) => create_user_command(cli, args, true),
    }
}

fn list_command(cli: &Cli, command: &ListCommand) -> Result<Value> {
    match command {
        ListCommand::Projects { stats } => list_projects(*stats),
        ListCommand::Tasks(filters) => {
            let project = project_for(cli, None)?;
            let (_lock, project) = lock_and_recover(project)?;
            let tasks = storage::read_tasks(&project)?;
            tasks_output(&project, tasks, filters, None)
        }
        ListCommand::Users => {
            let project = project_for(cli, None)?;
            to_value(storage::read_users(&project)?)
        }
        ListCommand::Resources(args) => crate::knowledge::commands::list_resources(cli, args),
        ListCommand::Decisions(args) => crate::knowledge::commands::list_decisions(cli, args),
    }
}

fn get_command(cli: &Cli, command: &GetCommand) -> Result<Value> {
    match command {
        GetCommand::Project { project } => {
            let project = storage::resolve_selector(project)?;
            let _lock = storage::lock_project(&project.path)?;
            let project = storage::reload_project(&project)?;
            crate::transaction::recover(&project)?;
            project_value(&project)
        }
        GetCommand::Task { task } => {
            let project = project_for(cli, Some(task))?;
            ensure_task_id(&project, task)?;
            let _lock = storage::lock_project(&project.path)?;
            let project = storage::reload_project(&project)?;
            crate::transaction::recover(&project)?;
            to_value(storage::read_task(&project, task)?)
        }
        GetCommand::User { user } => {
            let project = project_for(cli, None)?;
            to_value(resolve_user(&project, user)?)
        }
        GetCommand::Resource { resource } => {
            crate::knowledge::commands::get_resource(cli, resource)
        }
        GetCommand::Decision { decision } => {
            crate::knowledge::commands::get_decision(cli, decision)
        }
    }
}

fn update_command(cli: &Cli, command: &UpdateCommand) -> Result<Value> {
    match command {
        UpdateCommand::Task(args) => update_task(cli, args),
        UpdateCommand::User(args) => update_user(cli, args),
        UpdateCommand::Resource(args) => crate::knowledge::commands::update_resource(cli, args),
        UpdateCommand::Decision(args) => crate::knowledge::commands::update_decision(cli, args),
    }
}

fn project_for(cli: &Cli, task_id: Option<&str>) -> Result<Project> {
    storage::resolve_project(cli.project.as_deref(), task_id)
}

fn lock_and_recover(project: Project) -> Result<(storage::ProjectLock, Project)> {
    let lock = storage::lock_project(&project.path)?;
    let project = storage::reload_project(&project)?;
    crate::transaction::recover(&project)?;
    Ok((lock, project))
}

pub(crate) fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn to_value<T: serde::Serialize>(value: T) -> Result<Value> {
    serde_json::to_value(value)
        .map_err(|e| AppError::input(format!("cannot serialize result: {e}")))
}

fn slug(value: &str) -> String {
    let mut result = String::new();
    let mut separator = false;
    for character in value.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            if separator && !result.is_empty() {
                result.push('-');
            }
            result.push(character);
            separator = false;
        } else {
            separator = true;
        }
    }
    if result.is_empty() {
        "user".to_string()
    } else {
        result
    }
}

fn valid_prefix(prefix: &str) -> bool {
    let mut chars = prefix.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_uppercase())
        && (2..=10).contains(&prefix.len())
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

fn create_project(cli: &Cli, args: &CreateProjectArgs) -> Result<Value> {
    if args.name.trim().is_empty() {
        return Err(AppError::input("Project name cannot be empty"));
    }
    if !valid_prefix(&args.prefix) {
        return Err(AppError::input("Prefix must match ^[A-Z][A-Z0-9]{1,9}$")
            .detail("prefix", args.prefix.clone()));
    }
    let _root_lock = storage::lock_projects_root()?;
    if storage::discover_projects()?
        .iter()
        .any(|project| project.config.prefix.eq_ignore_ascii_case(&args.prefix))
    {
        return Err(AppError::new(
            "project_prefix_exists",
            format!("Project prefix {} already exists", args.prefix),
            ErrorCategory::Conflict,
        )
        .detail("prefix", args.prefix.clone()));
    }
    let path = args
        .path
        .as_deref()
        .map(storage::expand_path)
        .unwrap_or_else(|| storage::projects_root().join(slug(&args.name)));
    if path.exists()
        && fs::read_dir(&path)
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(true)
    {
        return Err(AppError::new(
            "project_path_exists",
            format!("Project path {} is not empty", path.display()),
            ErrorCategory::Conflict,
        )
        .detail("path", path.to_string_lossy().to_string()));
    }
    for subdir in ["tasks", "users", "changelog", "resources", "decisions"] {
        fs::create_dir_all(path.join(subdir)).map_err(|e| {
            AppError::io(
                e,
                format!("cannot create project directory {}", path.display()),
            )
        })?;
    }
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path.join(".tasker.lock"))
        .map_err(|e| AppError::io(e, "cannot create project lock file"))?;
    let config = default_project(args.name.trim().to_string(), args.prefix.clone());
    let project = Project {
        path: fs::canonicalize(&path).unwrap_or(path),
        config,
    };
    let _lock = storage::lock_project(&project.path)?;
    storage::write_yaml(&project.path.join("tasker.yaml"), &project.config)?;
    storage::write_meta(
        &project,
        &Meta {
            schema_version: SCHEMA_VERSION,
            next_task_number: 1,
            next_resource_number: 1,
            next_decision_number: 1,
        },
    )?;
    storage::atomic_write(
        &project.path.join(crate::knowledge::PROJECT_BRIEF_PATH),
        crate::knowledge::project_template(&project.config.name).as_bytes(),
    )?;
    let actor = actor_locked(cli, &project, None)?;
    event(
        &project,
        &actor,
        "project.created",
        None,
        None,
        json!({
            "name": {"from": null, "to": project.config.name},
            "prefix": {"from": null, "to": project.config.prefix}
        }),
    )?;
    project_value(&project)
}

fn project_value(project: &Project) -> Result<Value> {
    let mut value = json!({
        "name": project.config.name,
        "prefix": project.config.prefix,
        "path": project.path.to_string_lossy(),
        "workflow": project.config.workflow,
        "relations": project.config.relations,
        "config_file": project.path.join("tasker.yaml").to_string_lossy(),
    });
    let summary = crate::knowledge::commands::project_summary(project)?;
    value
        .as_object_mut()
        .unwrap()
        .extend(summary.as_object().unwrap().clone());
    Ok(value)
}

fn list_projects(stats: bool) -> Result<Value> {
    let projects = storage::discover_projects()?;
    if !stats {
        return to_value(
            projects
                .into_iter()
                .map(|p| ProjectSummary {
                    name: p.config.name,
                    prefix: p.config.prefix,
                    path: p.path.to_string_lossy().to_string(),
                })
                .collect::<Vec<_>>(),
        );
    }
    let mut values = Vec::new();
    for project in projects {
        let (_lock, project) = lock_and_recover(project)?;
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for task in storage::read_tasks(&project)? {
            *counts.entry(task.state).or_default() += 1;
        }
        values.push(
            json!({"name": project.config.name, "prefix": project.config.prefix,
            "path": project.path.to_string_lossy(), "stats": counts}),
        );
    }
    Ok(Value::Array(values))
}

fn actor_name(cli: &Cli, command_actor: Option<&str>) -> String {
    cli.actor
        .clone()
        .or_else(|| std::env::var("TASKER_ACTOR").ok())
        .or_else(|| command_actor.map(str::to_string))
        .or_else(|| std::env::var("USERNAME").ok())
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "unknown".into())
}

pub(crate) fn actor_locked(
    cli: &Cli,
    project: &Project,
    command_actor: Option<&str>,
) -> Result<User> {
    let name = actor_name(cli, command_actor);
    let (user, created) = ensure_user_locked(project, &name, UserKind::Unknown)?;
    if created {
        event(
            project,
            &user,
            "user.created",
            None,
            None,
            json!({"user": {"from": null, "to": user}}),
        )?;
    }
    Ok(user)
}

fn resolve_user(project: &Project, selector: &str) -> Result<User> {
    let users = storage::read_users(project)?;
    users
        .into_iter()
        .find(|user| user.id == selector || user.name.eq_ignore_ascii_case(selector))
        .ok_or_else(|| {
            AppError::new(
                "user_not_found",
                format!("User {selector} does not exist"),
                ErrorCategory::NotFound,
            )
            .detail("user", selector.to_string())
        })
}

fn ensure_user_locked(project: &Project, name: &str, kind: UserKind) -> Result<(User, bool)> {
    if name.trim().is_empty() {
        return Err(AppError::input("User name cannot be empty"));
    }
    let users = storage::read_users(project)?;
    if let Some(user) = users
        .iter()
        .find(|user| user.name.eq_ignore_ascii_case(name) || user.id == name)
    {
        return Ok((user.clone(), false));
    }
    let base = slug(name);
    let ids: HashSet<&str> = users.iter().map(|user| user.id.as_str()).collect();
    let mut id = base.clone();
    let mut suffix = 2;
    while ids.contains(id.as_str()) {
        id = format!("{base}-{suffix}");
        suffix += 1;
    }
    let user = User {
        schema_version: SCHEMA_VERSION,
        id,
        name: name.trim().to_string(),
        kind,
        created_at: now(),
    };
    storage::write_user(project, &user)?;
    Ok((user, true))
}

fn create_user_command(cli: &Cli, args: &CreateUserArgs, ensure: bool) -> Result<Value> {
    let project = project_for(cli, None)?;
    let _lock = storage::lock_project(&project.path)?;
    let project = storage::reload_project(&project)?;
    let existing = storage::read_users(&project)?
        .into_iter()
        .find(|u| u.name.eq_ignore_ascii_case(&args.name));
    if let Some(user) = existing {
        if ensure {
            return to_value(user);
        }
        return Err(AppError::new(
            "user_name_exists",
            format!("User name {} already exists", args.name),
            ErrorCategory::Conflict,
        )
        .detail("user", args.name.clone()));
    }
    let requested_actor = actor_name(cli, None);
    if requested_actor.eq_ignore_ascii_case(&args.name) {
        let (user, _) = ensure_user_locked(&project, &args.name, args.kind)?;
        event(
            &project,
            &user,
            "user.created",
            None,
            None,
            json!({"user": {"from": null, "to": user}}),
        )?;
        return to_value(user);
    }
    let actor = actor_locked(cli, &project, None)?;
    let (user, _) = ensure_user_locked(&project, &args.name, args.kind)?;
    event(
        &project,
        &actor,
        "user.created",
        None,
        None,
        json!({"user": {"from": null, "to": user}}),
    )?;
    to_value(user)
}

fn update_user(cli: &Cli, args: &UpdateUserArgs) -> Result<Value> {
    if args.name.trim().is_empty() {
        return Err(AppError::input("User name cannot be empty"));
    }
    let project = project_for(cli, None)?;
    let _lock = storage::lock_project(&project.path)?;
    let project = storage::reload_project(&project)?;
    let mut user = resolve_user(&project, &args.user)?;
    if storage::read_users(&project)?
        .iter()
        .any(|other| other.id != user.id && other.name.eq_ignore_ascii_case(&args.name))
    {
        return Err(AppError::new(
            "user_name_exists",
            format!("User name {} already exists", args.name),
            ErrorCategory::Conflict,
        ));
    }
    let actor = actor_locked(cli, &project, None)?;
    let old = user.name.clone();
    user.name = args.name.trim().to_string();
    storage::write_user(&project, &user)?;
    event(
        &project,
        &actor,
        "user.updated",
        None,
        None,
        json!({"name": {"from": old, "to": user.name}}),
    )?;
    to_value(user)
}

#[derive(Default)]
struct TaskInput {
    header: Option<String>,
    description: Option<String>,
    tags: Option<Vec<String>>,
    acceptance_criteria: Option<Vec<String>>,
    context_refs: Option<Vec<ContextReference>>,
}

fn structured_task_input(
    input: InputFormat,
    file: Option<&str>,
    protected: bool,
) -> Result<TaskInput> {
    if matches!(input, InputFormat::Human) {
        return Ok(TaskInput::default());
    }
    let text = storage::read_input(file)?;
    let value: Value = match input {
        InputFormat::Json => serde_json::from_str(&text)
            .map_err(|e| AppError::input(format!("Invalid JSON input: {e}")))?,
        InputFormat::Yaml => serde_yaml_ng::from_str(&text)
            .map_err(|e| AppError::input(format!("Invalid YAML input: {e}")))?,
        InputFormat::Human => unreachable!(),
    };
    let object = value
        .as_object()
        .ok_or_else(|| AppError::input("Task input must be an object"))?;
    let allowed = if protected {
        &["header", "description", "tags"][..]
    } else {
        &[
            "header",
            "description",
            "tags",
            "acceptance_criteria",
            "context_refs",
        ][..]
    };
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            let code = if protected {
                "protected_task_field"
            } else {
                "invalid_input"
            };
            return Err(AppError::new(
                code,
                format!("Task field {key} cannot be set by this operation"),
                ErrorCategory::Input,
            )
            .detail("field", key.clone()));
        }
    }
    let header = optional_string(object, "header")?;
    let description = optional_string(object, "description")?;
    let tags = match object.get("tags") {
        None => None,
        Some(value) => Some(
            serde_json::from_value(value.clone())
                .map_err(|e| AppError::input(format!("tags must be an array of strings: {e}")))?,
        ),
    };
    let acceptance_criteria = match object.get("acceptance_criteria") {
        None => None,
        Some(value) => Some(serde_json::from_value(value.clone()).map_err(|e| {
            AppError::input(format!(
                "acceptance_criteria must be an array of strings: {e}"
            ))
        })?),
    };
    let context_refs = match object.get("context_refs") {
        None => None,
        Some(value) => Some(serde_json::from_value(value.clone()).map_err(|error| {
            AppError::input(format!(
                "context_refs must be an array of context references: {error}"
            ))
        })?),
    };
    Ok(TaskInput {
        header,
        description,
        tags,
        acceptance_criteria,
        context_refs,
    })
}

fn optional_string(object: &Map<String, Value>, key: &str) -> Result<Option<String>> {
    object
        .get(key)
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| AppError::input(format!("{key} must be a string")))
        })
        .transpose()
}

fn description_value(direct: &Option<String>, file: &Option<String>) -> Result<Option<String>> {
    if let Some(path) = file {
        return fs::read_to_string(storage::expand_path(path))
            .map(Some)
            .map_err(|e| AppError::io(e, format!("cannot read description file {path}")));
    }
    Ok(direct.clone())
}

fn create_task(cli: &Cli, args: &CreateTaskArgs) -> Result<Value> {
    let mut input = structured_task_input(
        args.input_args.input,
        args.input_args.file.as_deref(),
        false,
    )?;
    if args.header.is_some() {
        input.header = args.header.clone();
    }
    if let Some(description) = description_value(&args.description, &args.description_file)? {
        input.description = Some(description);
    }
    if !args.tags.is_empty() {
        input.tags = Some(args.tags.clone());
    }
    if !args.acceptance_criteria.is_empty() {
        input.acceptance_criteria = Some(args.acceptance_criteria.clone());
    }
    let header = input
        .header
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AppError::input("Task header is required"))?;
    let project = project_for(cli, None)?;
    let _lock = storage::lock_project(&project.path)?;
    let project = storage::reload_project(&project)?;
    crate::transaction::recover(&project)?;
    validate_project_config(&project.config)?;
    let mut context_refs = if args.context_refs.is_empty() && args.from.is_empty() {
        input.context_refs.unwrap_or_default()
    } else {
        Vec::new()
    };
    for value in &args.context_refs {
        context_refs.push(crate::knowledge::commands::parse_context_reference(
            &project, value, None,
        )?);
    }
    for value in &args.from {
        context_refs.push(crate::knowledge::commands::parse_context_reference(
            &project,
            value,
            Some(ContextReferenceRole::InformedBy),
        )?);
    }
    crate::knowledge::commands::validate_context_references(&project, &mut context_refs)?;
    let actor = actor_locked(cli, &project, None)?;
    let mut meta = storage::read_meta(&project)?;
    let existing_tasks = storage::read_tasks(&project)?;
    let max_task_number = existing_tasks
        .iter()
        .filter_map(|task| task.id.rsplit_once('-')?.1.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    if meta.schema_version != SCHEMA_VERSION
        || meta.next_task_number == 0
        || meta.next_task_number <= max_task_number
    {
        return Err(AppError::project(
            "meta.json next_task_number must exceed all allocated task IDs",
        )
        .detail("next_task_number", meta.next_task_number)
        .detail("max_task_number", max_task_number));
    }
    let id = format!("{}-{}", project.config.prefix, meta.next_task_number);
    if project
        .path
        .join("tasks")
        .join(format!("{id}.json"))
        .exists()
    {
        return Err(AppError::new(
            "task_id_exists",
            format!("Task {id} already exists; refusing to reuse an allocated ID"),
            ErrorCategory::Conflict,
        )
        .detail("task_id", id));
    }
    meta.next_task_number += 1;
    storage::write_meta(&project, &meta)?;
    let timestamp = now();
    let mut task = Task {
        schema_version: SCHEMA_VERSION,
        id,
        header: header.trim().to_string(),
        description: input.description.unwrap_or_default(),
        acceptance_criteria: {
            let values = input.acceptance_criteria.unwrap_or_default();
            values
                .into_iter()
                .enumerate()
                .map(|(index, text)| AcceptanceCriterion {
                    id: format!("AC-{}", index + 1),
                    text,
                    completed: false,
                    completed_at: None,
                    completed_by: None,
                })
                .collect()
        },
        next_acceptance_number: 1,
        context: vec![],
        context_refs,
        state: project.config.workflow.initial_state.clone(),
        assignee: None,
        tags: input.tags.unwrap_or_default(),
        dependencies: vec![],
        relations: vec![],
        created_at: timestamp.clone(),
        created_by: actor.id.clone(),
        updated_at: timestamp,
        updated_by: actor.id.clone(),
        revision: 1,
    };
    task.next_acceptance_number = task.acceptance_criteria.len() as u64 + 1;
    task.normalize();
    storage::write_task(&project, &task)?;
    event(
        &project,
        &actor,
        "task.created",
        Some(&task.id),
        Some(task.revision),
        json!({"task": {"from": null, "to": task}}),
    )?;
    to_value(task)
}

fn update_task(cli: &Cli, args: &UpdateTaskArgs) -> Result<Value> {
    let mut patch =
        structured_task_input(args.input_args.input, args.input_args.file.as_deref(), true)?;
    if args.header.is_some() {
        patch.header = args.header.clone();
    }
    if let Some(description) = description_value(&args.description, &args.description_file)? {
        patch.description = Some(description);
    }
    if !args.tags.is_empty() || args.clear_tags {
        patch.tags = Some(args.tags.clone());
    }
    if patch.header.is_none() && patch.description.is_none() && patch.tags.is_none() {
        return Err(AppError::input("No mutable task fields were supplied"));
    }
    mutate_task(
        cli,
        &args.task,
        args.if_revision,
        None,
        |project, actor, task| {
            let mut changes = Map::new();
            if let Some(header) = patch.header {
                if header.trim().is_empty() {
                    return Err(AppError::input("Task header cannot be empty"));
                }
                if task.header != header {
                    changes.insert("header".into(), json!({"from": task.header, "to": header}));
                    task.header = header;
                }
            }
            if let Some(description) = patch.description
                && task.description != description
            {
                changes.insert(
                    "description".into(),
                    json!({"from": task.description, "to": description}),
                );
                task.description = description;
            }
            if let Some(tags) = patch.tags {
                let old = task.tags.clone();
                task.tags = tags;
                task.normalize();
                if old != task.tags {
                    changes.insert("tags".into(), json!({"from": old, "to": task.tags}));
                }
            }
            if changes.is_empty() {
                return Ok(("task.updated", Value::Object(changes), false));
            }
            let _ = (project, actor);
            Ok(("task.updated", Value::Object(changes), true))
        },
    )
}

fn check_revision(task: &Task, expected: Option<u64>) -> Result<()> {
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

fn mutate_task<F>(
    cli: &Cli,
    id: &str,
    revision: Option<u64>,
    command_actor: Option<&str>,
    operation: F,
) -> Result<Value>
where
    F: FnOnce(&Project, &User, &mut Task) -> Result<(&'static str, Value, bool)>,
{
    let project = project_for(cli, Some(id))?;
    let task = crate::service::mutation::mutate_task_record(
        project,
        id,
        revision,
        |project| actor_locked(cli, project, command_actor),
        operation,
    )?;
    to_value(task)
}

fn ensure_task_id(project: &Project, id: &str) -> Result<()> {
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
        .detail("project_prefix", project.config.prefix.clone()));
    }
    Ok(())
}

fn assign_command(cli: &Cli, args: &AssignArgs) -> Result<Value> {
    mutate_task(
        cli,
        &args.task,
        args.if_revision,
        None,
        |project, actor, task| {
            let (user, created) = ensure_user_locked(project, &args.user, UserKind::Unknown)?;
            if created {
                event(
                    project,
                    actor,
                    "user.created",
                    None,
                    None,
                    json!({"user": {"from": null, "to": user}}),
                )?;
            }
            let old = task.assignee.clone();
            if old.as_deref() == Some(&user.id) {
                return Ok(("task.assigned", json!({}), false));
            }
            task.assignee = Some(user.id.clone());
            Ok((
                "task.assigned",
                json!({"assignee": {"from": old, "to": user.id}}),
                true,
            ))
        },
    )
}

fn unassign_command(cli: &Cli, args: &MutateTaskArgs) -> Result<Value> {
    mutate_task(cli, &args.task, args.if_revision, None, |_, _, task| {
        let old = task.assignee.take();
        let changed = old.is_some();
        Ok((
            "task.unassigned",
            json!({"assignee": {"from": old, "to": null}}),
            changed,
        ))
    })
}

fn transition_command(cli: &Cli, args: &TransitionArgs) -> Result<Value> {
    let target = args.state.clone();
    mutate_task(
        cli,
        &args.task,
        args.if_revision,
        None,
        move |project, _, task| {
            verify_transition(&project.config, &task.state, &target)?;
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
                    .get(&target)
                    .is_some_and(|state| state.dependency_satisfied)
            {
                return Err(AppError::new(
                    "acceptance_criteria_incomplete",
                    "All acceptance criteria must be completed before entering a dependency-satisfying state",
                    ErrorCategory::Validation,
                )
                .detail("remaining_acceptance_criteria", remaining));
            }
            let old = std::mem::replace(&mut task.state, target.clone());
            Ok((
                "task.transitioned",
                json!({"state": {"from": old, "to": target}}),
                true,
            ))
        },
    )
}

pub(crate) fn verify_transition(config: &ProjectConfig, current: &str, target: &str) -> Result<()> {
    if !config.workflow.states.contains_key(target) {
        return Err(AppError::new(
            "unknown_state",
            format!("Unknown workflow state {target}"),
            ErrorCategory::Validation,
        )
        .detail("state", target.to_string())
        .detail(
            "configured_states",
            json!(config.workflow.states.keys().collect::<Vec<_>>()),
        ));
    }
    let allowed = config
        .workflow
        .transitions
        .get(current)
        .cloned()
        .unwrap_or_default();
    if !allowed.iter().any(|state| state == target) {
        return Err(AppError::new(
            "transition_not_allowed",
            format!("Transition from {current} to {target} is not allowed"),
            ErrorCategory::Validation,
        )
        .detail("current_state", current.to_string())
        .detail("requested_state", target.to_string())
        .detail("allowed_states", json!(allowed)));
    }
    Ok(())
}

fn unresolved_dependencies(
    task: &Task,
    tasks: &HashMap<String, Task>,
    config: &ProjectConfig,
) -> Vec<String> {
    task.dependencies
        .iter()
        .filter(|id| {
            tasks.get(*id).is_none_or(|dependency| {
                config
                    .workflow
                    .states
                    .get(&dependency.state)
                    .is_none_or(|state| !state.dependency_satisfied)
            })
        })
        .cloned()
        .collect()
}

pub(crate) fn status_for(
    task: &Task,
    tasks: &HashMap<String, Task>,
    config: &ProjectConfig,
) -> StatusView {
    let unresolved = unresolved_dependencies(task, tasks, config);
    let allowed = config
        .workflow
        .transitions
        .get(&task.state)
        .cloned()
        .unwrap_or_default();
    let terminal = config
        .workflow
        .states
        .get(&task.state)
        .is_some_and(|state| state.terminal);
    let claimable = !terminal
        && unresolved.is_empty()
        && task.assignee.is_none()
        && allowed.contains(&config.workflow.claim_state);
    let acceptance_criteria_remaining: Vec<String> = task
        .acceptance_criteria
        .iter()
        .filter(|criterion| !criterion.completed)
        .map(|criterion| criterion.id.clone())
        .collect();
    StatusView {
        id: task.id.clone(),
        state: task.state.clone(),
        assignee: task.assignee.clone(),
        acceptance_criteria_total: task.acceptance_criteria.len(),
        acceptance_criteria_completed: task.acceptance_criteria.len()
            - acceptance_criteria_remaining.len(),
        acceptance_criteria_remaining,
        dependency_blocked: !unresolved.is_empty(),
        unresolved_dependencies: unresolved,
        allowed_transitions: allowed,
        claimable,
    }
}

fn status_command(cli: &Cli, id: &str) -> Result<Value> {
    let project = project_for(cli, Some(id))?;
    ensure_task_id(&project, id)?;
    let (_lock, project) = lock_and_recover(project)?;
    let task = storage::read_task(&project, id)?;
    let tasks = storage::read_tasks(&project)?
        .into_iter()
        .map(|task| (task.id.clone(), task))
        .collect();
    to_value(status_for(&task, &tasks, &project.config))
}

fn claim_command(cli: &Cli, args: &ClaimArgs) -> Result<Value> {
    if args.target.eq_ignore_ascii_case("next") {
        return claim_next(cli, args);
    }
    claim_id(cli, &args.target, &args.as_user, args.if_revision)
}

fn claim_users_locked(cli: &Cli, project: &Project, as_user: &str) -> Result<(User, User)> {
    let (user, user_created) = ensure_user_locked(project, as_user, UserKind::Agent)?;
    let requested_actor = actor_name(cli, Some(as_user));
    let actor = if requested_actor.eq_ignore_ascii_case(&user.id)
        || requested_actor.eq_ignore_ascii_case(&user.name)
    {
        user.clone()
    } else {
        let (actor, actor_created) =
            ensure_user_locked(project, &requested_actor, UserKind::Unknown)?;
        if actor_created {
            event(
                project,
                &actor,
                "user.created",
                None,
                None,
                json!({"user": {"from": null, "to": actor}}),
            )?;
        }
        actor
    };
    if user_created {
        event(
            project,
            &actor,
            "user.created",
            None,
            None,
            json!({"user": {"from": null, "to": user}}),
        )?;
    }
    Ok((user, actor))
}

fn claim_id(cli: &Cli, id: &str, as_user: &str, revision: Option<u64>) -> Result<Value> {
    let project = project_for(cli, Some(id))?;
    ensure_task_id(&project, id)?;
    let (_lock, project) = lock_and_recover(project)?;
    let (user, actor) = claim_users_locked(cli, &project, as_user)?;
    let mut task = storage::read_task(&project, id)?;
    check_revision(&task, revision)?;
    claim_task_locked(&project, &actor, &user, &mut task)?;
    to_value(task)
}

fn claim_task_locked(project: &Project, actor: &User, user: &User, task: &mut Task) -> Result<()> {
    if let Some(owner) = &task.assignee
        && owner != &user.id
    {
        return Err(AppError::new(
            "task_already_assigned",
            format!("Task {} is already assigned to {owner}", task.id),
            ErrorCategory::Conflict,
        )
        .detail("task_id", task.id.clone())
        .detail("assignee", owner.clone()));
    }
    let tasks: HashMap<String, Task> = storage::read_tasks(project)?
        .into_iter()
        .map(|task| (task.id.clone(), task))
        .collect();
    let unresolved = unresolved_dependencies(task, &tasks, &project.config);
    if !unresolved.is_empty() {
        return Err(AppError::new(
            "dependencies_unsatisfied",
            format!("Task {} has unresolved dependencies", task.id),
            ErrorCategory::Validation,
        )
        .detail("task_id", task.id.clone())
        .detail("unresolved_dependencies", json!(unresolved)));
    }
    verify_transition(
        &project.config,
        &task.state,
        &project.config.workflow.claim_state,
    )?;
    let old_assignee = task.assignee.clone();
    let old_state = task.state.clone();
    task.assignee = Some(user.id.clone());
    task.state = project.config.workflow.claim_state.clone();
    task.revision += 1;
    task.updated_at = now();
    task.updated_by = actor.id.clone();
    storage::write_task(project, task)?;
    event(
        project,
        actor,
        "task.claimed",
        Some(&task.id),
        Some(task.revision),
        json!({
            "assignee": {"from": old_assignee, "to": user.id}, "state": {"from": old_state, "to": task.state}
        }),
    )
}

fn actionable(
    task: &Task,
    tasks: &HashMap<String, Task>,
    config: &ProjectConfig,
    user: Option<&User>,
    for_claim: bool,
) -> bool {
    let terminal = config
        .workflow
        .states
        .get(&task.state)
        .is_none_or(|state| state.terminal);
    if terminal || !unresolved_dependencies(task, tasks, config).is_empty() {
        return false;
    }
    match (&task.assignee, user) {
        (Some(_), None) => return false,
        (Some(owner), Some(user)) if owner != &user.id => return false,
        _ => {}
    }
    !for_claim
        || config
            .workflow
            .transitions
            .get(&task.state)
            .is_some_and(|states| states.contains(&config.workflow.claim_state))
}

fn claim_next(cli: &Cli, args: &ClaimArgs) -> Result<Value> {
    let project = project_for(cli, None)?;
    let (_lock, project) = lock_and_recover(project)?;
    let (user, actor) = claim_users_locked(cli, &project, &args.as_user)?;
    let tasks_vec = storage::read_tasks(&project)?;
    let tasks: HashMap<String, Task> = tasks_vec
        .iter()
        .cloned()
        .map(|task| (task.id.clone(), task))
        .collect();
    let mut candidates: Vec<Task> = tasks_vec
        .into_iter()
        .filter(|task| actionable(task, &tasks, &project.config, Some(&user), true))
        .collect();
    candidates.sort_by(|a, b| task_id_cmp(&a.id, &b.id));
    let mut task = candidates.into_iter().next().ok_or_else(|| {
        AppError::new(
            "no_actionable_task",
            "No actionable task is available",
            ErrorCategory::NotFound,
        )
    })?;
    check_revision(&task, args.if_revision)?;
    claim_task_locked(&project, &actor, &user, &mut task)?;
    to_value(task)
}

fn next_command(cli: &Cli, args: &NextArgs) -> Result<Value> {
    let project = project_for(cli, None)?;
    let (_lock, project) = lock_and_recover(project)?;
    let user = args
        .as_user
        .as_deref()
        .map(|name| resolve_user(&project, name))
        .transpose()?;
    let tasks_vec = storage::read_tasks(&project)?;
    let tasks: HashMap<String, Task> = tasks_vec
        .iter()
        .cloned()
        .map(|task| (task.id.clone(), task))
        .collect();
    let selected: Vec<TaskSummary> = tasks_vec
        .into_iter()
        .filter(|task| actionable(task, &tasks, &project.config, user.as_ref(), false))
        .take(args.limit)
        .map(|task| summary(&project, &tasks, &task))
        .collect();
    to_value(selected)
}

fn dependency_command(cli: &Cli, command: &DependencyCommand) -> Result<Value> {
    match command {
        DependencyCommand::Add(args) => dependency_add(cli, args),
        DependencyCommand::Remove(args) => dependency_remove(cli, args),
        DependencyCommand::List(args) => dependency_list(cli, args),
        DependencyCommand::Check(args) => dependency_check(cli, &args.task),
    }
}

fn dependency_add(cli: &Cli, args: &PairTaskArgs) -> Result<Value> {
    if args.task == args.dependency {
        return Err(AppError::new(
            "self_dependency",
            "A task cannot depend on itself",
            ErrorCategory::Cycle,
        )
        .detail("task_id", args.task.clone()));
    }
    let dependency = args.dependency.clone();
    mutate_task(
        cli,
        &args.task,
        args.if_revision,
        None,
        move |project, _, task| {
            ensure_task_id(project, &dependency)?;
            storage::read_task(project, &dependency)?;
            if task.dependencies.contains(&dependency) {
                return Err(AppError::new(
                    "dependency_exists",
                    format!("{} already depends on {dependency}", task.id),
                    ErrorCategory::Conflict,
                ));
            }
            let tasks = storage::read_tasks(project)?;
            if let Some(path) = graph_path(&tasks, &dependency, &task.id, |item| {
                item.dependencies.clone()
            }) {
                let mut cycle = vec![task.id.clone()];
                cycle.extend(path);
                return Err(AppError::new(
                    "dependency_cycle",
                    "Adding the dependency would create a cycle",
                    ErrorCategory::Cycle,
                )
                .detail("cycle", json!(cycle)));
            }
            task.dependencies.push(dependency.clone());
            Ok((
                "task.dependency_added",
                json!({"dependency": {"from": null, "to": dependency}}),
                true,
            ))
        },
    )
}

fn dependency_remove(cli: &Cli, args: &PairTaskArgs) -> Result<Value> {
    let dependency = args.dependency.clone();
    mutate_task(
        cli,
        &args.task,
        args.if_revision,
        None,
        move |_, _, task| {
            let before = task.dependencies.len();
            task.dependencies.retain(|id| id != &dependency);
            if before == task.dependencies.len() {
                return Err(AppError::new(
                    "dependency_not_found",
                    format!("{} does not depend on {dependency}", task.id),
                    ErrorCategory::NotFound,
                ));
            }
            Ok((
                "task.dependency_removed",
                json!({"dependency": {"from": dependency, "to": null}}),
                true,
            ))
        },
    )
}

fn dependency_list(cli: &Cli, args: &DependencyListArgs) -> Result<Value> {
    let project = project_for(cli, Some(&args.task))?;
    let (_lock, project) = lock_and_recover(project)?;
    let task = storage::read_task(&project, &args.task)?;
    let tasks = storage::read_tasks(&project)?;
    let mut ids = if args.reverse {
        tasks
            .iter()
            .filter(|candidate| candidate.dependencies.contains(&task.id))
            .map(|candidate| candidate.id.clone())
            .collect()
    } else if args.recursive {
        transitive_ids(&tasks, &task.id, |item| item.dependencies.clone())
    } else {
        task.dependencies
    };
    ids.sort_by(|a, b| task_id_cmp(a, b));
    ids.dedup();
    Ok(json!(ids))
}

fn dependency_check(cli: &Cli, id: &str) -> Result<Value> {
    let project = project_for(cli, Some(id))?;
    let (_lock, project) = lock_and_recover(project)?;
    let task = storage::read_task(&project, id)?;
    let tasks: HashMap<String, Task> = storage::read_tasks(&project)?
        .into_iter()
        .map(|task| (task.id.clone(), task))
        .collect();
    let unresolved = unresolved_dependencies(&task, &tasks, &project.config);
    Ok(
        json!({"task_id": id, "dependency_blocked": !unresolved.is_empty(), "unresolved_dependencies": unresolved}),
    )
}

fn acceptance_command(cli: &Cli, command: &AcceptanceCommand) -> Result<Value> {
    match command {
        AcceptanceCommand::Add(args) => acceptance_add(cli, args),
        AcceptanceCommand::Remove(args) => acceptance_remove(cli, args),
        AcceptanceCommand::Check(args) => acceptance_set_completed(cli, args, true),
        AcceptanceCommand::Uncheck(args) => acceptance_set_completed(cli, args, false),
        AcceptanceCommand::List(args) => {
            let project = project_for(cli, Some(&args.task))?;
            let (_lock, project) = lock_and_recover(project)?;
            let task = storage::read_task(&project, &args.task)?;
            to_value(task.acceptance_criteria)
        }
    }
}

fn acceptance_add(cli: &Cli, args: &AcceptanceAddArgs) -> Result<Value> {
    let text = args.criterion.trim().to_string();
    if text.is_empty() {
        return Err(AppError::input("Acceptance criterion cannot be empty"));
    }
    mutate_task(
        cli,
        &args.task,
        args.if_revision,
        None,
        move |_, _, task| {
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
            let existing_max = task
                .acceptance_criteria
                .iter()
                .filter_map(|criterion| criterion.id.strip_prefix("AC-")?.parse::<u64>().ok())
                .max()
                .unwrap_or(0);
            let next = task.next_acceptance_number.max(existing_max + 1);
            task.next_acceptance_number = next + 1;
            let criterion = AcceptanceCriterion {
                id: format!("AC-{next}"),
                text: text.clone(),
                completed: false,
                completed_at: None,
                completed_by: None,
            };
            task.acceptance_criteria.push(criterion.clone());
            Ok((
                "task.acceptance_added",
                json!({"acceptance_criterion": {"from": null, "to": criterion}}),
                true,
            ))
        },
    )
}

fn acceptance_remove(cli: &Cli, args: &AcceptanceIdArgs) -> Result<Value> {
    let id = args.acceptance_id.to_uppercase();
    mutate_task(
        cli,
        &args.task,
        args.if_revision,
        None,
        move |_, _, task| {
            let position = task
                .acceptance_criteria
                .iter()
                .position(|criterion| criterion.id.eq_ignore_ascii_case(&id))
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
                json!({"acceptance_criterion": {"from": removed, "to": null}}),
                true,
            ))
        },
    )
}

fn acceptance_set_completed(cli: &Cli, args: &AcceptanceIdArgs, completed: bool) -> Result<Value> {
    let id = args.acceptance_id.to_uppercase();
    mutate_task(
        cli,
        &args.task,
        args.if_revision,
        None,
        move |_, actor, task| {
            let criterion = task
                .acceptance_criteria
                .iter_mut()
                .find(|criterion| criterion.id.eq_ignore_ascii_case(&id))
                .ok_or_else(|| {
                    AppError::new(
                        "acceptance_criterion_not_found",
                        format!("Acceptance criterion {id} does not exist"),
                        ErrorCategory::NotFound,
                    )
                })?;
            if criterion.completed == completed {
                return Ok((
                    if completed {
                        "task.acceptance_completed"
                    } else {
                        "task.acceptance_reopened"
                    },
                    json!({}),
                    false,
                ));
            }
            let previous = criterion.clone();
            criterion.completed = completed;
            criterion.completed_at = completed.then(now);
            criterion.completed_by = completed.then(|| actor.id.clone());
            let updated = criterion.clone();
            Ok((
                if completed {
                    "task.acceptance_completed"
                } else {
                    "task.acceptance_reopened"
                },
                json!({"acceptance_criterion": {"from": previous, "to": updated}}),
                true,
            ))
        },
    )
}

fn context_command(cli: &Cli, command: &ContextCommand) -> Result<Value> {
    match command {
        ContextCommand::Add(args) => context_add(cli, args),
        ContextCommand::List(args) => {
            let project = project_for(cli, Some(&args.task))?;
            let (_lock, project) = lock_and_recover(project)?;
            let task = storage::read_task(&project, &args.task)?;
            let entries: Vec<TaskContextEntry> = task
                .context
                .into_iter()
                .filter(|entry| args.kind.is_none_or(|kind| entry.kind == kind))
                .collect();
            to_value(entries)
        }
    }
}

fn context_add(cli: &Cli, args: &ContextAddArgs) -> Result<Value> {
    let message = args.message.trim().to_string();
    if message.is_empty() {
        return Err(AppError::input("Context message cannot be empty"));
    }
    let kind = args.kind;
    mutate_task(
        cli,
        &args.task,
        args.if_revision,
        None,
        move |_, actor, task| {
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
                message: message.clone(),
                created_at: now(),
                created_by: actor.id.clone(),
            };
            task.context.push(entry.clone());
            Ok((
                "task.context_added",
                json!({"context": {"from": null, "to": entry}}),
                true,
            ))
        },
    )
}

fn graph_path<F>(tasks: &[Task], start: &str, target: &str, edges: F) -> Option<Vec<String>>
where
    F: Fn(&Task) -> Vec<String>,
{
    let map: HashMap<&str, &Task> = tasks.iter().map(|task| (task.id.as_str(), task)).collect();
    let mut queue = VecDeque::from([(start.to_string(), vec![start.to_string()])]);
    let mut visited = HashSet::new();
    while let Some((id, path)) = queue.pop_front() {
        if id == target {
            return Some(path);
        }
        if !visited.insert(id.clone()) {
            continue;
        }
        if let Some(task) = map.get(id.as_str()) {
            for next in edges(task) {
                let mut next_path = path.clone();
                next_path.push(next.clone());
                queue.push_back((next, next_path));
            }
        }
    }
    None
}

fn transitive_ids<F>(tasks: &[Task], start: &str, edges: F) -> Vec<String>
where
    F: Fn(&Task) -> Vec<String>,
{
    let map: HashMap<&str, &Task> = tasks.iter().map(|task| (task.id.as_str(), task)).collect();
    let mut found = BTreeSet::new();
    let mut queue = VecDeque::from([start.to_string()]);
    while let Some(id) = queue.pop_front() {
        if let Some(task) = map.get(id.as_str()) {
            for next in edges(task) {
                if found.insert(next.clone()) {
                    queue.push_back(next);
                }
            }
        }
    }
    found.remove(start);
    found.into_iter().collect()
}

fn relation_command(cli: &Cli, command: &RelationCommand) -> Result<Value> {
    match command {
        RelationCommand::Add(args) => relation_add(cli, args),
        RelationCommand::Remove(args) => relation_remove(cli, args),
        RelationCommand::List(args) => relation_list(cli, args),
    }
}

fn relation_type<'a>(project: &'a Project, kind: &str) -> Result<&'a RelationType> {
    project.config.relations.get(kind).ok_or_else(|| {
        AppError::new(
            "unknown_relation_type",
            format!("Unknown relation type {kind}"),
            ErrorCategory::Validation,
        )
        .detail("relation_type", kind.to_string())
        .detail(
            "configured_relation_types",
            json!(project.config.relations.keys().collect::<Vec<_>>()),
        )
    })
}

fn relation_add(cli: &Cli, args: &RelationMutationArgs) -> Result<Value> {
    if args.from == args.to {
        return Err(AppError::new(
            "self_relation",
            "A task cannot relate to itself",
            ErrorCategory::Validation,
        ));
    }
    let kind = args.kind.clone();
    let target = args.to.clone();
    mutate_task(
        cli,
        &args.from,
        args.if_revision,
        None,
        move |project, _, task| {
            let config = relation_type(project, &kind)?.clone();
            ensure_task_id(project, &target)?;
            storage::read_task(project, &target)?;
            let tasks = storage::read_tasks(project)?;
            let duplicate = task
                .relations
                .iter()
                .any(|relation| relation.kind == kind && relation.target == target)
                || (config.symmetric
                    && tasks
                        .iter()
                        .find(|other| other.id == target)
                        .is_some_and(|other| {
                            other
                                .relations
                                .iter()
                                .any(|r| r.kind == kind && r.target == task.id)
                        }));
            if duplicate {
                return Err(AppError::new(
                    "relation_exists",
                    "The relationship already exists",
                    ErrorCategory::Conflict,
                ));
            }
            if config.acyclic {
                let relation_kind = kind.clone();
                let symmetric = config.symmetric;
                if let Some(path) = graph_path(&tasks, &target, &task.id, |item| {
                    let mut edges: Vec<String> = item
                        .relations
                        .iter()
                        .filter(|r| r.kind == relation_kind)
                        .map(|r| r.target.clone())
                        .collect();
                    if symmetric {
                        edges.extend(
                            tasks
                                .iter()
                                .filter(|other| {
                                    other
                                        .relations
                                        .iter()
                                        .any(|r| r.kind == relation_kind && r.target == item.id)
                                })
                                .map(|other| other.id.clone()),
                        );
                    }
                    edges
                }) {
                    let mut cycle = vec![task.id.clone()];
                    cycle.extend(path);
                    return Err(AppError::new(
                        "relation_cycle",
                        "Adding the relationship would create a cycle",
                        ErrorCategory::Cycle,
                    )
                    .detail("cycle", json!(cycle)));
                }
            }
            task.relations.push(Relation {
                kind: kind.clone(),
                target: target.clone(),
            });
            Ok((
                "task.relation_added",
                json!({"relation": {"from": null, "to": {"type": kind, "target": target}}}),
                true,
            ))
        },
    )
}

fn relation_remove(cli: &Cli, args: &RelationMutationArgs) -> Result<Value> {
    let kind = args.kind.clone();
    let target = args.to.clone();
    mutate_task(
        cli,
        &args.from,
        args.if_revision,
        None,
        move |project, _, task| {
            relation_type(project, &kind)?;
            let before = task.relations.len();
            task.relations
                .retain(|r| !(r.kind == kind && r.target == target));
            if before == task.relations.len() {
                return Err(AppError::new(
                    "relation_not_found",
                    "The relationship does not exist",
                    ErrorCategory::NotFound,
                ));
            }
            Ok((
                "task.relation_removed",
                json!({"relation": {"from": {"type": kind, "target": target}, "to": null}}),
                true,
            ))
        },
    )
}

fn relation_list(cli: &Cli, args: &RelationListArgs) -> Result<Value> {
    let project = project_for(cli, Some(&args.task))?;
    let (_lock, project) = lock_and_recover(project)?;
    storage::read_task(&project, &args.task)?;
    let tasks = storage::read_tasks(&project)?;
    let mut result = Vec::new();
    if matches!(args.direction, Direction::Outgoing | Direction::Both)
        && let Some(task) = tasks.iter().find(|task| task.id == args.task)
    {
        for relation in &task.relations {
            result.push(RelationView {
                source: task.id.clone(),
                kind: relation.kind.clone(),
                target: relation.target.clone(),
                direction: "outgoing".into(),
            });
        }
    }
    if matches!(args.direction, Direction::Incoming | Direction::Both) {
        for task in &tasks {
            for relation in task
                .relations
                .iter()
                .filter(|relation| relation.target == args.task)
            {
                let kind = project
                    .config
                    .relations
                    .get(&relation.kind)
                    .map(|config| config.inverse_label.clone())
                    .unwrap_or_else(|| relation.kind.clone());
                result.push(RelationView {
                    source: args.task.clone(),
                    kind,
                    target: task.id.clone(),
                    direction: "incoming".into(),
                });
            }
        }
    }
    result.sort_by(|a, b| {
        (&a.kind, &a.target, &a.direction).cmp(&(&b.kind, &b.target, &b.direction))
    });
    to_value(result)
}

fn search_command(cli: &Cli, command: &SearchCommand) -> Result<Value> {
    match command {
        SearchCommand::Tasks(args) => {
            let project = project_for(cli, None)?;
            let (_lock, project) = lock_and_recover(project)?;
            let tasks = storage::read_tasks(&project)?;
            tasks_output(&project, tasks, &args.filters, args.query.as_deref())
        }
        SearchCommand::Changelog(args) => {
            let project = project_for(cli, None)?;
            let terms = terms(&args.query);
            let events: Vec<Event> = storage::read_events(&project)?
                .into_iter()
                .filter(|event| {
                    let text = serde_json::to_string(event)
                        .unwrap_or_default()
                        .to_lowercase();
                    terms.iter().all(|term| text.contains(term))
                })
                .take(args.limit)
                .collect();
            to_value(events)
        }
        SearchCommand::Resources(args) => crate::knowledge::commands::search_resources(cli, args),
        SearchCommand::Decisions(args) => crate::knowledge::commands::search_decisions(cli, args),
    }
}

fn terms(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

fn tasks_output(
    project: &Project,
    tasks_vec: Vec<Task>,
    filters: &TaskFilterArgs,
    query: Option<&str>,
) -> Result<Value> {
    for state in &filters.states {
        if !project.config.workflow.states.contains_key(state) {
            return Err(AppError::new(
                "unknown_state",
                format!("Unknown state {state}"),
                ErrorCategory::Validation,
            ));
        }
    }
    let assignee = filters
        .assignee
        .as_deref()
        .map(|name| resolve_user(project, name))
        .transpose()?;
    let tasks: HashMap<String, Task> = tasks_vec
        .iter()
        .cloned()
        .map(|task| (task.id.clone(), task))
        .collect();
    let query_terms = query.map(terms).unwrap_or_default();
    let mut selected: Vec<Task> = tasks_vec
        .into_iter()
        .filter(|task| {
            if !filters.states.is_empty() && !filters.states.contains(&task.state) {
                return false;
            }
            if !filters
                .tags
                .iter()
                .all(|tag| task.tags.contains(&tag.to_lowercase()))
            {
                return false;
            }
            if filters.unassigned && task.assignee.is_some() {
                return false;
            }
            if assignee
                .as_ref()
                .is_some_and(|user| task.assignee.as_deref() != Some(&user.id))
            {
                return false;
            }
            let unresolved = unresolved_dependencies(task, &tasks, &project.config);
            if filters.ready
                && (project
                    .config
                    .workflow
                    .states
                    .get(&task.state)
                    .is_none_or(|s| s.terminal)
                    || !unresolved.is_empty())
            {
                return false;
            }
            if filters.dependency_blocked && unresolved.is_empty() {
                return false;
            }
            if !query_terms.is_empty() {
                let user_name = task
                    .assignee
                    .as_ref()
                    .and_then(|id| storage::read_user_file(project, id).ok())
                    .map(|u| u.name)
                    .unwrap_or_default();
                let acceptance = task
                    .acceptance_criteria
                    .iter()
                    .map(|criterion| criterion.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                let context = task
                    .context
                    .iter()
                    .map(|entry| entry.message.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                let context_refs = task
                    .context_refs
                    .iter()
                    .map(|reference| format!("{} {:?}", reference.id, reference.role))
                    .collect::<Vec<_>>()
                    .join(" ");
                let text = format!(
                    "{} {} {} {} {} {} {} {} {}",
                    task.id,
                    task.header,
                    task.description,
                    acceptance,
                    context,
                    context_refs,
                    task.tags.join(" "),
                    task.assignee.as_deref().unwrap_or(""),
                    user_name
                )
                .to_lowercase();
                if !query_terms.iter().all(|term| text.contains(term)) {
                    return false;
                }
            }
            true
        })
        .collect();
    selected.sort_by(|a, b| task_id_cmp(&a.id, &b.id));
    selected = selected
        .into_iter()
        .skip(filters.offset)
        .take(filters.limit)
        .collect();
    if filters.full {
        to_value(selected)
    } else {
        to_value(
            selected
                .iter()
                .map(|task| summary(project, &tasks, task))
                .collect::<Vec<_>>(),
        )
    }
}

fn summary(project: &Project, tasks: &HashMap<String, Task>, task: &Task) -> TaskSummary {
    TaskSummary {
        id: task.id.clone(),
        header: task.header.clone(),
        state: task.state.clone(),
        assignee: task.assignee.clone(),
        tags: task.tags.clone(),
        dependency_count: task.dependencies.len(),
        dependency_blocked: !unresolved_dependencies(task, tasks, &project.config).is_empty(),
    }
}

fn changelog_command(cli: &Cli, args: &ChangelogArgs) -> Result<Value> {
    let project = project_for(cli, args.task.as_deref())?;
    let events: Vec<Event> = storage::read_events(&project)?
        .into_iter()
        .filter(|event| {
            args.task
                .as_ref()
                .is_none_or(|id| event.task_id.as_ref() == Some(id))
                && args.actor.as_ref().is_none_or(|actor| {
                    event.actor.id.eq_ignore_ascii_case(actor)
                        || event.actor.name.eq_ignore_ascii_case(actor)
                })
                && args
                    .action
                    .as_ref()
                    .is_none_or(|action| &event.action == action)
        })
        .take(args.limit)
        .collect();
    to_value(events)
}

pub(crate) fn event(
    project: &Project,
    actor: &User,
    action: &str,
    task_id: Option<&str>,
    revision: Option<u64>,
    changes: Value,
) -> Result<()> {
    storage::write_event(
        project,
        &Event {
            schema_version: SCHEMA_VERSION,
            timestamp: now(),
            actor: actor.into(),
            action: action.into(),
            task_id: task_id.map(str::to_string),
            task_revision: revision,
            changes,
        },
    )
}

fn validate_project_config(config: &ProjectConfig) -> Result<()> {
    if config.schema_version != SCHEMA_VERSION {
        return Err(AppError::project(format!(
            "Unsupported project schema version {}",
            config.schema_version
        )));
    }
    if !valid_prefix(&config.prefix) {
        return Err(AppError::project("Project prefix is invalid"));
    }
    if !config
        .workflow
        .states
        .contains_key(&config.workflow.initial_state)
    {
        return Err(AppError::project("workflow.initial_state is unknown"));
    }
    if !config
        .workflow
        .states
        .contains_key(&config.workflow.claim_state)
    {
        return Err(AppError::project("workflow.claim_state is unknown"));
    }
    for (from, targets) in &config.workflow.transitions {
        if !config.workflow.states.contains_key(from)
            || targets
                .iter()
                .any(|target| !config.workflow.states.contains_key(target))
        {
            return Err(AppError::project(
                "Workflow transitions reference unknown states",
            ));
        }
    }
    Ok(())
}

fn validate_command(cli: &Cli) -> Result<Value> {
    let project = match project_for(cli, None) {
        Ok(project) => project,
        Err(error) if error.code == "invalid_project_config" => {
            let result = json!({"valid": false, "errors": [problem("malformed_project_config", &error.message, json!({}))], "warnings": []});
            return Err(AppError::new(
                "validation_failed",
                "Project validation failed",
                ErrorCategory::Validation,
            )
            .detail("result", result));
        }
        Err(error) => return Err(error),
    };
    validate_loaded_project(&project)
}

pub(crate) fn validate_loaded_project(project: &Project) -> Result<Value> {
    let mut errors = Vec::<Value>::new();
    let mut warnings = Vec::<Value>::new();
    if let Err(error) = validate_project_config(&project.config) {
        errors.push(problem(&error.code, &error.message, json!({})));
    }
    let task_files = storage::json_files(&project.path.join("tasks"))?;
    let mut tasks = Vec::new();
    for path in task_files {
        match storage::read_json::<Task>(&path) {
            Ok(task) => {
                let filename = path.file_stem().unwrap_or_default().to_string_lossy();
                if filename != task.id {
                    errors.push(problem(
                        "task_filename_mismatch",
                        "Task filename and ID differ",
                        json!({"task_id": task.id, "file": path.to_string_lossy()}),
                    ));
                }
                if task.schema_version != SCHEMA_VERSION {
                    errors.push(problem(
                        "invalid_schema_version",
                        "Task schema version is invalid",
                        json!({"task_id": task.id}),
                    ));
                }
                if !task.id.starts_with(&format!("{}-", project.config.prefix)) {
                    errors.push(problem(
                        "task_prefix_mismatch",
                        "Task ID prefix differs from project",
                        json!({"task_id": task.id}),
                    ));
                }
                if !project.config.workflow.states.contains_key(&task.state) {
                    errors.push(problem(
                        "unknown_task_state",
                        "Task has an unknown state",
                        json!({"task_id": task.id, "state": task.state}),
                    ));
                }
                let acceptance_ids: HashSet<&str> = task
                    .acceptance_criteria
                    .iter()
                    .map(|criterion| criterion.id.as_str())
                    .collect();
                if acceptance_ids.len() != task.acceptance_criteria.len()
                    || task.acceptance_criteria.iter().any(|criterion| {
                        criterion.text.trim().is_empty()
                            || criterion
                                .id
                                .strip_prefix("AC-")
                                .and_then(|number| number.parse::<u64>().ok())
                                .is_none()
                            || criterion.completed != criterion.completed_at.is_some()
                            || criterion.completed != criterion.completed_by.is_some()
                    })
                {
                    errors.push(problem(
                        "invalid_acceptance_criteria",
                        "Acceptance criteria require unique AC-N IDs, text, and consistent completion metadata",
                        json!({"task_id": task.id}),
                    ));
                }
                let context_ids: HashSet<&str> =
                    task.context.iter().map(|entry| entry.id.as_str()).collect();
                if context_ids.len() != task.context.len()
                    || task.context.iter().any(|entry| {
                        entry.message.trim().is_empty()
                            || entry
                                .id
                                .strip_prefix("CTX-")
                                .and_then(|number| number.parse::<u64>().ok())
                                .is_none()
                    })
                {
                    errors.push(problem(
                        "invalid_task_context",
                        "Task context requires unique CTX-N IDs and non-empty messages",
                        json!({"task_id": task.id}),
                    ));
                }
                if task.dependencies.iter().collect::<HashSet<_>>().len() != task.dependencies.len()
                {
                    errors.push(problem(
                        "duplicate_dependency",
                        "Task has duplicate dependencies",
                        json!({"task_id": task.id}),
                    ));
                }
                if task.dependencies.contains(&task.id) {
                    errors.push(problem(
                        "self_dependency",
                        "Task depends on itself",
                        json!({"task_id": task.id}),
                    ));
                }
                tasks.push(task);
            }
            Err(error) => errors.push(problem(
                "malformed_task_json",
                &error.message,
                json!({"file": path.to_string_lossy()}),
            )),
        }
    }
    let user_files = storage::json_files(&project.path.join("users"))?;
    let mut users = Vec::new();
    for path in user_files {
        match storage::read_json::<User>(&path) {
            Ok(user) => {
                if user.schema_version != SCHEMA_VERSION {
                    errors.push(problem(
                        "invalid_schema_version",
                        "User schema version is invalid",
                        json!({"user": user.id}),
                    ));
                }
                if path.file_stem().and_then(|stem| stem.to_str()) != Some(user.id.as_str()) {
                    errors.push(problem(
                        "user_filename_mismatch",
                        "User filename must match its ID",
                        json!({"file": path.to_string_lossy(), "user_id": user.id}),
                    ));
                }
                users.push(user);
            }
            Err(error) => errors.push(problem(
                "malformed_user_json",
                &error.message,
                json!({"file": path.to_string_lossy()}),
            )),
        }
    }
    let user_ids: HashSet<&str> = users.iter().map(|user| user.id.as_str()).collect();
    let mut names = HashSet::new();
    for user in &users {
        if !names.insert(user.name.to_lowercase()) {
            errors.push(problem(
                "duplicate_user_name",
                "User names must be unique case-insensitively",
                json!({"user": user.id}),
            ));
        }
    }
    let task_ids: HashSet<&str> = tasks.iter().map(|task| task.id.as_str()).collect();
    for task in &tasks {
        if task
            .assignee
            .as_ref()
            .is_some_and(|id| !user_ids.contains(id.as_str()))
        {
            errors.push(problem(
                "missing_assignee_user",
                "Task assignee does not exist",
                json!({"task_id": task.id, "assignee": task.assignee}),
            ));
        }
        for dependency in &task.dependencies {
            if !task_ids.contains(dependency.as_str()) {
                errors.push(problem(
                    "dependency_target_missing",
                    "Dependency target does not exist",
                    json!({"task_id": task.id, "dependency": dependency}),
                ));
            }
        }
        for relation in &task.relations {
            if !project.config.relations.contains_key(&relation.kind) {
                errors.push(problem(
                    "unknown_relation_type",
                    "Relation type is not configured",
                    json!({"task_id": task.id, "relation_type": relation.kind}),
                ));
            }
            if relation.target == task.id {
                errors.push(problem(
                    "self_relation",
                    "Task has a self relation",
                    json!({"task_id": task.id}),
                ));
            }
            if !task_ids.contains(relation.target.as_str()) {
                errors.push(problem(
                    "relation_target_missing",
                    "Relation target does not exist",
                    json!({"task_id": task.id, "target": relation.target}),
                ));
            }
        }
    }
    detect_all_cycles(
        &tasks,
        |task| task.dependencies.clone(),
        "dependency_cycle",
        &mut errors,
    );
    for (kind, config) in &project.config.relations {
        if config.acyclic {
            if config.symmetric {
                detect_symmetric_relation_cycle(&tasks, kind, &mut errors);
            } else {
                let wanted = kind.clone();
                detect_all_cycles(
                    &tasks,
                    |task| {
                        task.relations
                            .iter()
                            .filter(|r| r.kind == wanted)
                            .map(|r| r.target.clone())
                            .collect()
                    },
                    "relation_cycle",
                    &mut errors,
                );
            }
        }
    }
    match storage::read_meta(project) {
        Ok(meta) => {
            let max_id = tasks
                .iter()
                .filter_map(|task| task.id.rsplit_once('-')?.1.parse::<u64>().ok())
                .max()
                .unwrap_or(0);
            if meta.schema_version != SCHEMA_VERSION
                || meta.next_task_number == 0
                || meta.next_task_number <= max_id
            {
                errors.push(problem(
                    "invalid_next_task_number",
                    "meta.json next_task_number must exceed all allocated IDs",
                    json!({"next_task_number": meta.next_task_number, "max_task_number": max_id}),
                ));
            }
        }
        Err(error) => errors.push(problem("malformed_meta", &error.message, json!({}))),
    }
    for path in storage::json_files(&project.path.join("changelog"))? {
        match storage::read_json::<Event>(&path) {
            Ok(event) if event.schema_version == SCHEMA_VERSION => {}
            Ok(_) => errors.push(problem(
                "invalid_schema_version",
                "Changelog schema version is invalid",
                json!({"file": path.to_string_lossy()}),
            )),
            Err(error) => errors.push(problem(
                "malformed_changelog_event",
                &error.message,
                json!({"file": path.to_string_lossy()}),
            )),
        }
    }
    validate_knowledge(project, &tasks, &mut errors, &mut warnings)?;
    let problem_key = |value: &Value| {
        (
            value["code"].as_str().unwrap_or("").to_string(),
            value["path"]
                .as_str()
                .or_else(|| value["file"].as_str())
                .unwrap_or("")
                .to_string(),
            value["id"]
                .as_str()
                .or_else(|| value["task_id"].as_str())
                .unwrap_or("")
                .to_string(),
            value["message"].as_str().unwrap_or("").to_string(),
        )
    };
    errors.sort_by_key(&problem_key);
    warnings.sort_by_key(problem_key);
    let result = json!({"valid": errors.is_empty(), "errors": errors, "warnings": warnings});
    if result["valid"] == false {
        Err(AppError::new(
            "validation_failed",
            "Project validation failed",
            ErrorCategory::Validation,
        )
        .detail("result", result))
    } else {
        Ok(result)
    }
}

fn validate_knowledge(
    project: &Project,
    tasks: &[Task],
    errors: &mut Vec<Value>,
    warnings: &mut Vec<Value>,
) -> Result<()> {
    let project_brief_path = project.path.join(crate::knowledge::PROJECT_BRIEF_PATH);
    match fs::symlink_metadata(&project_brief_path) {
        Ok(metadata) if !metadata.file_type().is_file() => errors.push(problem(
            "project_brief_not_file",
            "PROJECT.md must be a regular file",
            json!({"path": project_brief_path.to_string_lossy()}),
        )),
        Ok(_) => match fs::read(&project_brief_path) {
            Ok(bytes) if std::str::from_utf8(&bytes).is_err() => errors.push(problem(
                "project_brief_not_utf8",
                "PROJECT.md must contain valid UTF-8 Markdown",
                json!({"path": project_brief_path.to_string_lossy()}),
            )),
            Ok(_) => {}
            Err(error) => errors.push(problem(
                "project_brief_unreadable",
                &format!("PROJECT.md cannot be read: {error}"),
                json!({"path": project_brief_path.to_string_lossy()}),
            )),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => errors.push(problem(
            "project_brief_unreadable",
            &format!("PROJECT.md cannot be inspected: {error}"),
            json!({"path": project_brief_path.to_string_lossy()}),
        )),
    }

    let missing: Vec<&str> = [
        crate::knowledge::PROJECT_BRIEF_PATH,
        "resources",
        "decisions",
    ]
    .into_iter()
    .filter(|path| !project.path.join(path).exists())
    .collect();
    if !missing.is_empty() {
        warnings.push(problem(
            "project_knowledge_not_initialized",
            "Project knowledge paths are not fully initialized",
            json!({
                "path": project.path.to_string_lossy(),
                "missing": missing,
                "suggestion": format!("tasker project brief init -p {}", project.config.prefix)
            }),
        ));
    }

    let resources = match crate::knowledge::scan_resources(project) {
        Ok(scan) => {
            for malformed in scan.errors {
                errors.push(problem(
                    &malformed.error.code,
                    &malformed.error.message,
                    json!({"path": malformed.path.to_string_lossy()}),
                ));
            }
            scan.records
        }
        Err(error) => {
            errors.push(problem(
                &error.code,
                &error.message,
                json!({"path": project.path.join("resources").to_string_lossy()}),
            ));
            Vec::new()
        }
    };
    for resource in &resources {
        if let Some(source_path) = &resource.metadata.source_path
            && let Err(error) = crate::knowledge::safe_path::read(&project.path, source_path)
        {
            errors.push(problem(
                &error.code,
                &error.message,
                json!({"path": resource.path.to_string_lossy(), "id": resource.metadata.id}),
            ));
        }
    }

    let decisions = match crate::knowledge::scan_decisions(project) {
        Ok(scan) => {
            for malformed in scan.errors {
                errors.push(problem(
                    &malformed.error.code,
                    &malformed.error.message,
                    json!({"path": malformed.path.to_string_lossy()}),
                ));
            }
            scan.records
        }
        Err(error) => {
            errors.push(problem(
                &error.code,
                &error.message,
                json!({"path": project.path.join("decisions").to_string_lossy()}),
            ));
            Vec::new()
        }
    };
    let decision_map: HashMap<&str, &DecisionMeta> = decisions
        .iter()
        .map(|decision| (decision.metadata.id.as_str(), &decision.metadata))
        .collect();
    let mut replacement_for: HashMap<&str, &str> = HashMap::new();
    for decision in &decisions {
        for predecessor in &decision.metadata.supersedes {
            match decision_map.get(predecessor.as_str()) {
                None => errors.push(problem(
                    "missing_supersession_target",
                    "supersedes refers to a missing decision",
                    json!({"path": decision.path.to_string_lossy(), "id": decision.metadata.id, "target": predecessor}),
                )),
                Some(old)
                    if old.status != DecisionStatus::Superseded
                        || old.superseded_by.as_deref() != Some(&decision.metadata.id) =>
                {
                    errors.push(problem(
                        "inconsistent_supersession",
                        "supersedes and superseded_by metadata are inconsistent",
                        json!({"path": decision.path.to_string_lossy(), "id": decision.metadata.id, "target": predecessor}),
                    ));
                }
                Some(_) => {}
            }
            if replacement_for
                .insert(predecessor.as_str(), decision.metadata.id.as_str())
                .is_some()
            {
                errors.push(problem(
                    "multiple_decision_replacements",
                    "A decision has multiple replacement decisions",
                    json!({"id": predecessor}),
                ));
            }
        }
        if let Some(successor) = &decision.metadata.superseded_by {
            match decision_map.get(successor.as_str()) {
                Some(new) if new.supersedes.contains(&decision.metadata.id) => {}
                _ => errors.push(problem(
                    "inconsistent_supersession",
                    "superseded_by has no matching successor edge",
                    json!({"path": decision.path.to_string_lossy(), "id": decision.metadata.id, "target": successor}),
                )),
            }
        }
    }
    for decision in &decisions {
        let mut seen = HashSet::new();
        let mut stack = decision.metadata.supersedes.clone();
        while let Some(id) = stack.pop() {
            if id == decision.metadata.id {
                errors.push(problem(
                    "supersession_cycle",
                    "Decision supersession graph contains a cycle",
                    json!({"id": decision.metadata.id}),
                ));
                break;
            }
            if seen.insert(id.clone())
                && let Some(next) = decision_map.get(id.as_str())
            {
                stack.extend(next.supersedes.clone());
            }
        }
    }

    let resource_ids: HashSet<&str> = resources
        .iter()
        .map(|resource| resource.metadata.id.as_str())
        .collect();
    let decision_ids: HashSet<&str> = decisions
        .iter()
        .map(|decision| decision.metadata.id.as_str())
        .collect();
    for task in tasks {
        let mut targets = HashSet::new();
        for reference in &task.context_refs {
            let canonical = match reference.kind {
                ContextReferenceKind::Resource => {
                    crate::knowledge::canonical_id(project, &reference.id, 'R')
                }
                ContextReferenceKind::Decision => {
                    crate::knowledge::canonical_id(project, &reference.id, 'D')
                }
            };
            if !matches!(canonical.as_ref(), Ok(canonical) if canonical == &reference.id) {
                errors.push(problem(
                    "context_ref_kind_mismatch",
                    "Context reference kind, project, or canonical ID is invalid",
                    json!({"task_id": task.id, "id": reference.id}),
                ));
            }
            let exists = match reference.kind {
                ContextReferenceKind::Resource => resource_ids.contains(reference.id.as_str()),
                ContextReferenceKind::Decision => decision_ids.contains(reference.id.as_str()),
            };
            if !exists {
                errors.push(problem(
                    "dangling_context_ref",
                    "Context reference target does not exist",
                    json!({"task_id": task.id, "id": reference.id}),
                ));
            }
            if !targets.insert(reference.id.as_str()) {
                errors.push(problem(
                    "duplicate_context_ref",
                    "Task contains duplicate knowledge targets",
                    json!({"task_id": task.id, "id": reference.id}),
                ));
            }
        }
        let mut sorted_references = task.context_refs.clone();
        sort_context_refs(&mut sorted_references);
        if sorted_references != task.context_refs {
            errors.push(problem(
                "noncanonical_context_refs",
                "Task context references must be sorted by kind and numeric ID",
                json!({"task_id": task.id}),
            ));
        }
    }

    if let Ok(meta) = storage::read_meta(project) {
        let max_resource = resources
            .iter()
            .filter_map(|item| crate::knowledge::model::numeric_id(&item.metadata.id, 'R'))
            .max()
            .unwrap_or(0);
        let max_decision = decisions
            .iter()
            .filter_map(|item| crate::knowledge::model::numeric_id(&item.metadata.id, 'D'))
            .max()
            .unwrap_or(0);
        if meta.next_resource_number == 0 || meta.next_resource_number <= max_resource {
            errors.push(problem(
                "invalid_next_resource_number",
                "meta.json next_resource_number must exceed all resource IDs",
                json!({"next_resource_number": meta.next_resource_number, "max_resource_number": max_resource}),
            ));
        }
        if meta.next_decision_number == 0 || meta.next_decision_number <= max_decision {
            errors.push(problem(
                "invalid_next_decision_number",
                "meta.json next_decision_number must exceed all decision IDs",
                json!({"next_decision_number": meta.next_decision_number, "max_decision_number": max_decision}),
            ));
        }
    }

    let transactions = project.path.join(".tasker-transactions");
    let transaction_metadata = fs::symlink_metadata(&transactions);
    if !matches!(
        transaction_metadata.as_ref(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound
    ) {
        let inspection_valid = match crate::transaction::inspect_pending(project) {
            Ok(()) => true,
            Err(error) => {
                errors.push(problem(
                    &error.code,
                    &error.message,
                    json!({"path": transactions.to_string_lossy()}),
                ));
                false
            }
        };
        if inspection_valid
            && fs::read_dir(&transactions)
                .map(|mut entries| entries.next().is_some())
                .unwrap_or(false)
        {
            errors.push(problem(
                "transaction_recovery_pending",
                "Knowledge transaction artifacts require recovery before mutation",
                json!({"path": transactions.to_string_lossy()}),
            ));
        }
    }
    Ok(())
}

fn problem(code: &str, message: &str, details: Value) -> Value {
    let mut object = details.as_object().cloned().unwrap_or_default();
    object.insert("code".into(), json!(code));
    object.insert("message".into(), json!(message));
    Value::Object(object)
}

fn detect_symmetric_relation_cycle(tasks: &[Task], kind: &str, errors: &mut Vec<Value>) {
    let mut parent: HashMap<String, String> = tasks
        .iter()
        .map(|task| (task.id.clone(), task.id.clone()))
        .collect();
    fn root(parent: &mut HashMap<String, String>, id: &str) -> String {
        let next = parent.get(id).cloned().unwrap_or_else(|| id.to_string());
        if next == id {
            return next;
        }
        let result = root(parent, &next);
        parent.insert(id.to_string(), result.clone());
        result
    }
    for task in tasks {
        for relation in task
            .relations
            .iter()
            .filter(|relation| relation.kind == kind)
        {
            let left = root(&mut parent, &task.id);
            let right = root(&mut parent, &relation.target);
            if left == right {
                errors.push(problem(
                    "relation_cycle",
                    "Graph contains a cycle",
                    json!({"relation_type": kind, "edge": [task.id, relation.target]}),
                ));
                return;
            }
            parent.insert(left, right);
        }
    }
}

fn detect_all_cycles<F>(tasks: &[Task], edges: F, code: &str, errors: &mut Vec<Value>)
where
    F: Fn(&Task) -> Vec<String>,
{
    let map: HashMap<&str, &Task> = tasks.iter().map(|task| (task.id.as_str(), task)).collect();
    let mut globally_done = HashSet::new();
    fn visit<'a, F>(
        id: &'a str,
        map: &HashMap<&'a str, &'a Task>,
        edges: &F,
        done: &mut HashSet<String>,
        stack: &mut Vec<String>,
    ) -> Option<Vec<String>>
    where
        F: Fn(&Task) -> Vec<String>,
    {
        if let Some(position) = stack.iter().position(|item| item == id) {
            let mut cycle = stack[position..].to_vec();
            cycle.push(id.to_string());
            return Some(cycle);
        }
        if done.contains(id) {
            return None;
        }
        stack.push(id.to_string());
        if let Some(task) = map.get(id) {
            for next in edges(task) {
                if let Some(cycle) = visit(&next, map, edges, done, stack) {
                    return Some(cycle);
                }
            }
        }
        stack.pop();
        done.insert(id.to_string());
        None
    }
    for task in tasks {
        if let Some(cycle) = visit(&task.id, &map, &edges, &mut globally_done, &mut Vec::new()) {
            errors.push(problem(
                code,
                "Graph contains a cycle",
                json!({"cycle": cycle}),
            ));
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, deps: &[&str]) -> Task {
        Task {
            schema_version: 1,
            id: id.into(),
            header: id.into(),
            description: String::new(),
            acceptance_criteria: vec![],
            next_acceptance_number: 1,
            context: vec![],
            context_refs: vec![],
            state: "backlog".into(),
            assignee: None,
            tags: vec![],
            dependencies: deps.iter().map(|s| (*s).into()).collect(),
            relations: vec![],
            created_at: "x".into(),
            created_by: "u".into(),
            updated_at: "x".into(),
            updated_by: "u".into(),
            revision: 1,
        }
    }

    #[test]
    fn dependency_paths_cover_transitive_and_diamond_graphs() {
        let tasks = vec![
            task("D-1", &["D-2", "D-3"]),
            task("D-2", &["D-4"]),
            task("D-3", &["D-4"]),
            task("D-4", &[]),
        ];
        assert_eq!(
            graph_path(&tasks, "D-2", "D-4", |t| t.dependencies.clone()).unwrap(),
            vec!["D-2", "D-4"]
        );
        let mut ids = transitive_ids(&tasks, "D-1", |t| t.dependencies.clone());
        ids.sort();
        assert_eq!(ids, vec!["D-2", "D-3", "D-4"]);
        assert!(graph_path(&tasks, "D-4", "D-1", |t| t.dependencies.clone()).is_none());
    }

    #[test]
    fn transition_rules_are_enforced() {
        let config = default_project("Demo".into(), "DE".into());
        assert!(verify_transition(&config, "backlog", "in_progress").is_ok());
        let error = verify_transition(&config, "done", "backlog").unwrap_err();
        assert_eq!(error.code, "transition_not_allowed");
        assert_eq!(
            verify_transition(&config, "backlog", "imaginary")
                .unwrap_err()
                .code,
            "unknown_state"
        );
    }

    #[test]
    fn slug_generation_is_stable() {
        assert_eq!(slug("Pi Backend"), "pi-backend");
        assert_eq!(slug("Pi   Backend"), "pi-backend");
    }
}
