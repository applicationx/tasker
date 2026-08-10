use super::PROJECT_BRIEF_PATH;
use super::frontmatter::normalized_body;
use super::model::{
    DecisionMeta, DecisionScope, DecisionStatus, MarkdownDocument, ResourceKind, ResourceMeta,
    ResourceStatus, normalize_tags, numeric_id,
};
use crate::app::{actor_locked, event, now};
use crate::cli::*;
use crate::error::{AppError, ErrorCategory, Result};
use crate::model::{
    ContextReference, ContextReferenceKind, ContextReferenceRole, Meta, Task, sort_context_refs,
};
use crate::storage::{self, Project};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::{IsTerminal, Read};
use uuid::Uuid;

pub fn project_command(cli: &Cli, command: &ProjectCommand) -> Result<Value> {
    let project = storage::resolve_project(cli.project.as_deref(), None)?;
    match command {
        ProjectCommand::Brief { command } => project_brief(cli, &project, command),
    }
}

fn project_brief(cli: &Cli, project: &Project, command: &ProjectBriefCommand) -> Result<Value> {
    let path = project.path.join(PROJECT_BRIEF_PATH);
    match command {
        ProjectBriefCommand::Show => {
            let content = fs::read_to_string(&path).map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    AppError::new(
                        "project_brief_not_found",
                        "PROJECT.md does not exist; run tasker project brief init",
                        ErrorCategory::NotFound,
                    )
                } else {
                    AppError::io(error, format!("cannot read {}", path.display()))
                }
            })?;
            Ok(json!({"path": PROJECT_BRIEF_PATH, "content": content}))
        }
        ProjectBriefCommand::Path => Ok(json!({
            "project_relative_path": PROJECT_BRIEF_PATH,
            "absolute_path": path.to_string_lossy()
        })),
        ProjectBriefCommand::Init { force } => set_project_brief(
            cli,
            project,
            super::project_template(&project.config.name),
            true,
            *force,
        ),
        ProjectBriefCommand::Set(args) => {
            let content = required_body(args.content.as_deref(), args.file.as_deref())?;
            set_project_brief(cli, project, normalized_body(&content), false, true)
        }
    }
}

fn set_project_brief(
    cli: &Cli,
    project: &Project,
    content: String,
    initializing: bool,
    overwrite: bool,
) -> Result<Value> {
    let _lock = storage::lock_project(&project.path)?;
    let project = storage::reload_project(project)?;
    crate::transaction::recover(&project)?;
    let path = project.path.join(PROJECT_BRIEF_PATH);
    let before = match fs::read(&path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(AppError::io(
                error,
                format!("cannot read {}", path.display()),
            ));
        }
    };
    if initializing && before.is_some() && !overwrite {
        return Err(AppError::new(
            "project_brief_exists",
            "PROJECT.md already exists; use --force to replace it",
            ErrorCategory::Conflict,
        ));
    }
    if before.as_deref() == Some(content.as_bytes()) {
        return Ok(json!({"path": PROJECT_BRIEF_PATH, "content": content}));
    }
    let actor = actor_locked(cli, &project, None)?;
    storage::atomic_write(&path, content.as_bytes())?;
    let action = if before.is_none() {
        "project.brief.created"
    } else {
        "project.brief.updated"
    };
    let transaction_id = Uuid::new_v4().to_string();
    if let Err(error) = event(
        &project,
        &actor,
        action,
        None,
        None,
        json!({
            "path": PROJECT_BRIEF_PATH,
            "before_sha256": before.as_deref().map(super::sha256),
            "after_sha256": super::sha256(content.as_bytes()),
            "before_bytes": before.as_ref().map(Vec::len),
            "after_bytes": content.len(),
            "transaction_id": transaction_id,
            "initialized": initializing
        }),
    ) {
        restore_file(&path, before.as_deref())?;
        return Err(error);
    }
    Ok(json!({"path": PROJECT_BRIEF_PATH, "content": content}))
}

#[derive(Default)]
struct ResourcePatch {
    title: Option<String>,
    kind: Option<ResourceKind>,
    status: Option<ResourceStatus>,
    content: Option<String>,
    path: Option<String>,
    tags: Option<Vec<String>>,
    include: Option<bool>,
}

pub fn create_resource(cli: &Cli, args: &CreateResourceArgs) -> Result<Value> {
    let mut patch = resource_input(&args.input_args)?;
    apply_create_resource_args(&mut patch, args)?;
    if patch.content.is_none()
        && patch.path.is_none()
        && matches!(args.input_args.input, InputFormat::Human)
    {
        patch.content = piped_stdin().ok().filter(|content| !content.is_empty());
    }
    if patch.path.is_some() && patch.content.is_some() {
        return Err(AppError::input(
            "Referenced resource --path cannot be combined with managed content",
        ));
    }
    let title = nonempty(patch.title, "Resource title is required")?;
    let kind = patch
        .kind
        .ok_or_else(|| AppError::input("Resource --kind is required"))?;
    let status = patch.status.unwrap_or(ResourceStatus::Draft);
    let source_path = patch
        .path
        .map(|path| super::safe_path::normalize(&path))
        .transpose()?;
    let body = if let Some(content) = patch.content {
        normalized_body(&content)
    } else if source_path.is_some() {
        "This resource references existing project documentation.\n".into()
    } else {
        format!("# {title}\n")
    };
    let project = storage::resolve_project(cli.project.as_deref(), None)?;
    if let Some(path) = &source_path {
        super::safe_path::read(&project.path, path)?;
    }
    let _lock = storage::lock_project(&project.path)?;
    let project = storage::reload_project(&project)?;
    crate::transaction::recover(&project)?;
    super::create_managed_directory(&project, "resources")?;
    let existing = super::read_resources(&project)?;
    let mut meta = storage::read_meta(&project)?;
    validate_allocator(
        &meta,
        &existing
            .iter()
            .map(|item| numeric_id(&item.metadata.id, 'R').unwrap_or(0))
            .collect::<Vec<_>>(),
        true,
    )?;
    let actor = actor_locked(cli, &project, None)?;
    let id = format!("{}-R{}", project.config.prefix, meta.next_resource_number);
    let timestamp = now();
    let mut tags = patch.tags.unwrap_or_default();
    normalize_tags(&mut tags);
    let filename = format!("{}-{}.md", id, super::slug(&title));
    let document = MarkdownDocument {
        metadata: ResourceMeta {
            id: id.clone(),
            title,
            kind,
            status,
            source_path,
            tags,
            include_in_project_brief: patch.include.unwrap_or(false),
            created_at: timestamp.clone(),
            updated_at: timestamp,
            created_by: actor.id.clone(),
            updated_by: actor.id.clone(),
            revision: 1,
        },
        body,
        path: project.path.join("resources").join(filename),
    };
    let meta_path = project.path.join("meta.json");
    let old_meta_bytes = fs::read(&meta_path)
        .map_err(|error| AppError::io(error, "cannot read meta.json before resource creation"))?;
    meta.next_resource_number += 1;
    let new_meta_bytes = json_bytes(&meta)?;
    let bytes = super::frontmatter::serialize(&document.metadata, &document.body)?;
    let batch = crate::transaction::MutationBatch::prepare(
        &project,
        "resource.created",
        vec![
            (meta_path, Some(old_meta_bytes), new_meta_bytes),
            (document.path.clone(), None, bytes.clone()),
        ],
    )?;
    batch.apply(&project)?;
    if let Err(error) = event(
        &project,
        &actor,
        "resource.created",
        None,
        None,
        json!({
            "resource_id": id, "path": relative(&project, &document.path), "revision": 1,
            "metadata": {"from": null, "to": document.metadata},
            "sha256": super::sha256(&bytes), "bytes": bytes.len(), "transaction_id": batch.id()
        }),
    ) {
        batch.rollback(&project)?;
        batch.finish()?;
        return Err(error);
    }
    batch.finish()?;
    super::resource_value(&project, &document, true)
}

fn apply_create_resource_args(patch: &mut ResourcePatch, args: &CreateResourceArgs) -> Result<()> {
    patch.title = args.title.clone().or(patch.title.take());
    patch.kind = args.kind.or(patch.kind);
    patch.status = args.status.or(patch.status);
    if let Some(content) = &args.content {
        patch.content = Some(content.clone());
    }
    if let Some(file) = &args.file {
        patch.content = Some(read_utf8(file)?);
    }
    if let Some(path) = &args.path {
        patch.path = Some(path.clone());
    }
    if !args.tags.is_empty() {
        patch.tags = Some(args.tags.clone());
    }
    if args.include_in_project_brief {
        patch.include = Some(true);
    }
    if args.exclude_from_project_brief {
        patch.include = Some(false);
    }
    Ok(())
}

fn apply_update_resource_args(patch: &mut ResourcePatch, args: &UpdateResourceArgs) -> Result<()> {
    patch.title = args.title.clone().or(patch.title.take());
    patch.kind = args.kind.or(patch.kind);
    patch.status = args.status.or(patch.status);
    if let Some(content) = &args.content {
        patch.content = Some(content.clone());
    }
    if let Some(file) = &args.file {
        patch.content = Some(read_utf8(file)?);
    }
    if let Some(path) = &args.path {
        patch.path = Some(path.clone());
    }
    if !args.tags.is_empty() || args.clear_tags {
        patch.tags = Some(args.tags.clone());
    }
    if args.include_in_project_brief {
        patch.include = Some(true);
    }
    if args.exclude_from_project_brief {
        patch.include = Some(false);
    }
    Ok(())
}

fn resource_input(input: &KnowledgeInputArgs) -> Result<ResourcePatch> {
    let Some(object) = structured_object(input)? else {
        return Ok(ResourcePatch::default());
    };
    reject_unknown(
        &object,
        &[
            "title",
            "kind",
            "status",
            "content",
            "path",
            "tags",
            "include_in_project_brief",
        ],
    )?;
    Ok(ResourcePatch {
        title: string_field(&object, "title")?,
        kind: enum_field(&object, "kind")?,
        status: enum_field(&object, "status")?,
        content: string_field(&object, "content")?,
        path: string_field(&object, "path")?,
        tags: vec_field(&object, "tags")?,
        include: bool_field(&object, "include_in_project_brief")?,
    })
}

pub fn list_resources(cli: &Cli, args: &ResourceListArgs) -> Result<Value> {
    let project = storage::resolve_project(cli.project.as_deref(), None)?;
    let _lock = storage::lock_project(&project.path)?;
    let project = storage::reload_project(&project)?;
    crate::transaction::recover(&project)?;
    resource_results(&project, args, None, false)
}

fn resource_results(
    project: &Project,
    args: &ResourceListArgs,
    query: Option<&str>,
    search_includes_archived: bool,
) -> Result<Value> {
    let wanted_tags: Vec<String> = args.tags.iter().map(|tag| tag.to_lowercase()).collect();
    let wanted_terms = query.map(terms).unwrap_or_default();
    let explicit_status = !args.statuses.is_empty();
    let mut values = Vec::new();
    for document in super::read_resources(project)? {
        let metadata = &document.metadata;
        if (!explicit_status
            && !args.include_archived
            && !search_includes_archived
            && metadata.status == ResourceStatus::Archived)
            || (explicit_status && !args.statuses.contains(&metadata.status))
            || (!args.kinds.is_empty() && !args.kinds.contains(&metadata.kind))
            || !wanted_tags.iter().all(|tag| metadata.tags.contains(tag))
        {
            continue;
        }
        if !wanted_terms.is_empty() {
            let live = metadata
                .source_path
                .as_deref()
                .and_then(|path| super::safe_path::read(&project.path, path).ok())
                .unwrap_or_else(|| document.body.clone());
            let text = format!(
                "{} {} {:?} {:?} {} {} {}",
                metadata.id,
                metadata.title,
                metadata.kind,
                metadata.status,
                metadata.tags.join(" "),
                metadata.source_path.as_deref().unwrap_or(""),
                live
            )
            .to_ascii_lowercase();
            if !wanted_terms.iter().all(|term| text.contains(term)) {
                continue;
            }
        }
        values.push(super::resource_value(project, &document, args.full)?);
    }
    Ok(Value::Array(
        values
            .into_iter()
            .skip(args.offset)
            .take(args.limit)
            .collect(),
    ))
}

pub fn get_resource(cli: &Cli, id: &str) -> Result<Value> {
    let project = storage::resolve_project(cli.project.as_deref(), Some(id))?;
    let id = super::canonical_id(&project, id, 'R')?;
    let _lock = storage::lock_project(&project.path)?;
    let project = storage::reload_project(&project)?;
    crate::transaction::recover(&project)?;
    let document = super::read_resources(&project)?
        .into_iter()
        .find(|item| item.metadata.id == id)
        .ok_or_else(|| not_found("resource", &id))?;
    super::resource_value(&project, &document, true)
}

pub fn update_resource(cli: &Cli, args: &UpdateResourceArgs) -> Result<Value> {
    let mut patch = resource_input(&args.input_args)?;
    apply_update_resource_args(&mut patch, args)?;
    if patch.content.is_none() && matches!(args.input_args.input, InputFormat::Human) {
        patch.content = piped_stdin().ok().filter(|content| !content.is_empty());
    }
    if patch.title.is_none()
        && patch.kind.is_none()
        && patch.status.is_none()
        && patch.content.is_none()
        && patch.path.is_none()
        && patch.tags.is_none()
        && patch.include.is_none()
    {
        return Err(AppError::input("No mutable resource fields were supplied"));
    }
    let project = storage::resolve_project(cli.project.as_deref(), Some(&args.resource))?;
    let id = super::canonical_id(&project, &args.resource, 'R')?;
    let _lock = storage::lock_project(&project.path)?;
    let project = storage::reload_project(&project)?;
    crate::transaction::recover(&project)?;
    let mut document = super::read_resources(&project)?
        .into_iter()
        .find(|item| item.metadata.id == id)
        .ok_or_else(|| not_found("resource", &id))?;
    revision_guard(&id, document.metadata.revision, args.if_revision)?;
    let before_bytes = fs::read(&document.path)
        .map_err(|error| AppError::io(error, "cannot read resource before update"))?;
    let before_metadata = document.metadata.clone();
    if let Some(title) = patch.title {
        document.metadata.title = nonempty(Some(title), "Resource title cannot be empty")?;
    }
    if let Some(kind) = patch.kind {
        document.metadata.kind = kind;
    }
    if let Some(status) = patch.status {
        document.metadata.status = status;
    }
    match (
        document.metadata.source_path.is_some(),
        patch.content,
        patch.path,
    ) {
        (true, Some(_), _) => {
            return Err(AppError::new(
                "resource_source_type_conflict",
                "Referenced resources cannot replace managed content",
                ErrorCategory::Conflict,
            ));
        }
        (false, _, Some(_)) => {
            return Err(AppError::new(
                "resource_source_type_conflict",
                "Managed resources cannot become referenced",
                ErrorCategory::Conflict,
            ));
        }
        (false, Some(content), None) => document.body = normalized_body(&content),
        (true, None, Some(path)) => {
            let path = super::safe_path::normalize(&path)?;
            super::safe_path::read(&project.path, &path)?;
            document.metadata.source_path = Some(path);
        }
        _ => {}
    }
    if let Some(mut tags) = patch.tags {
        normalize_tags(&mut tags);
        document.metadata.tags = tags;
    }
    if let Some(include) = patch.include {
        document.metadata.include_in_project_brief = include;
    }
    let comparison = super::frontmatter::serialize(&document.metadata, &document.body)?;
    if comparison == before_bytes {
        return super::resource_value(&project, &document, true);
    }
    let actor = actor_locked(cli, &project, None)?;
    document.metadata.revision += 1;
    document.metadata.updated_at = now();
    document.metadata.updated_by = actor.id.clone();
    let after = super::frontmatter::serialize(&document.metadata, &document.body)?;
    write_managed_document(&project, "resources", &document.path, &after)?;
    let transaction_id = Uuid::new_v4().to_string();
    if let Err(error) = event(
        &project,
        &actor,
        "resource.updated",
        None,
        None,
        json!({
            "resource_id": id, "path": relative(&project, &document.path), "revision": document.metadata.revision,
            "metadata": {"from": before_metadata, "to": document.metadata},
            "before_sha256": super::sha256(&before_bytes), "after_sha256": super::sha256(&after),
            "before_bytes": before_bytes.len(), "after_bytes": after.len(), "transaction_id": transaction_id
        }),
    ) {
        write_managed_document(&project, "resources", &document.path, &before_bytes)?;
        return Err(error);
    }
    super::resource_value(&project, &document, true)
}

pub fn archive_resource(cli: &Cli, args: &KnowledgeRevisionArgs) -> Result<Value> {
    let project = storage::resolve_project(cli.project.as_deref(), Some(&args.id))?;
    let id = super::canonical_id(&project, &args.id, 'R')?;
    let _lock = storage::lock_project(&project.path)?;
    let project = storage::reload_project(&project)?;
    crate::transaction::recover(&project)?;
    let mut document = super::read_resources(&project)?
        .into_iter()
        .find(|item| item.metadata.id == id)
        .ok_or_else(|| not_found("resource", &id))?;
    revision_guard(&id, document.metadata.revision, args.if_revision)?;
    if document.metadata.status == ResourceStatus::Archived {
        return super::resource_value(&project, &document, true);
    }
    let before =
        fs::read(&document.path).map_err(|error| AppError::io(error, "cannot read resource"))?;
    let before_metadata = document.metadata.clone();
    let actor = actor_locked(cli, &project, None)?;
    let old_status = document.metadata.status;
    document.metadata.status = ResourceStatus::Archived;
    document.metadata.revision += 1;
    document.metadata.updated_at = now();
    document.metadata.updated_by = actor.id.clone();
    let after = super::frontmatter::serialize(&document.metadata, &document.body)?;
    write_managed_document(&project, "resources", &document.path, &after)?;
    if let Err(error) = event(
        &project,
        &actor,
        "resource.archived",
        None,
        None,
        json!({
            "resource_id": id, "status": {"from": old_status, "to": "archived"}, "revision": document.metadata.revision,
            "metadata": {"from": before_metadata, "to": document.metadata},
            "path": relative(&project, &document.path), "before_sha256": super::sha256(&before), "after_sha256": super::sha256(&after),
            "before_bytes": before.len(), "after_bytes": after.len(), "transaction_id": Uuid::new_v4().to_string()
        }),
    ) {
        write_managed_document(&project, "resources", &document.path, &before)?;
        return Err(error);
    }
    super::resource_value(&project, &document, true)
}

#[derive(Default)]
struct DecisionPatch {
    title: Option<String>,
    scope: Option<DecisionScope>,
    content: Option<String>,
    tags: Option<Vec<String>>,
}

fn default_decision_body(title: &str) -> String {
    format!(
        "# Context\n\nDescribe why {title} requires a decision.\n\n# Options considered\n\nDescribe the options considered.\n\n# Decision\n\nDescribe the chosen direction.\n\n# Rationale\n\nDescribe why this direction is preferred.\n\n# Consequences\n\nDescribe resulting constraints and work.\n"
    )
}

pub fn create_decision(cli: &Cli, args: &CreateDecisionArgs) -> Result<Value> {
    let mut patch = decision_input(&args.input_args)?;
    override_decision_patch(
        &mut patch,
        args.title.clone(),
        args.scope,
        args.content.clone(),
        args.file.as_deref(),
        &args.tags,
        false,
    )?;
    let title = nonempty(patch.title, "Decision title is required")?;
    let title_slug = super::slug(&title);
    let content = patch
        .content
        .or_else(|| piped_stdin().ok().filter(|value| !value.is_empty()))
        .unwrap_or_else(|| default_decision_body(&title));
    let body = normalized_body(&content);
    super::validate_decision_sections(&body)?;
    let project = storage::resolve_project(cli.project.as_deref(), None)?;
    let _lock = storage::lock_project(&project.path)?;
    let project = storage::reload_project(&project)?;
    crate::transaction::recover(&project)?;
    super::create_managed_directory(&project, "decisions")?;
    let existing = super::read_decisions(&project)?;
    let mut meta = storage::read_meta(&project)?;
    validate_allocator(
        &meta,
        &existing
            .iter()
            .map(|item| numeric_id(&item.metadata.id, 'D').unwrap_or(0))
            .collect::<Vec<_>>(),
        false,
    )?;
    let actor = actor_locked(cli, &project, None)?;
    let id = format!("{}-D{}", project.config.prefix, meta.next_decision_number);
    let timestamp = now();
    let mut tags = patch.tags.unwrap_or_default();
    normalize_tags(&mut tags);
    let document = MarkdownDocument {
        metadata: DecisionMeta {
            id: id.clone(),
            title,
            status: DecisionStatus::Proposed,
            scope: patch.scope.unwrap_or(DecisionScope::Project),
            tags,
            created_at: timestamp.clone(),
            updated_at: timestamp,
            created_by: actor.id.clone(),
            updated_by: actor.id.clone(),
            decided_at: None,
            decided_by: None,
            supersedes: Vec::new(),
            superseded_by: None,
            revision: 1,
        },
        body,
        path: project
            .path
            .join("decisions")
            .join(format!("{}-{}.md", id, title_slug)),
    };
    let meta_path = project.path.join("meta.json");
    let old_meta_bytes = fs::read(&meta_path)
        .map_err(|error| AppError::io(error, "cannot read meta.json before decision creation"))?;
    meta.next_decision_number += 1;
    let new_meta_bytes = json_bytes(&meta)?;
    let bytes = super::frontmatter::serialize(&document.metadata, &document.body)?;
    let batch = crate::transaction::MutationBatch::prepare(
        &project,
        "decision.created",
        vec![
            (meta_path, Some(old_meta_bytes), new_meta_bytes),
            (document.path.clone(), None, bytes.clone()),
        ],
    )?;
    batch.apply(&project)?;
    if let Err(error) = event(
        &project,
        &actor,
        "decision.created",
        None,
        None,
        json!({
            "decision_id": id, "path": relative(&project, &document.path), "revision": 1,
            "metadata": {"from": null, "to": document.metadata},
            "sha256": super::sha256(&bytes), "bytes": bytes.len(), "transaction_id": batch.id()
        }),
    ) {
        batch.rollback(&project)?;
        batch.finish()?;
        return Err(error);
    }
    batch.finish()?;
    super::decision_value(&document, true)
}

fn decision_input(input: &KnowledgeInputArgs) -> Result<DecisionPatch> {
    let Some(object) = structured_object(input)? else {
        return Ok(DecisionPatch::default());
    };
    reject_unknown(&object, &["title", "scope", "content", "tags"])?;
    Ok(DecisionPatch {
        title: string_field(&object, "title")?,
        scope: enum_field(&object, "scope")?,
        content: string_field(&object, "content")?,
        tags: vec_field(&object, "tags")?,
    })
}

fn override_decision_patch(
    patch: &mut DecisionPatch,
    title: Option<String>,
    scope: Option<DecisionScope>,
    content: Option<String>,
    file: Option<&str>,
    tags: &[String],
    clear_tags: bool,
) -> Result<()> {
    if title.is_some() {
        patch.title = title;
    }
    if scope.is_some() {
        patch.scope = scope;
    }
    if let Some(content) = content {
        patch.content = Some(content);
    }
    if let Some(file) = file {
        patch.content = Some(read_utf8(file)?);
    }
    if !tags.is_empty() || clear_tags {
        patch.tags = Some(tags.to_vec());
    }
    Ok(())
}

pub fn list_decisions(cli: &Cli, args: &DecisionListArgs) -> Result<Value> {
    let project = storage::resolve_project(cli.project.as_deref(), None)?;
    let _lock = storage::lock_project(&project.path)?;
    let project = storage::reload_project(&project)?;
    crate::transaction::recover(&project)?;
    decision_results(&project, args, None)
}

fn decision_results(
    project: &Project,
    args: &DecisionListArgs,
    query: Option<&str>,
) -> Result<Value> {
    let wanted_tags: Vec<String> = args.tags.iter().map(|tag| tag.to_lowercase()).collect();
    let wanted_terms = query.map(terms).unwrap_or_default();
    let mut values = Vec::new();
    for document in super::read_decisions(project)? {
        let metadata = &document.metadata;
        if (!args.statuses.is_empty() && !args.statuses.contains(&metadata.status))
            || (!args.scopes.is_empty() && !args.scopes.contains(&metadata.scope))
            || !wanted_tags.iter().all(|tag| metadata.tags.contains(tag))
        {
            continue;
        }
        if !wanted_terms.is_empty() {
            let text = format!(
                "{} {} {:?} {:?} {} {} {} {}",
                metadata.id,
                metadata.title,
                metadata.status,
                metadata.scope,
                metadata.tags.join(" "),
                metadata.supersedes.join(" "),
                metadata.superseded_by.as_deref().unwrap_or(""),
                document.body
            )
            .to_ascii_lowercase();
            if !wanted_terms.iter().all(|term| text.contains(term)) {
                continue;
            }
        }
        values.push(super::decision_value(&document, args.full)?);
    }
    Ok(Value::Array(
        values
            .into_iter()
            .skip(args.offset)
            .take(args.limit)
            .collect(),
    ))
}

pub fn get_decision(cli: &Cli, id: &str) -> Result<Value> {
    let project = storage::resolve_project(cli.project.as_deref(), Some(id))?;
    let id = super::canonical_id(&project, id, 'D')?;
    let _lock = storage::lock_project(&project.path)?;
    let project = storage::reload_project(&project)?;
    crate::transaction::recover(&project)?;
    let document = super::read_decisions(&project)?
        .into_iter()
        .find(|item| item.metadata.id == id)
        .ok_or_else(|| not_found("decision", &id))?;
    super::decision_value(&document, true)
}

pub fn update_decision(cli: &Cli, args: &UpdateDecisionArgs) -> Result<Value> {
    let mut patch = decision_input(&args.input_args)?;
    override_decision_patch(
        &mut patch,
        args.title.clone(),
        args.scope,
        args.content.clone(),
        args.file.as_deref(),
        &args.tags,
        args.clear_tags,
    )?;
    if patch.content.is_none() && matches!(args.input_args.input, InputFormat::Human) {
        patch.content = piped_stdin().ok().filter(|content| !content.is_empty());
    }
    if patch.title.is_none()
        && patch.scope.is_none()
        && patch.content.is_none()
        && patch.tags.is_none()
    {
        return Err(AppError::input("No mutable decision fields were supplied"));
    }
    let project = storage::resolve_project(cli.project.as_deref(), Some(&args.decision))?;
    let id = super::canonical_id(&project, &args.decision, 'D')?;
    let _lock = storage::lock_project(&project.path)?;
    let project = storage::reload_project(&project)?;
    crate::transaction::recover(&project)?;
    let mut document = super::read_decisions(&project)?
        .into_iter()
        .find(|item| item.metadata.id == id)
        .ok_or_else(|| not_found("decision", &id))?;
    revision_guard(&id, document.metadata.revision, args.if_revision)?;
    if document.metadata.status != DecisionStatus::Proposed
        && (patch.title.is_some() || patch.scope.is_some() || patch.content.is_some())
    {
        return Err(AppError::new(
            "decision_immutable",
            "Settled decisions permit tag updates only",
            ErrorCategory::Conflict,
        ));
    }
    let before =
        fs::read(&document.path).map_err(|error| AppError::io(error, "cannot read decision"))?;
    let before_metadata = document.metadata.clone();
    if let Some(title) = patch.title {
        document.metadata.title = nonempty(Some(title), "Decision title cannot be empty")?;
    }
    if let Some(scope) = patch.scope {
        document.metadata.scope = scope;
    }
    if let Some(content) = patch.content {
        let body = normalized_body(&content);
        super::validate_decision_sections(&body)?;
        document.body = body;
    }
    if let Some(mut tags) = patch.tags {
        normalize_tags(&mut tags);
        document.metadata.tags = tags;
    }
    if super::frontmatter::serialize(&document.metadata, &document.body)? == before {
        return super::decision_value(&document, true);
    }
    let actor = actor_locked(cli, &project, None)?;
    document.metadata.revision += 1;
    document.metadata.updated_at = now();
    document.metadata.updated_by = actor.id.clone();
    let after = super::frontmatter::serialize(&document.metadata, &document.body)?;
    write_managed_document(&project, "decisions", &document.path, &after)?;
    let mut changes = knowledge_changes(
        "decision_id",
        &id,
        &document.path,
        &project,
        document.metadata.revision,
        &before,
        &after,
    );
    changes.as_object_mut().unwrap().insert(
        "metadata".into(),
        json!({"from": before_metadata, "to": document.metadata}),
    );
    if let Err(error) = event(&project, &actor, "decision.updated", None, None, changes) {
        write_managed_document(&project, "decisions", &document.path, &before)?;
        return Err(error);
    }
    super::decision_value(&document, true)
}

pub fn decision_lifecycle(cli: &Cli, command: &DecisionCommand) -> Result<Value> {
    match command {
        DecisionCommand::Accept(args) => {
            set_decision_status(cli, args, DecisionStatus::Accepted, "decision.accepted")
        }
        DecisionCommand::Reject(args) => {
            set_decision_status(cli, args, DecisionStatus::Rejected, "decision.rejected")
        }
        DecisionCommand::Supersede(args) => supersede(cli, args),
    }
}

fn set_decision_status(
    cli: &Cli,
    args: &KnowledgeRevisionArgs,
    status: DecisionStatus,
    action: &str,
) -> Result<Value> {
    let project = storage::resolve_project(cli.project.as_deref(), Some(&args.id))?;
    let id = super::canonical_id(&project, &args.id, 'D')?;
    let _lock = storage::lock_project(&project.path)?;
    let project = storage::reload_project(&project)?;
    crate::transaction::recover(&project)?;
    let mut document = super::read_decisions(&project)?
        .into_iter()
        .find(|item| item.metadata.id == id)
        .ok_or_else(|| not_found("decision", &id))?;
    revision_guard(&id, document.metadata.revision, args.if_revision)?;
    if document.metadata.status != DecisionStatus::Proposed {
        return Err(AppError::new(
            "invalid_decision_transition",
            "Only a proposed decision can be accepted or rejected",
            ErrorCategory::Validation,
        ));
    }
    let before =
        fs::read(&document.path).map_err(|error| AppError::io(error, "cannot read decision"))?;
    let before_metadata = document.metadata.clone();
    let actor = actor_locked(cli, &project, None)?;
    let timestamp = now();
    document.metadata.status = status;
    document.metadata.decided_at = Some(timestamp.clone());
    document.metadata.decided_by = Some(actor.id.clone());
    document.metadata.updated_at = timestamp;
    document.metadata.updated_by = actor.id.clone();
    document.metadata.revision += 1;
    let after = super::frontmatter::serialize(&document.metadata, &document.body)?;
    write_managed_document(&project, "decisions", &document.path, &after)?;
    let mut changes = knowledge_changes(
        "decision_id",
        &id,
        &document.path,
        &project,
        document.metadata.revision,
        &before,
        &after,
    );
    changes.as_object_mut().unwrap().insert(
        "metadata".into(),
        json!({"from": before_metadata, "to": document.metadata}),
    );
    if let Err(error) = event(&project, &actor, action, None, None, changes) {
        write_managed_document(&project, "decisions", &document.path, &before)?;
        return Err(error);
    }
    super::decision_value(&document, true)
}

fn supersede(cli: &Cli, args: &SupersedeArgs) -> Result<Value> {
    if args.decision.eq_ignore_ascii_case(&args.replacement) {
        return Err(AppError::new(
            "self_supersession",
            "A decision cannot supersede itself",
            ErrorCategory::Cycle,
        ));
    }
    let project = storage::resolve_project(cli.project.as_deref(), Some(&args.decision))?;
    let old_id = super::canonical_id(&project, &args.decision, 'D')?;
    let new_id = super::canonical_id(&project, &args.replacement, 'D')?;
    let _lock = storage::lock_project(&project.path)?;
    let project = storage::reload_project(&project)?;
    crate::transaction::recover(&project)?;
    let decisions = super::read_decisions(&project)?;
    let mut old = decisions
        .iter()
        .find(|item| item.metadata.id == old_id)
        .cloned()
        .ok_or_else(|| not_found("decision", &old_id))?;
    let mut replacement = decisions
        .iter()
        .find(|item| item.metadata.id == new_id)
        .cloned()
        .ok_or_else(|| not_found("decision", &new_id))?;
    revision_guard(&old_id, old.metadata.revision, args.if_revision)?;
    revision_guard(
        &new_id,
        replacement.metadata.revision,
        args.with_if_revision,
    )?;
    if old.metadata.status != DecisionStatus::Accepted
        || replacement.metadata.status != DecisionStatus::Accepted
    {
        return Err(AppError::new(
            "invalid_decision_transition",
            "Both superseded and replacement decisions must be accepted",
            ErrorCategory::Validation,
        ));
    }
    if old.metadata.superseded_by.is_some() {
        return Err(AppError::new(
            "decision_already_superseded",
            "The superseded decision already has a replacement",
            ErrorCategory::Conflict,
        ));
    }
    if replacement.metadata.supersedes.contains(&old_id) {
        return Err(AppError::new(
            "supersession_exists",
            "The supersession already exists",
            ErrorCategory::Conflict,
        ));
    }
    if follows_supersedes(&decisions, &old_id, &new_id) {
        return Err(AppError::new(
            "supersession_cycle",
            "Supersession would create a cycle",
            ErrorCategory::Cycle,
        ));
    }
    let old_before =
        fs::read(&old.path).map_err(|error| AppError::io(error, "cannot read old decision"))?;
    let replacement_before = fs::read(&replacement.path)
        .map_err(|error| AppError::io(error, "cannot read replacement"))?;
    let actor = actor_locked(cli, &project, None)?;
    let timestamp = now();
    old.metadata.status = DecisionStatus::Superseded;
    old.metadata.superseded_by = Some(new_id.clone());
    old.metadata.updated_at = timestamp.clone();
    old.metadata.updated_by = actor.id.clone();
    old.metadata.revision += 1;
    replacement.metadata.supersedes.push(old_id.clone());
    replacement
        .metadata
        .supersedes
        .sort_by_key(|id| numeric_id(id, 'D').unwrap_or(u64::MAX));
    replacement.metadata.updated_at = timestamp;
    replacement.metadata.updated_by = actor.id.clone();
    replacement.metadata.revision += 1;
    let old_after = super::frontmatter::serialize(&old.metadata, &old.body)?;
    let replacement_after =
        super::frontmatter::serialize(&replacement.metadata, &replacement.body)?;
    let batch = crate::transaction::MutationBatch::prepare(
        &project,
        "decision.superseded",
        vec![
            (
                old.path.clone(),
                Some(old_before.clone()),
                old_after.clone(),
            ),
            (
                replacement.path.clone(),
                Some(replacement_before.clone()),
                replacement_after.clone(),
            ),
        ],
    )?;
    batch.apply(&project)?;
    let changes = json!({"decision_id": old_id, "replacement_id": new_id, "old_revision": old.metadata.revision, "replacement_revision": replacement.metadata.revision,
        "status": {"from": "accepted", "to": "superseded"}, "superseded_by": {"from": null, "to": new_id},
        "replacement_supersedes": {"from": replacement.metadata.supersedes.iter().filter(|id| *id != &old_id).collect::<Vec<_>>(), "to": replacement.metadata.supersedes},
        "old_before_sha256": super::sha256(&old_before), "old_after_sha256": super::sha256(&old_after), "replacement_before_sha256": super::sha256(&replacement_before), "replacement_after_sha256": super::sha256(&replacement_after), "transaction_id": batch.id()});
    if let Err(error) = event(&project, &actor, "decision.superseded", None, None, changes) {
        batch.rollback(&project)?;
        batch.finish()?;
        return Err(error);
    }
    batch.finish()?;
    Ok(
        json!({"superseded": super::decision_value(&old, true)?, "replacement": super::decision_value(&replacement, true)?}),
    )
}

fn follows_supersedes(
    decisions: &[MarkdownDocument<DecisionMeta>],
    start: &str,
    target: &str,
) -> bool {
    let map: HashMap<&str, &DecisionMeta> = decisions
        .iter()
        .map(|item| (item.metadata.id.as_str(), &item.metadata))
        .collect();
    let mut stack = vec![start];
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if id == target {
            return true;
        }
        if seen.insert(id)
            && let Some(meta) = map.get(id)
        {
            stack.extend(meta.supersedes.iter().map(String::as_str));
        }
    }
    false
}

pub fn context_ref(cli: &Cli, command: &ContextRefCommand) -> Result<Value> {
    match command {
        ContextRefCommand::Add(args) => mutate_context_ref(
            cli,
            &args.task,
            &args.knowledge_id,
            Some(args.role),
            args.if_revision,
        ),
        ContextRefCommand::Remove(args) => {
            mutate_context_ref(cli, &args.task, &args.knowledge_id, None, args.if_revision)
        }
        ContextRefCommand::List(args) => {
            let project = storage::resolve_project(cli.project.as_deref(), Some(&args.task))?;
            let _lock = storage::lock_project(&project.path)?;
            let project = storage::reload_project(&project)?;
            crate::transaction::recover(&project)?;
            Ok(json!(
                storage::read_task(&project, &args.task)?.context_refs
            ))
        }
        ContextRefCommand::Reverse { knowledge_id } => {
            let project = storage::resolve_project(cli.project.as_deref(), Some(knowledge_id))?;
            let id = canonical_knowledge_id(&project, knowledge_id)?;
            let _lock = storage::lock_project(&project.path)?;
            let project = storage::reload_project(&project)?;
            crate::transaction::recover(&project)?;
            ensure_target(&project, &id)?;
            let rows: Vec<Value> = storage::read_tasks(&project)?
                .into_iter()
                .filter_map(|task| {
                    task.context_refs
                        .iter()
                        .find(|reference| reference.id == id)
                        .map(|reference| json!({"task_id": task.id, "reference": reference}))
                })
                .collect();
            Ok(Value::Array(rows))
        }
    }
}

fn mutate_context_ref(
    cli: &Cli,
    task_id: &str,
    knowledge_id: &str,
    role: Option<ContextReferenceRole>,
    revision: Option<u64>,
) -> Result<Value> {
    let project = storage::resolve_project(cli.project.as_deref(), Some(task_id))?;
    let id = canonical_knowledge_id(&project, knowledge_id)?;
    let _lock = storage::lock_project(&project.path)?;
    let project = storage::reload_project(&project)?;
    crate::transaction::recover(&project)?;
    ensure_target(&project, &id)?;
    let mut task = storage::read_task(&project, task_id)?;
    task_revision_guard(&task, revision)?;
    let task_path = project.path.join("tasks").join(format!("{task_id}.json"));
    let old_bytes = fs::read(&task_path)
        .map_err(|error| AppError::io(error, "cannot read task before context reference update"))?;
    let action;
    let changed;
    if let Some(role) = role {
        if let Some(existing) = task
            .context_refs
            .iter()
            .find(|reference| reference.id == id)
        {
            if existing.role == role {
                return Ok(json!(task));
            }
            return Err(AppError::new(
                "context_ref_role_conflict",
                "The target is already linked with another role",
                ErrorCategory::Conflict,
            ));
        }
        task.context_refs.push(ContextReference {
            kind: kind_for_id(&id),
            id: id.clone(),
            role,
        });
        action = "task.context_ref.added";
        changed = json!({"context_ref": {"from": null, "to": task.context_refs.last()}});
    } else {
        let index = task
            .context_refs
            .iter()
            .position(|reference| reference.id == id)
            .ok_or_else(|| {
                AppError::new(
                    "context_ref_not_found",
                    "The task does not link that knowledge target",
                    ErrorCategory::NotFound,
                )
            })?;
        let removed = task.context_refs.remove(index);
        action = "task.context_ref.removed";
        changed = json!({"context_ref": {"from": removed, "to": null}});
    }
    let actor = actor_locked(cli, &project, None)?;
    task.revision += 1;
    task.updated_at = now();
    task.updated_by = actor.id.clone();
    task.normalize();
    let new_bytes = json_bytes(&task)?;
    let batch = crate::transaction::MutationBatch::prepare(
        &project,
        action,
        vec![(task_path, Some(old_bytes), new_bytes)],
    )?;
    batch.apply(&project)?;
    let mut changed = changed;
    changed
        .as_object_mut()
        .unwrap()
        .insert("transaction_id".into(), json!(batch.id()));
    if let Err(error) = event(
        &project,
        &actor,
        action,
        Some(&task.id),
        Some(task.revision),
        changed,
    ) {
        batch.rollback(&project)?;
        batch.finish()?;
        return Err(error);
    }
    batch.finish()?;
    Ok(json!(task))
}

pub fn parse_context_reference(
    project: &Project,
    value: &str,
    default_role: Option<ContextReferenceRole>,
) -> Result<ContextReference> {
    let (id, role) = if let Some((id, role)) = value.rsplit_once(':') {
        (id, parse_role(role)?)
    } else {
        (
            value,
            default_role.ok_or_else(|| AppError::input("Context reference must use ID:ROLE"))?,
        )
    };
    let id = canonical_knowledge_id(project, id)?;
    ensure_target(project, &id)?;
    Ok(ContextReference {
        kind: kind_for_id(&id),
        id,
        role,
    })
}

pub fn validate_context_references(
    project: &Project,
    references: &mut [ContextReference],
) -> Result<()> {
    let mut ids = HashSet::new();
    for reference in references.iter_mut() {
        reference.id = canonical_knowledge_id(project, &reference.id)?;
        if reference.kind != kind_for_id(&reference.id) {
            return Err(AppError::new(
                "context_ref_kind_mismatch",
                "Context reference kind disagrees with its ID",
                ErrorCategory::Validation,
            ));
        }
        ensure_target(project, &reference.id)?;
        if !ids.insert(reference.id.clone()) {
            return Err(AppError::new(
                "context_ref_conflict",
                "A task can reference each knowledge target only once",
                ErrorCategory::Conflict,
            ));
        }
    }
    sort_context_refs(references);
    Ok(())
}

fn ensure_target(project: &Project, id: &str) -> Result<()> {
    let exists = if id.contains("-R") {
        super::read_resources(project)?
            .iter()
            .any(|item| item.metadata.id == id)
    } else {
        super::read_decisions(project)?
            .iter()
            .any(|item| item.metadata.id == id)
    };
    if exists {
        Ok(())
    } else {
        Err(not_found("knowledge target", id))
    }
}

fn canonical_knowledge_id(project: &Project, value: &str) -> Result<String> {
    let upper = value.to_ascii_uppercase();
    if upper.contains("-R") {
        super::canonical_id(project, &upper, 'R')
    } else if upper.contains("-D") {
        super::canonical_id(project, &upper, 'D')
    } else {
        Err(AppError::input(
            "Knowledge ID must be project-prefixed -RN or -DN",
        ))
    }
}
fn kind_for_id(id: &str) -> ContextReferenceKind {
    if id.contains("-R") {
        ContextReferenceKind::Resource
    } else {
        ContextReferenceKind::Decision
    }
}
fn parse_role(value: &str) -> Result<ContextReferenceRole> {
    match value.replace('-', "_").to_ascii_lowercase().as_str() {
        "implements" => Ok(ContextReferenceRole::Implements),
        "informed_by" => Ok(ContextReferenceRole::InformedBy),
        "constrained_by" => Ok(ContextReferenceRole::ConstrainedBy),
        "verifies" => Ok(ContextReferenceRole::Verifies),
        _ => Err(AppError::input("Unknown context reference role")),
    }
}

pub fn search_resources(cli: &Cli, args: &SearchResourcesArgs) -> Result<Value> {
    let project = storage::resolve_project(cli.project.as_deref(), None)?;
    let _lock = storage::lock_project(&project.path)?;
    let project = storage::reload_project(&project)?;
    crate::transaction::recover(&project)?;
    resource_results(&project, &args.filters, args.query.as_deref(), true)
}
pub fn search_decisions(cli: &Cli, args: &SearchDecisionsArgs) -> Result<Value> {
    let project = storage::resolve_project(cli.project.as_deref(), None)?;
    let _lock = storage::lock_project(&project.path)?;
    let project = storage::reload_project(&project)?;
    crate::transaction::recover(&project)?;
    decision_results(&project, &args.filters, args.query.as_deref())
}

pub fn project_summary(project: &Project) -> Result<Value> {
    let resources = super::read_resources(project)?;
    let decisions = super::read_decisions(project)?;
    let tasks = storage::read_tasks(project)?;
    let mut by_resource_status = BTreeMap::new();
    let mut by_kind = BTreeMap::new();
    for resource in &resources {
        *by_resource_status
            .entry(format!("{:?}", resource.metadata.status).to_ascii_lowercase())
            .or_insert(0usize) += 1;
        *by_kind
            .entry(format!("{:?}", resource.metadata.kind).to_ascii_lowercase())
            .or_insert(0usize) += 1;
    }
    let mut by_decision_status = BTreeMap::new();
    for decision in &decisions {
        *by_decision_status
            .entry(format!("{:?}", decision.metadata.status).to_ascii_lowercase())
            .or_insert(0usize) += 1;
    }
    let resource_ids: HashSet<&str> = resources
        .iter()
        .map(|item| item.metadata.id.as_str())
        .collect();
    let decision_ids: HashSet<&str> = decisions
        .iter()
        .map(|item| item.metadata.id.as_str())
        .collect();
    let broken_sources = resources
        .iter()
        .filter(|resource| {
            resource
                .metadata
                .source_path
                .as_deref()
                .is_some_and(|path| super::safe_path::read(&project.path, path).is_err())
        })
        .count();
    let broken = broken_sources
        + tasks
            .iter()
            .flat_map(|task| &task.context_refs)
            .filter(|reference| match reference.kind {
                ContextReferenceKind::Resource => !resource_ids.contains(reference.id.as_str()),
                ContextReferenceKind::Decision => !decision_ids.contains(reference.id.as_str()),
            })
            .count();
    Ok(json!({
        "project_brief": {"path": PROJECT_BRIEF_PATH, "exists": project.path.join(PROJECT_BRIEF_PATH).is_file()},
        "knowledge": {
            "editable_paths": {"project_brief": project.path.join(PROJECT_BRIEF_PATH).to_string_lossy(), "resources": project.path.join("resources").to_string_lossy(), "decisions": project.path.join("decisions").to_string_lossy()},
            "resource_counts": {"total": resources.len(), "by_status": by_resource_status, "by_kind": by_kind},
            "decision_counts": {"total": decisions.len(), "by_status": by_decision_status},
            "accepted_project_decision_ids": decisions.iter().filter(|item| item.metadata.status == DecisionStatus::Accepted && item.metadata.scope == DecisionScope::Project).map(|item| item.metadata.id.clone()).collect::<Vec<_>>(),
            "broken_reference_count": broken
        }
    }))
}

pub fn brief(cli: &Cli, args: &BriefArgs) -> Result<Value> {
    let project = storage::resolve_project(cli.project.as_deref(), args.task.as_deref())?;
    let _lock = storage::lock_project(&project.path)?;
    let project = storage::reload_project(&project)?;
    crate::transaction::recover(&project)?;
    let tasks = storage::read_tasks(&project)?;
    let task = args
        .task
        .as_ref()
        .map(|id| {
            tasks
                .iter()
                .find(|task| &task.id == id)
                .cloned()
                .ok_or_else(|| not_found("task", id))
        })
        .transpose()?;
    let resources = super::read_resources(&project)?;
    let decisions = super::read_decisions(&project)?;
    let project_brief = fs::read_to_string(project.path.join(PROJECT_BRIEF_PATH)).ok();
    let mut warnings = Vec::new();
    if project_brief.is_none() {
        warnings.push(json!({"code":"project_brief_missing","path":PROJECT_BRIEF_PATH,"message":"PROJECT.md is missing"}));
    }
    let linked_ids: BTreeSet<String> = task
        .as_ref()
        .into_iter()
        .flat_map(|task| {
            task.context_refs
                .iter()
                .map(|reference| reference.id.clone())
        })
        .collect();
    let mut resource_views = Vec::new();
    for document in &resources {
        let linked = linked_ids.contains(&document.metadata.id);
        if (task.is_some() && !linked)
            || (document.metadata.status == ResourceStatus::Archived && !args.include_archived)
        {
            continue;
        }
        let include_body = if task.is_some() {
            document.metadata.status != ResourceStatus::Archived || args.include_archived
        } else {
            document.metadata.status == ResourceStatus::Active
                && document.metadata.include_in_project_brief
        };
        let mut value = super::resource_value(&project, document, false)?;
        if include_body {
            match if let Some(path) = &document.metadata.source_path {
                super::safe_path::read(&project.path, path)
            } else {
                Ok(document.body.clone())
            } {
                Ok(content) => {
                    value
                        .as_object_mut()
                        .unwrap()
                        .insert("content".into(), json!(content));
                }
                Err(error) => warnings.push(
                    json!({"code":error.code,"id":document.metadata.id,"message":error.message}),
                ),
            }
        }
        resource_views.push(value);
    }
    let mut linked_decisions = Vec::new();
    let mut project_decisions = Vec::new();
    let mut proposed = Vec::new();
    let mut summaries = Vec::new();
    for document in &decisions {
        let summary = super::decision_value(document, false)?;
        summaries.push(summary.clone());
        if linked_ids.contains(&document.metadata.id) {
            linked_decisions.push(super::decision_value(document, true)?);
        } else if document.metadata.status == DecisionStatus::Accepted
            && document.metadata.scope == DecisionScope::Project
        {
            project_decisions.push(super::decision_value(document, true)?);
        } else if document.metadata.status == DecisionStatus::Proposed {
            proposed.push(super::decision_value(document, true)?);
        }
    }
    for id in &linked_ids {
        if !resources.iter().any(|item| &item.metadata.id == id)
            && !decisions.iter().any(|item| &item.metadata.id == id)
        {
            warnings.push(json!({"code":"broken_context_ref","id":id,"message":"Linked knowledge target is missing"}));
        }
    }
    warnings.sort_by_key(|warning| {
        (
            warning["code"].as_str().unwrap_or("").to_string(),
            warning["path"].as_str().unwrap_or("").to_string(),
            warning["id"].as_str().unwrap_or("").to_string(),
            warning["message"].as_str().unwrap_or("").to_string(),
        )
    });
    let task_value = task.as_ref().map(|task| json!({"id":task.id,"header":task.header,"description":task.description,"state":task.state,"assignee":task.assignee,"tags":task.tags,"context_refs":task.context_refs,"revision":task.revision}));
    let task_map: HashMap<String, Task> = tasks
        .iter()
        .cloned()
        .map(|task| (task.id.clone(), task))
        .collect();
    let (dependency_status, dependencies, relations, acceptance, context) = if let Some(task) =
        &task
    {
        let unresolved: Vec<String> = task
            .dependencies
            .iter()
            .filter(|id| {
                task_map.get(*id).is_none_or(|dependency| {
                    project
                        .config
                        .workflow
                        .states
                        .get(&dependency.state)
                        .is_none_or(|state| !state.dependency_satisfied)
                })
            })
            .cloned()
            .collect();
        (Some(json!({"blocked":!unresolved.is_empty(),"unresolved":unresolved})), task.dependencies.iter().filter_map(|id| task_map.get(id)).map(|dependency| json!({"id":dependency.id,"header":dependency.header,"state":dependency.state,"assignee":dependency.assignee})).collect::<Vec<_>>(), json!(task.relations), json!(task.acceptance_criteria), json!(task.context))
    } else {
        (None, Vec::new(), json!([]), json!([]), json!([]))
    };
    let mut value = json!({
        "project":{"id":project.config.prefix,"name":project.config.name,"path":project.path.to_string_lossy(),"configuration":{"initial_state":project.config.workflow.initial_state,"claim_state":project.config.workflow.claim_state,"states":project.config.workflow.states,"relations":project.config.relations}},
        "project_brief":{"path":PROJECT_BRIEF_PATH,"content":project_brief.unwrap_or_default()},
        "task":task_value,"acceptance_criteria":acceptance,"task_context":context,"dependency_status":dependency_status,"dependencies":dependencies,"relations":relations,"resources":resource_views,
        "decisions":{"project":project_decisions,"linked":linked_decisions,"proposed":proposed,"summaries":summaries},"warnings":warnings,
        "truncation":{"truncated":false,"max_bytes":args.max_bytes,"included_bytes":0,"omitted":[]}
    });
    fit_brief(cli, &mut value, args.max_bytes)?;
    Ok(value)
}

fn fit_brief(cli: &Cli, value: &mut Value, max_bytes: usize) -> Result<()> {
    stabilize_size(cli, value);
    if rendered_size(cli, value) <= max_bytes {
        return Ok(());
    }
    let mut omissions = Vec::new();
    let mut paths: Vec<(String, String, String)> = Vec::new();
    // Build the omission list from lowest to highest inclusion priority.
    if let Some(items) = value
        .pointer("/decisions/proposed")
        .and_then(Value::as_array)
    {
        for index in (0..items.len()).rev() {
            if items[index].get("content").is_some() {
                paths.push((
                    format!("/decisions/proposed/{index}/content"),
                    "decisions.proposed".into(),
                    items[index]["id"].as_str().unwrap_or("").into(),
                ));
            }
        }
    }
    if let Some(items) = value["resources"].as_array() {
        for index in (0..items.len()).rev() {
            if items[index]["status"] == "archived" && items[index].get("content").is_some() {
                paths.push((
                    format!("/resources/{index}/content"),
                    "resources".into(),
                    items[index]["id"].as_str().unwrap_or("").into(),
                ));
            }
        }
    }
    if let Some(items) = value.pointer("/decisions/linked").and_then(Value::as_array) {
        for index in (0..items.len()).rev() {
            if items[index]["status"] != "accepted" && items[index].get("content").is_some() {
                paths.push((
                    format!("/decisions/linked/{index}/content"),
                    "decisions.linked".into(),
                    items[index]["id"].as_str().unwrap_or("").into(),
                ));
            }
        }
    }
    paths.push((
        "/project_brief/content".into(),
        "project_brief".into(),
        "PROJECT.md".into(),
    ));
    if let Some(items) = value
        .pointer("/decisions/project")
        .and_then(Value::as_array)
    {
        for index in (0..items.len()).rev() {
            if items[index].get("content").is_some() {
                paths.push((
                    format!("/decisions/project/{index}/content"),
                    "decisions.project".into(),
                    items[index]["id"].as_str().unwrap_or("").into(),
                ));
            }
        }
    }
    if let Some(items) = value["resources"].as_array() {
        for index in (0..items.len()).rev() {
            if items[index]["status"] != "archived" && items[index].get("content").is_some() {
                paths.push((
                    format!("/resources/{index}/content"),
                    "resources".into(),
                    items[index]["id"].as_str().unwrap_or("").into(),
                ));
            }
        }
    }
    if let Some(items) = value.pointer("/decisions/linked").and_then(Value::as_array) {
        for index in (0..items.len()).rev() {
            if items[index]["status"] == "accepted" && items[index].get("content").is_some() {
                paths.push((
                    format!("/decisions/linked/{index}/content"),
                    "decisions.linked".into(),
                    items[index]["id"].as_str().unwrap_or("").into(),
                ));
            }
        }
    }
    for (pointer, section, id) in paths {
        if rendered_size(cli, value) <= max_bytes {
            break;
        }
        if let Some(field) = value.pointer_mut(&pointer)
            && field.as_str().is_some_and(|text| !text.is_empty())
        {
            *field = json!("");
            omissions
                .push(json!({"section":section,"id":id,"field":"content","reason":"max_bytes"}));
        }
    }
    set_omissions(value, &omissions);
    // Drop optional metadata one record at a time, from the lowest inclusion
    // priority upward. Linked accepted decisions and non-archived resource
    // metadata are retained; a limit unable to hold them is too small.
    omit_metadata_while_needed(
        cli,
        value,
        max_bytes,
        "/decisions/summaries",
        "decisions.summaries",
        |_| true,
        &mut omissions,
    );
    omit_metadata_while_needed(
        cli,
        value,
        max_bytes,
        "/decisions/proposed",
        "decisions.proposed",
        |_| true,
        &mut omissions,
    );
    omit_metadata_while_needed(
        cli,
        value,
        max_bytes,
        "/resources",
        "resources",
        |item| item["status"] == "archived",
        &mut omissions,
    );
    omit_metadata_while_needed(
        cli,
        value,
        max_bytes,
        "/decisions/linked",
        "decisions.linked",
        |item| item["status"] != "accepted",
        &mut omissions,
    );
    omit_metadata_while_needed(
        cli,
        value,
        max_bytes,
        "/decisions/project",
        "decisions.project",
        |_| true,
        &mut omissions,
    );
    for (pointer, section) in [
        ("/task_context", "task_context"),
        ("/dependencies", "dependencies"),
        ("/relations", "relations"),
    ] {
        omit_metadata_while_needed(
            cli,
            value,
            max_bytes,
            pointer,
            section,
            |_| true,
            &mut omissions,
        );
    }
    // Mandatory text is shortened only after complete optional records and
    // bodies have been omitted. Character iteration guarantees UTF-8 bounds.
    for (pointer, section, id) in [(
        "/task/description",
        "task",
        value["task"]["id"].as_str().unwrap_or("").to_string(),
    )] {
        while rendered_size(cli, value) > max_bytes {
            let Some(text) = value.pointer(pointer).and_then(Value::as_str) else {
                break;
            };
            if text.is_empty() {
                break;
            }
            let character_count = text.chars().count();
            let keep = if character_count <= 2 {
                0
            } else {
                (character_count - 1) / 2
            };
            let shortened: String = text.chars().take(keep).collect();
            *value.pointer_mut(pointer).unwrap() = json!(if shortened.is_empty() {
                String::new()
            } else {
                format!("{shortened}…")
            });
            let field = pointer.rsplit('/').next().unwrap_or(pointer);
            if !omissions.iter().any(|omission| {
                omission["section"] == section && omission["id"] == id && omission["field"] == field
            }) {
                omissions
                    .push(json!({"section":section,"id":id,"field":field,"reason":"max_bytes"}));
            }
            value["truncation"]["truncated"] = json!(true);
            value["truncation"]["omitted"] = json!(omissions.clone());
        }
    }
    let criterion_count = value["acceptance_criteria"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0);
    for index in 0..criterion_count {
        let pointer = format!("/acceptance_criteria/{index}/text");
        while rendered_size(cli, value) > max_bytes {
            let Some(text) = value.pointer(&pointer).and_then(Value::as_str) else {
                break;
            };
            if text.is_empty() {
                break;
            }
            let count = text.chars().count();
            let keep = if count <= 2 { 0 } else { (count - 1) / 2 };
            let shortened: String = text.chars().take(keep).collect();
            *value.pointer_mut(&pointer).unwrap() = json!(if shortened.is_empty() {
                String::new()
            } else {
                format!("{shortened}…")
            });
            let id = value["acceptance_criteria"][index]["id"]
                .as_str()
                .unwrap_or("")
                .to_string();
            if !omissions.iter().any(|omission| {
                omission["section"] == "acceptance_criteria"
                    && omission["id"] == id
                    && omission["field"] == "text"
            }) {
                omissions.push(json!({"section":"acceptance_criteria","id":id,"field":"text","reason":"max_bytes"}));
                value["truncation"]["truncated"] = json!(true);
                value["truncation"]["omitted"] = json!(omissions.clone());
            }
        }
    }
    // Identity is the highest-priority text and is shortened only after the
    // complete task contract and acceptance text have been considered.
    for (pointer, section, id) in [
        (
            "/task/header",
            "task",
            value["task"]["id"].as_str().unwrap_or("").to_string(),
        ),
        (
            "/project/name",
            "project",
            value["project"]["id"].as_str().unwrap_or("").to_string(),
        ),
    ] {
        while rendered_size(cli, value) > max_bytes {
            let Some(text) = value.pointer(pointer).and_then(Value::as_str) else {
                break;
            };
            if text.is_empty() {
                break;
            }
            let count = text.chars().count();
            let keep = if count <= 2 { 0 } else { (count - 1) / 2 };
            let shortened: String = text.chars().take(keep).collect();
            *value.pointer_mut(pointer).unwrap() = json!(if shortened.is_empty() {
                String::new()
            } else {
                format!("{shortened}…")
            });
            let field = pointer.rsplit('/').next().unwrap_or(pointer);
            if !omissions.iter().any(|omission| {
                omission["section"] == section && omission["id"] == id && omission["field"] == field
            }) {
                omissions
                    .push(json!({"section":section,"id":id,"field":field,"reason":"max_bytes"}));
                set_omissions(value, &omissions);
            }
        }
    }
    stabilize_size(cli, value);
    if rendered_size(cli, value) > max_bytes {
        return Err(AppError::new(
            "brief_limit_too_small",
            "The required brief skeleton cannot fit within --max-bytes",
            ErrorCategory::Input,
        )
        .detail("max_bytes", max_bytes));
    }
    Ok(())
}

fn set_omissions(value: &mut Value, omissions: &[Value]) {
    value["truncation"]["truncated"] = json!(!omissions.is_empty());
    value["truncation"]["omitted"] = json!(omissions);
}

fn omit_metadata_while_needed<F>(
    cli: &Cli,
    value: &mut Value,
    max_bytes: usize,
    pointer: &str,
    section: &str,
    predicate: F,
    omissions: &mut Vec<Value>,
) where
    F: Fn(&Value) -> bool,
{
    while rendered_size(cli, value) > max_bytes {
        let index = value
            .pointer(pointer)
            .and_then(Value::as_array)
            .and_then(|items| items.iter().rposition(&predicate));
        let Some(index) = index else {
            break;
        };
        let Some(items) = value.pointer_mut(pointer).and_then(Value::as_array_mut) else {
            break;
        };
        let item = items.remove(index);
        let id = item
            .get("id")
            .or_else(|| item.get("task_id"))
            .or_else(|| item.get("target"))
            .and_then(Value::as_str)
            .unwrap_or("");
        omissions.push(json!({
            "section": section,
            "id": id,
            "field": "metadata",
            "reason": "max_bytes"
        }));
        set_omissions(value, omissions);
    }
}

fn rendered_size(cli: &Cli, value: &Value) -> usize {
    crate::render_command_success(cli, value).len()
}
fn stabilize_size(cli: &Cli, value: &mut Value) {
    for _ in 0..8 {
        let size = rendered_size(cli, value);
        if value["truncation"]["included_bytes"] == json!(size) {
            break;
        }
        value["truncation"]["included_bytes"] = json!(size);
    }
}

fn structured_object(args: &KnowledgeInputArgs) -> Result<Option<Map<String, Value>>> {
    if matches!(args.input, InputFormat::Human) {
        return Ok(None);
    }
    let text = storage::read_input(args.input_file.as_deref())?;
    let value: Value = match args.input {
        InputFormat::Json => serde_json::from_str(&text)
            .map_err(|error| AppError::input(format!("Invalid JSON input: {error}")))?,
        InputFormat::Yaml => serde_yaml_ng::from_str(&text)
            .map_err(|error| AppError::input(format!("Invalid YAML input: {error}")))?,
        InputFormat::Human => unreachable!(),
    };
    value
        .as_object()
        .cloned()
        .map(Some)
        .ok_or_else(|| AppError::input("Structured input must be an object"))
}
fn reject_unknown(object: &Map<String, Value>, allowed: &[&str]) -> Result<()> {
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        Err(AppError::input(format!(
            "Unknown structured input field {key}"
        )))
    } else {
        Ok(())
    }
}
fn string_field(object: &Map<String, Value>, key: &str) -> Result<Option<String>> {
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
fn bool_field(object: &Map<String, Value>, key: &str) -> Result<Option<bool>> {
    object
        .get(key)
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| AppError::input(format!("{key} must be a boolean")))
        })
        .transpose()
}
fn vec_field(object: &Map<String, Value>, key: &str) -> Result<Option<Vec<String>>> {
    object
        .get(key)
        .map(|value| {
            serde_json::from_value(value.clone()).map_err(|error| {
                AppError::input(format!("{key} must be an array of strings: {error}"))
            })
        })
        .transpose()
}
fn enum_field<T: serde::de::DeserializeOwned>(
    object: &Map<String, Value>,
    key: &str,
) -> Result<Option<T>> {
    object
        .get(key)
        .map(|value| {
            serde_json::from_value(value.clone())
                .map_err(|error| AppError::input(format!("invalid {key}: {error}")))
        })
        .transpose()
}
fn required_body(content: Option<&str>, file: Option<&str>) -> Result<String> {
    if let Some(content) = content {
        Ok(content.into())
    } else if let Some(file) = file {
        read_utf8(file)
    } else {
        piped_stdin().map_err(|_| {
            AppError::input("Markdown content is required through --content, --file, or stdin")
        })
    }
}
fn piped_stdin() -> Result<String> {
    if std::io::stdin().is_terminal() {
        return Err(AppError::input("stdin is a terminal"));
    }
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .map_err(|error| AppError::io(error, "cannot read stdin"))?;
    Ok(text)
}
fn read_utf8(path: &str) -> Result<String> {
    fs::read_to_string(storage::expand_path(path))
        .map_err(|error| AppError::io(error, format!("cannot read Markdown file {path}")))
}
fn nonempty(value: Option<String>, message: &str) -> Result<String> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string())
        .ok_or_else(|| AppError::input(message))
}
fn validate_allocator(meta: &Meta, existing: &[u64], resource: bool) -> Result<()> {
    let next = if resource {
        meta.next_resource_number
    } else {
        meta.next_decision_number
    };
    let max = existing.iter().copied().max().unwrap_or(0);
    if meta.schema_version != crate::model::SCHEMA_VERSION || next == 0 || next <= max {
        Err(AppError::new(
            "invalid_knowledge_allocator",
            "Knowledge allocator must exceed every allocated ID",
            ErrorCategory::Validation,
        )
        .detail("next_number", next)
        .detail("max_number", max))
    } else {
        Ok(())
    }
}
fn revision_guard(id: &str, current: u64, expected: Option<u64>) -> Result<()> {
    if let Some(expected) = expected
        && expected != current
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
fn task_revision_guard(task: &Task, expected: Option<u64>) -> Result<()> {
    revision_guard(&task.id, task.revision, expected)
}
fn not_found(kind: &str, id: &str) -> AppError {
    AppError::new(
        format!("{}_not_found", kind.replace(' ', "_")),
        format!("{kind} {id} does not exist"),
        ErrorCategory::NotFound,
    )
    .detail("id", id.to_string())
}
fn relative(project: &Project, path: &std::path::Path) -> String {
    path.strip_prefix(&project.path)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
fn write_managed_document(
    project: &Project,
    directory: &str,
    path: &std::path::Path,
    bytes: &[u8],
) -> Result<()> {
    super::validate_managed_file(project, directory, path, false)?;
    storage::atomic_write(path, bytes)
}
fn restore_file(path: &std::path::Path, before: Option<&[u8]>) -> Result<()> {
    if let Some(bytes) = before {
        storage::atomic_write(path, bytes)
    } else {
        fs::remove_file(path)
            .or_else(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(error)
                }
            })
            .map_err(|error| AppError::io(error, format!("cannot roll back {}", path.display())))
    }
}
fn terms(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}
fn json_bytes<T: serde::Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| AppError::input(format!("cannot serialize JSON: {error}")))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn knowledge_changes(
    entity_key: &str,
    id: &str,
    path: &std::path::Path,
    project: &Project,
    revision: u64,
    before: &[u8],
    after: &[u8],
) -> Value {
    json!({entity_key:id,"path":relative(project,path),"revision":revision,"before_sha256":super::sha256(before),"after_sha256":super::sha256(after),"before_bytes":before.len(),"after_bytes":after.len(),"transaction_id":Uuid::new_v4().to_string()})
}
