pub mod commands;
pub mod frontmatter;
pub mod model;
pub mod safe_path;

use crate::error::{AppError, ErrorCategory, Result};
use crate::storage::Project;
use chrono::DateTime;
use model::{DecisionMeta, MarkdownDocument, ResourceMeta, numeric_id};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const PROJECT_BRIEF_PATH: &str = "PROJECT.md";

pub fn project_template(name: &str) -> String {
    format!(
        "# {name}\n\n## Purpose\n\nDescribe why this project exists.\n\n## Goals\n\n- Define the outcomes the project should achieve.\n\n## Non-goals\n\n- Define what is deliberately outside the project.\n\n## Constraints\n\n- Record technical, product, operational, or compatibility constraints.\n\n## Architecture\n\nDescribe the current high-level design.\n\n## Current direction\n\nDescribe the currently intended implementation direction.\n\n## Open questions\n\n- Record important unresolved questions.\n"
    )
}

pub struct KnowledgeScan<T> {
    pub records: Vec<MarkdownDocument<T>>,
    pub errors: Vec<KnowledgeScanError>,
}

pub struct KnowledgeScanError {
    pub path: PathBuf,
    pub error: AppError,
}

pub fn create_managed_directory(project: &Project, name: &str) -> Result<PathBuf> {
    if !matches!(name, "resources" | "decisions") {
        return Err(AppError::input("Unknown managed knowledge directory"));
    }
    let path = project.path.join(name);
    match fs::create_dir(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(AppError::io(
                error,
                format!("cannot create project {name} directory"),
            ));
        }
    }
    managed_directory(project, name)?
        .ok_or_else(|| unsafe_managed_path(&path, "managed knowledge directory was not created"))
}

pub fn validate_managed_file(
    project: &Project,
    name: &str,
    path: &Path,
    allow_missing: bool,
) -> Result<()> {
    let root = managed_directory(project, name)?.ok_or_else(|| {
        unsafe_managed_path(
            &project.path.join(name),
            "managed knowledge directory does not exist",
        )
    })?;
    if path.parent() != Some(root.as_path()) || path.file_name().is_none() {
        return Err(unsafe_managed_path(
            path,
            "managed knowledge file is not directly inside its managed directory",
        ));
    }
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && allow_missing => {
            return Ok(());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(unsafe_managed_path(
                path,
                "managed knowledge file no longer exists",
            ));
        }
        Err(error) => {
            return Err(unsafe_managed_path(
                path,
                &format!("cannot inspect managed knowledge file: {error}"),
            ));
        }
    };
    if !metadata.file_type().is_file() || crate::storage::is_symlink_or_reparse(&metadata) {
        return Err(unsafe_managed_path(
            path,
            "managed knowledge files must be regular non-reparse files",
        ));
    }
    let canonical_root = canonical_managed_root(project, &root)?;
    let canonical_file = fs::canonicalize(path).map_err(|error| {
        unsafe_managed_path(
            path,
            &format!("cannot resolve managed knowledge file: {error}"),
        )
    })?;
    if canonical_file.parent() != Some(canonical_root.as_path()) {
        return Err(unsafe_managed_path(
            path,
            "managed knowledge file resolves outside its managed project directory",
        ));
    }
    Ok(())
}

fn managed_directory(project: &Project, name: &str) -> Result<Option<PathBuf>> {
    if !matches!(name, "resources" | "decisions") {
        return Err(AppError::input("Unknown managed knowledge directory"));
    }
    let path = project.path.join(name);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(unsafe_managed_path(
                &path,
                &format!("cannot inspect managed knowledge directory: {error}"),
            ));
        }
    };
    if !metadata.file_type().is_dir() || crate::storage::is_symlink_or_reparse(&metadata) {
        return Err(unsafe_managed_path(
            &path,
            "managed knowledge roots must be real non-reparse directories",
        ));
    }
    canonical_managed_root(project, &path)?;
    Ok(Some(path))
}

fn canonical_managed_root(project: &Project, path: &Path) -> Result<PathBuf> {
    let canonical_project = fs::canonicalize(&project.path).map_err(|error| {
        unsafe_managed_path(
            &project.path,
            &format!("cannot resolve project directory: {error}"),
        )
    })?;
    let canonical_root = fs::canonicalize(path).map_err(|error| {
        unsafe_managed_path(
            path,
            &format!("cannot resolve managed knowledge directory: {error}"),
        )
    })?;
    if canonical_root.parent() != Some(canonical_project.as_path()) {
        return Err(unsafe_managed_path(
            path,
            "managed knowledge root resolves outside the project",
        ));
    }
    Ok(canonical_root)
}

fn unsafe_managed_path(path: &Path, message: &str) -> AppError {
    AppError::new(
        "unsafe_managed_knowledge_path",
        message,
        ErrorCategory::Validation,
    )
    .detail("path", path.to_string_lossy().to_string())
}

fn markdown_candidates(project: &Project, name: &str) -> Result<Vec<PathBuf>> {
    let Some(dir) = managed_directory(project, name)? else {
        return Ok(Vec::new());
    };
    let mut paths = Vec::new();
    for entry in fs::read_dir(&dir)
        .map_err(|error| AppError::io(error, format!("cannot scan {}", dir.display())))?
    {
        let path = entry
            .map_err(|error| AppError::io(error, "cannot read knowledge directory entry"))?
            .path();
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

pub fn scan_resources(project: &Project) -> Result<KnowledgeScan<ResourceMeta>> {
    let mut records = Vec::new();
    let mut errors = Vec::new();
    let mut ids = HashSet::new();
    for path in markdown_candidates(project, "resources")? {
        let parsed = (|| -> Result<MarkdownDocument<ResourceMeta>> {
            validate_managed_file(project, "resources", &path, false)?;
            let bytes = fs::read(&path)
                .map_err(|error| AppError::io(error, format!("cannot read {}", path.display())))?;
            let (mut metadata, body) = frontmatter::parse::<ResourceMeta>(&bytes, "resource")?;
            normalize_resource(project, &path, &mut metadata)?;
            if !ids.insert(metadata.id.clone()) {
                return Err(AppError::new(
                    "duplicate_resource_id",
                    format!("Duplicate resource ID {}", metadata.id),
                    ErrorCategory::Validation,
                ));
            }
            Ok(MarkdownDocument {
                metadata,
                body,
                path: path.clone(),
            })
        })();
        match parsed {
            Ok(record) => records.push(record),
            Err(error) => errors.push(KnowledgeScanError {
                path: path.clone(),
                error,
            }),
        }
    }
    records.sort_by_key(|record| numeric_id(&record.metadata.id, 'R').unwrap_or(u64::MAX));
    Ok(KnowledgeScan { records, errors })
}

pub fn scan_decisions(project: &Project) -> Result<KnowledgeScan<DecisionMeta>> {
    let mut records = Vec::new();
    let mut errors = Vec::new();
    let mut ids = HashSet::new();
    for path in markdown_candidates(project, "decisions")? {
        let parsed = (|| -> Result<MarkdownDocument<DecisionMeta>> {
            validate_managed_file(project, "decisions", &path, false)?;
            let bytes = fs::read(&path)
                .map_err(|error| AppError::io(error, format!("cannot read {}", path.display())))?;
            let (mut metadata, body) = frontmatter::parse::<DecisionMeta>(&bytes, "decision")?;
            normalize_decision(project, &path, &mut metadata, &body)?;
            if !ids.insert(metadata.id.clone()) {
                return Err(AppError::new(
                    "duplicate_decision_id",
                    format!("Duplicate decision ID {}", metadata.id),
                    ErrorCategory::Validation,
                ));
            }
            Ok(MarkdownDocument {
                metadata,
                body,
                path: path.clone(),
            })
        })();
        match parsed {
            Ok(record) => records.push(record),
            Err(error) => errors.push(KnowledgeScanError {
                path: path.clone(),
                error,
            }),
        }
    }
    records.sort_by_key(|record| numeric_id(&record.metadata.id, 'D').unwrap_or(u64::MAX));
    Ok(KnowledgeScan { records, errors })
}

pub fn read_resources(project: &Project) -> Result<Vec<MarkdownDocument<ResourceMeta>>> {
    let scan = scan_resources(project)?;
    if let Some(error) = scan.errors.into_iter().next() {
        Err(error.error)
    } else {
        Ok(scan.records)
    }
}

pub fn read_decisions(project: &Project) -> Result<Vec<MarkdownDocument<DecisionMeta>>> {
    let scan = scan_decisions(project)?;
    if let Some(error) = scan.errors.into_iter().next() {
        Err(error.error)
    } else {
        Ok(scan.records)
    }
}

pub fn normalize_resource(
    project: &Project,
    path: &Path,
    metadata: &mut ResourceMeta,
) -> Result<()> {
    ensure_filename(project, path, &metadata.id, 'R')?;
    validate_common(
        &metadata.title,
        &metadata.tags,
        &metadata.created_at,
        &metadata.updated_at,
        metadata.revision,
        &metadata.id,
    )?;
    if metadata.created_by.trim().is_empty() || metadata.updated_by.trim().is_empty() {
        return Err(AppError::new(
            "invalid_knowledge_attribution",
            "Resource attribution cannot be empty",
            ErrorCategory::Validation,
        ));
    }
    if let Some(source_path) = &metadata.source_path {
        let normalized = safe_path::normalize(source_path)?;
        if normalized != *source_path {
            return Err(AppError::new(
                "noncanonical_resource_path",
                "Resource source_path must use canonical portable separators",
                ErrorCategory::Validation,
            ));
        }
    }
    Ok(())
}

pub fn normalize_decision(
    project: &Project,
    path: &Path,
    metadata: &mut DecisionMeta,
    body: &str,
) -> Result<()> {
    ensure_filename(project, path, &metadata.id, 'D')?;
    validate_common(
        &metadata.title,
        &metadata.tags,
        &metadata.created_at,
        &metadata.updated_at,
        metadata.revision,
        &metadata.id,
    )?;
    validate_decision_sections(body)?;
    if metadata.created_by.trim().is_empty()
        || metadata.updated_by.trim().is_empty()
        || metadata
            .decided_by
            .as_ref()
            .is_some_and(|actor| actor.trim().is_empty())
        || metadata
            .decided_at
            .as_ref()
            .is_some_and(|timestamp| DateTime::parse_from_rfc3339(timestamp).is_err())
    {
        return Err(AppError::new(
            "invalid_knowledge_attribution",
            "Decision attribution must be non-empty and use RFC3339 timestamps",
            ErrorCategory::Validation,
        ));
    }
    let decided = metadata.decided_at.is_some() && metadata.decided_by.is_some();
    let has_decision_attribution = metadata.decided_at.is_some() || metadata.decided_by.is_some();
    if matches!(
        metadata.status,
        model::DecisionStatus::Proposed | model::DecisionStatus::Rejected
    ) && !metadata.supersedes.is_empty()
    {
        return Err(invalid_decision(
            "Proposed and rejected decisions cannot supersede prior decisions",
        ));
    }
    match metadata.status {
        model::DecisionStatus::Proposed
            if has_decision_attribution || metadata.superseded_by.is_some() =>
        {
            return Err(invalid_decision(
                "A proposed decision cannot have decision or supersession attribution",
            ));
        }
        model::DecisionStatus::Accepted | model::DecisionStatus::Rejected
            if !decided || metadata.superseded_by.is_some() =>
        {
            return Err(invalid_decision(
                "Accepted and rejected decisions require complete decision attribution and no successor",
            ));
        }
        model::DecisionStatus::Superseded if !decided || metadata.superseded_by.is_none() => {
            return Err(invalid_decision(
                "A superseded decision requires decision attribution and superseded_by",
            ));
        }
        _ => {}
    }
    let mut sorted_supersedes = metadata.supersedes.clone();
    sorted_supersedes.sort_by_key(|id| numeric_id(id, 'D').unwrap_or(u64::MAX));
    if metadata.supersedes.iter().collect::<HashSet<_>>().len() != metadata.supersedes.len()
        || metadata.supersedes.iter().any(|id| {
            id == &metadata.id
                || !matches!(canonical_id(project, id, 'D').as_ref(), Ok(canonical) if canonical == id)
        })
        || sorted_supersedes != metadata.supersedes
        || metadata.superseded_by.as_ref().is_some_and(|id| {
            !matches!(canonical_id(project, id, 'D').as_ref(), Ok(canonical) if canonical == id)
        })
    {
        return Err(invalid_decision(
            "supersession IDs must be canonical, unique, numerically sorted, and cannot contain the decision itself",
        ));
    }
    Ok(())
}

fn validate_common(
    title: &str,
    tags: &[String],
    created_at: &str,
    updated_at: &str,
    revision: u64,
    id: &str,
) -> Result<()> {
    if title.trim().is_empty() || revision == 0 {
        return Err(AppError::new(
            "invalid_knowledge_metadata",
            "Knowledge title must be non-empty and revision must be at least 1",
            ErrorCategory::Validation,
        )
        .detail("id", id.to_string()));
    }
    if DateTime::parse_from_rfc3339(created_at).is_err()
        || DateTime::parse_from_rfc3339(updated_at).is_err()
    {
        return Err(AppError::new(
            "invalid_knowledge_timestamp",
            "Knowledge timestamps must be RFC3339",
            ErrorCategory::Validation,
        )
        .detail("id", id.to_string()));
    }
    let mut normalized = tags.to_vec();
    model::normalize_tags(&mut normalized);
    if normalized != tags {
        return Err(AppError::new(
            "noncanonical_tags",
            "Tags must be lowercase, sorted, unique, and non-empty",
            ErrorCategory::Validation,
        )
        .detail("id", id.to_string()));
    }
    Ok(())
}

fn ensure_filename(project: &Project, path: &Path, id: &str, marker: char) -> Result<()> {
    let canonical = canonical_id(project, id, marker)?;
    if canonical != id {
        return Err(AppError::new(
            "noncanonical_knowledge_id",
            format!("ID {id} must be uppercase"),
            ErrorCategory::Validation,
        ));
    }
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let valid_lead = stem == id
        || stem.strip_prefix(&format!("{id}-")).is_some_and(|suffix| {
            !suffix.is_empty()
                && !suffix.starts_with('-')
                && !suffix.ends_with('-')
                && suffix.chars().all(|character| {
                    character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
                })
        });
    if !valid_lead {
        return Err(AppError::new(
            if marker == 'R' {
                "resource_filename_mismatch"
            } else {
                "decision_filename_mismatch"
            },
            format!("Filename {} must begin with {id}", path.display()),
            ErrorCategory::Validation,
        ));
    }
    Ok(())
}

pub fn canonical_id(project: &Project, value: &str, marker: char) -> Result<String> {
    let id = value.to_ascii_uppercase();
    let expected = format!("{}-{marker}", project.config.prefix);
    let number = id
        .strip_prefix(&expected)
        .and_then(|number| number.parse::<u64>().ok());
    if number.is_none_or(|number| number == 0 || format!("{expected}{number}") != id) {
        return Err(AppError::new(
            "knowledge_project_mismatch",
            format!("{value} is not a {}-{marker}N ID", project.config.prefix),
            ErrorCategory::Validation,
        )
        .detail("id", value.to_string())
        .detail("project_prefix", project.config.prefix.clone()));
    }
    Ok(id)
}

pub fn resource_value(
    project: &Project,
    document: &MarkdownDocument<ResourceMeta>,
    full: bool,
) -> Result<Value> {
    let content = if full {
        Some(if let Some(path) = &document.metadata.source_path {
            safe_path::read(&project.path, path)?
        } else {
            document.body.clone()
        })
    } else {
        None
    };
    let mut value = serde_json::to_value(&document.metadata)
        .map_err(|error| AppError::input(format!("cannot serialize resource: {error}")))?;
    let object = value.as_object_mut().unwrap();
    object.insert(
        "source_type".into(),
        json!(if document.metadata.source_path.is_some() {
            "referenced"
        } else {
            "managed"
        }),
    );
    if let Some(content) = content {
        object.insert("content".into(), json!(content));
    }
    Ok(value)
}

pub fn decision_value(document: &MarkdownDocument<DecisionMeta>, full: bool) -> Result<Value> {
    let mut value = serde_json::to_value(&document.metadata)
        .map_err(|error| AppError::input(format!("cannot serialize decision: {error}")))?;
    if full {
        value
            .as_object_mut()
            .unwrap()
            .insert("content".into(), json!(document.body));
    }
    Ok(value)
}

pub fn validate_decision_sections(body: &str) -> Result<()> {
    let required = [
        "context",
        "options considered",
        "decision",
        "rationale",
        "consequences",
    ];
    let mut found = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim_start();
        let hashes = trimmed
            .chars()
            .take_while(|character| *character == '#')
            .count();
        if !(1..=6).contains(&hashes) || !trimmed[hashes..].starts_with(char::is_whitespace) {
            continue;
        }
        let title = trimmed[hashes..]
            .trim()
            .trim_end_matches('#')
            .trim()
            .to_ascii_lowercase();
        if required.contains(&title.as_str()) {
            found.push(title);
        }
    }
    if found != required {
        return Err(invalid_decision(
            "Decision content requires Context, Options considered, Decision, Rationale, and Consequences headings in order",
        ));
    }
    Ok(())
}

fn invalid_decision(message: &str) -> AppError {
    AppError::new("invalid_decision", message, ErrorCategory::Validation)
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn slug(value: &str) -> String {
    let mut output = String::new();
    let mut dash = false;
    for character in value.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            if dash && !output.is_empty() {
                output.push('-');
            }
            output.push(character);
            dash = false;
        } else {
            dash = true;
        }
    }
    if output.is_empty() {
        "knowledge".into()
    } else {
        output
    }
}
