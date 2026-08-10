use crate::error::{AppError, ErrorCategory, Result};
use crate::knowledge::sha256;
use crate::model::Event;
use crate::storage::{self, Project};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

const TRANSACTION_DIR: &str = ".tasker-transactions";
const TRANSACTION_ACTIONS: &[&str] = &[
    "resource.created",
    "decision.created",
    "decision.superseded",
    "task.context_ref.added",
    "task.context_ref.removed",
];

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    transaction_id: String,
    action: String,
    files: Vec<ManifestFile>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFile {
    path: String,
    before_sha256: Option<String>,
    after_sha256: String,
    before_image: Option<String>,
    after_image: String,
}

pub struct MutationBatch {
    root: PathBuf,
    manifest: Manifest,
}

impl MutationBatch {
    pub fn prepare(
        project: &Project,
        action: &str,
        files: Vec<(PathBuf, Option<Vec<u8>>, Vec<u8>)>,
    ) -> Result<Self> {
        if !TRANSACTION_ACTIONS.contains(&action) || files.is_empty() {
            return Err(AppError::input("Unsupported or empty transaction batch"));
        }
        let transaction_id = Uuid::new_v4().to_string();
        let root = project.path.join(TRANSACTION_DIR).join(&transaction_id);
        fs::create_dir_all(&root).map_err(|error| {
            AppError::io(error, format!("cannot stage transaction {transaction_id}"))
        })?;
        let staged = (|| -> Result<Manifest> {
            let mut entries = Vec::new();
            for (index, (path, before, after)) in files.into_iter().enumerate() {
                let relative = relative_path(project, &path)?;
                let before_image = before.as_ref().map(|_| format!("{index}.before"));
                let after_image = format!("{index}.after");
                if let (Some(name), Some(bytes)) = (&before_image, &before) {
                    storage::atomic_write(&root.join(name), bytes)?;
                }
                storage::atomic_write(&root.join(&after_image), &after)?;
                entries.push(ManifestFile {
                    path: relative,
                    before_sha256: before.as_deref().map(sha256),
                    after_sha256: sha256(&after),
                    before_image,
                    after_image,
                });
            }
            let manifest = Manifest {
                transaction_id,
                action: action.to_string(),
                files: entries,
            };
            storage::write_json(&root.join("manifest.json"), &manifest)?;
            Ok(manifest)
        })();
        match staged {
            Ok(manifest) => Ok(Self { root, manifest }),
            Err(error) => {
                cleanup_root(&root)?;
                Err(error)
            }
        }
    }

    pub fn id(&self) -> &str {
        &self.manifest.transaction_id
    }

    pub fn apply(&self, project: &Project) -> Result<()> {
        if let Err(error) = validate_manifest(project, &self.root, &self.manifest) {
            cleanup_root(&self.root)?;
            return Err(error);
        }
        if let Err(apply_error) = apply_images(project, &self.root, &self.manifest, true) {
            if let Err(rollback_error) = apply_images(project, &self.root, &self.manifest, false) {
                return Err(recovery_conflict(format!(
                    "Transaction apply failed ({}) and rollback failed ({}); recovery artifacts were retained",
                    apply_error.message, rollback_error.message
                ))
                .detail("transaction_id", self.id().to_string()));
            }
            cleanup_root(&self.root)?;
            return Err(apply_error);
        }
        Ok(())
    }

    pub fn rollback(&self, project: &Project) -> Result<()> {
        validate_manifest(project, &self.root, &self.manifest)?;
        apply_images(project, &self.root, &self.manifest, false)
    }

    pub fn finish(self) -> Result<()> {
        cleanup_root(&self.root)
    }
}

pub fn recover(project: &Project) -> Result<()> {
    let transactions = project.path.join(TRANSACTION_DIR);
    let roots = pending_roots(&transactions)?;
    if roots.is_empty() {
        remove_empty_transaction_dir(&transactions);
        return Ok(());
    }
    let events = storage::read_events(project)?;
    for root in roots {
        let manifest = read_manifest(&root)?;
        validate_manifest(project, &root, &manifest)?;
        let committed = events.iter().any(|event| {
            event.action == manifest.action
                && transaction_id(event) == Some(manifest.transaction_id.as_str())
        });
        verify_current(project, &manifest)?;
        apply_images(project, &root, &manifest, committed)?;
        cleanup_root(&root)?;
    }
    remove_empty_transaction_dir(&transactions);
    Ok(())
}

/// Validate pending recovery state without changing it. Used only by `validate`.
pub fn inspect_pending(project: &Project) -> Result<()> {
    let transactions = project.path.join(TRANSACTION_DIR);
    let roots = pending_roots(&transactions)?;
    if roots.is_empty() {
        return Ok(());
    }
    for root in roots {
        let manifest = read_manifest(&root)?;
        validate_manifest(project, &root, &manifest)?;
        verify_current(project, &manifest)?;
        verify_images(&root, &manifest)?;
    }
    Ok(())
}

fn transaction_id(event: &Event) -> Option<&str> {
    event.changes.get("transaction_id")?.as_str()
}

fn pending_roots(transactions: &Path) -> Result<Vec<PathBuf>> {
    let metadata = match fs::symlink_metadata(transactions) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(recovery_conflict(format!(
                "Cannot inspect pending transaction directory: {error}"
            )));
        }
    };
    if !metadata.file_type().is_dir() || storage::is_symlink_or_reparse(&metadata) {
        return Err(recovery_conflict(
            "Pending transaction path must be a real project directory",
        ));
    }
    let mut roots = Vec::new();
    for entry in fs::read_dir(transactions).map_err(|error| {
        recovery_conflict(format!("Cannot inspect pending transactions: {error}"))
    })? {
        let entry = entry.map_err(|error| {
            recovery_conflict(format!("Cannot inspect pending transaction entry: {error}"))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            recovery_conflict(format!("Cannot inspect pending transaction entry: {error}"))
        })?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
            recovery_conflict(format!("Cannot inspect pending transaction entry: {error}"))
        })?;
        if !file_type.is_dir()
            || file_type.is_symlink()
            || storage::is_symlink_or_reparse(&metadata)
        {
            return Err(recovery_conflict(
                "Pending transaction entries must be real directories",
            ));
        }
        roots.push(entry.path());
    }
    roots.sort();
    Ok(roots)
}

fn read_manifest(root: &Path) -> Result<Manifest> {
    let path = root.join("manifest.json");
    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        recovery_conflict(format!(
            "Cannot inspect pending transaction manifest {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.file_type().is_file() || storage::is_symlink_or_reparse(&metadata) {
        return Err(recovery_conflict(
            "Pending transaction manifest must be a regular file",
        ));
    }
    storage::read_json(&path).map_err(|error| {
        recovery_conflict(format!(
            "Cannot read pending transaction manifest: {}",
            error.message
        ))
    })
}

fn validate_manifest(project: &Project, root: &Path, manifest: &Manifest) -> Result<()> {
    let root_id = root.file_name().and_then(|name| name.to_str());
    if Uuid::parse_str(&manifest.transaction_id).is_err()
        || root_id != Some(manifest.transaction_id.as_str())
        || !TRANSACTION_ACTIONS.contains(&manifest.action.as_str())
        || manifest.files.is_empty()
    {
        return Err(recovery_conflict(
            "Pending transaction manifest identity or action is invalid",
        ));
    }
    let mut paths = HashSet::new();
    for (index, file) in manifest.files.iter().enumerate() {
        if !valid_hash(&file.after_sha256)
            || file
                .before_sha256
                .as_ref()
                .is_some_and(|hash| !valid_hash(hash))
            || file.before_sha256.is_some() != file.before_image.is_some()
            || file.after_image != format!("{index}.after")
            || file
                .before_image
                .as_ref()
                .is_some_and(|name| name != &format!("{index}.before"))
            || !paths.insert(file.path.as_str())
        {
            return Err(recovery_conflict(
                "Pending transaction manifest file metadata is inconsistent",
            )
            .detail("transaction_id", manifest.transaction_id.clone()));
        }
        target_path(project, &file.path)?;
        image_path(root, &file.after_image)?;
        if let Some(image) = &file.before_image {
            image_path(root, image)?;
        }
    }
    Ok(())
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn verify_current(project: &Project, manifest: &Manifest) -> Result<()> {
    for file in &manifest.files {
        let path = target_path(project, &file.path)?;
        let current = match fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(recovery_conflict(format!(
                    "Cannot inspect {} during recovery: {error}",
                    path.display()
                )));
            }
        };
        let hash = current.as_deref().map(sha256);
        if hash.as_ref() != file.before_sha256.as_ref()
            && hash.as_deref() != Some(&file.after_sha256)
        {
            return Err(recovery_conflict(format!(
                "{} was manually changed during a pending transaction",
                path.display()
            ))
            .detail("path", path.to_string_lossy().to_string())
            .detail("transaction_id", manifest.transaction_id.clone()));
        }
    }
    Ok(())
}

fn verify_images(root: &Path, manifest: &Manifest) -> Result<()> {
    for file in &manifest.files {
        verify_image(root, &file.after_image, &file.after_sha256, manifest)?;
        if let (Some(image), Some(hash)) = (&file.before_image, &file.before_sha256) {
            verify_image(root, image, hash, manifest)?;
        }
    }
    Ok(())
}

fn verify_image(root: &Path, name: &str, expected: &str, manifest: &Manifest) -> Result<Vec<u8>> {
    let image = image_path(root, name)?;
    let bytes = fs::read(&image).map_err(|error| {
        recovery_conflict(format!(
            "Cannot read transaction image {}: {error}",
            image.display()
        ))
    })?;
    if sha256(&bytes) != expected {
        return Err(recovery_conflict("A transaction image hash is invalid")
            .detail("transaction_id", manifest.transaction_id.clone()));
    }
    Ok(bytes)
}

fn apply_images(project: &Project, root: &Path, manifest: &Manifest, after: bool) -> Result<()> {
    for file in &manifest.files {
        let target = target_path(project, &file.path)?;
        if !after && file.before_image.is_none() {
            match fs::remove_file(&target) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(AppError::io(
                        error,
                        "cannot remove rolled-back created file",
                    ));
                }
            }
            continue;
        }
        let (image_name, expected) = if after {
            (file.after_image.as_str(), file.after_sha256.as_str())
        } else {
            match (&file.before_image, &file.before_sha256) {
                (Some(image), Some(hash)) => (image.as_str(), hash.as_str()),
                _ => {
                    return Err(recovery_conflict(
                        "Pending transaction rollback metadata is inconsistent",
                    ));
                }
            }
        };
        let bytes = verify_image(root, image_name, expected, manifest)?;
        storage::atomic_write(&target, &bytes)?;
    }
    Ok(())
}

fn image_path(root: &Path, name: &str) -> Result<PathBuf> {
    if name.is_empty()
        || Path::new(name).components().count() != 1
        || !matches!(
            Path::new(name).components().next(),
            Some(Component::Normal(_))
        )
    {
        return Err(recovery_conflict(
            "Transaction image path must be a single file name",
        ));
    }
    let path = root.join(name);
    let metadata = fs::symlink_metadata(&path).map_err(|error| {
        recovery_conflict(format!(
            "Cannot inspect transaction image {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.file_type().is_file() || storage::is_symlink_or_reparse(&metadata) {
        return Err(recovery_conflict(
            "Transaction image must be a regular staged file",
        ));
    }
    Ok(path)
}

fn target_path(project: &Project, value: &str) -> Result<PathBuf> {
    if value.is_empty()
        || value.contains('\\')
        || value.contains('\0')
        || Path::new(value)
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(recovery_conflict(
            "Transaction target path is not a canonical project-relative path",
        ));
    }
    let components: Vec<&str> = value.split('/').collect();
    let allowed = components.as_slice() == ["meta.json"]
        || matches!(components.as_slice(), ["tasks", name] if name.ends_with(".json"))
        || matches!(components.as_slice(), ["resources", name] if name.ends_with(".md"))
        || matches!(components.as_slice(), ["decisions", name] if name.ends_with(".md"));
    if !allowed {
        return Err(recovery_conflict(
            "Transaction target is not a managed mutable file",
        ));
    }
    let target = project.path.join(value);
    if let [directory @ ("resources" | "decisions"), _] = components.as_slice() {
        crate::knowledge::validate_managed_file(project, directory, &target, true).map_err(
            |error| {
                recovery_conflict(format!(
                    "Transaction target is not safely contained: {}",
                    error.message
                ))
                .detail("path", target.to_string_lossy().to_string())
            },
        )?;
    }
    let project_root = fs::canonicalize(&project.path).map_err(|error| {
        recovery_conflict(format!("Cannot resolve project during recovery: {error}"))
    })?;
    let resolved = if target.exists() {
        fs::canonicalize(&target).map_err(|error| {
            recovery_conflict(format!("Cannot resolve transaction target: {error}"))
        })?
    } else {
        let parent = target.parent().ok_or_else(|| {
            recovery_conflict("Transaction target does not have a project parent")
        })?;
        let parent = fs::canonicalize(parent).map_err(|error| {
            recovery_conflict(format!("Cannot resolve transaction target parent: {error}"))
        })?;
        parent.join(
            target
                .file_name()
                .ok_or_else(|| recovery_conflict("Transaction target does not have a file name"))?,
        )
    };
    if !resolved.starts_with(&project_root) {
        return Err(recovery_conflict(
            "Transaction target resolves outside the project",
        ));
    }
    Ok(target)
}

fn relative_path(project: &Project, path: &Path) -> Result<String> {
    let relative = path
        .strip_prefix(&project.path)
        .map_err(|_| AppError::input("Transaction target must be inside the project"))?;
    let value = relative.to_string_lossy().replace('\\', "/");
    target_path(project, &value)?;
    Ok(value)
}

fn cleanup_root(root: &Path) -> Result<()> {
    match fs::remove_dir_all(root) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(AppError::io(
                error,
                format!("cannot remove transaction {}", root.display()),
            ));
        }
    }
    if let Some(parent) = root.parent() {
        remove_empty_transaction_dir(parent);
    }
    Ok(())
}

fn remove_empty_transaction_dir(path: &Path) {
    if fs::read_dir(path)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(false)
    {
        let _ = fs::remove_dir(path);
    }
}

fn recovery_conflict(message: impl Into<String>) -> AppError {
    AppError::new(
        "transaction_recovery_conflict",
        message,
        ErrorCategory::Conflict,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ActorSnapshot, Event, Meta, SCHEMA_VERSION, default_project};
    use serde_json::json;
    use tempfile::TempDir;

    fn fixture() -> (TempDir, Project, PathBuf, PathBuf) {
        let temp = TempDir::new().unwrap();
        for directory in ["changelog", "tasks", "resources", "decisions"] {
            fs::create_dir(temp.path().join(directory)).unwrap();
        }
        fs::write(temp.path().join(".tasker.lock"), b"").unwrap();
        let project = Project {
            path: temp.path().to_path_buf(),
            config: default_project("Demo".into(), "DEM".into()),
        };
        let first = project.path.join("decisions/DEM-D1-first.md");
        let second = project.path.join("decisions/DEM-D2-second.md");
        fs::write(&first, b"before first").unwrap();
        fs::write(&second, b"before second").unwrap();
        (temp, project, first, second)
    }

    #[test]
    fn uncommitted_recovery_rolls_back_and_committed_recovery_completes() {
        let (_temp, project, first, second) = fixture();
        let files = || {
            vec![
                (
                    first.clone(),
                    Some(b"before first".to_vec()),
                    b"after first".to_vec(),
                ),
                (
                    second.clone(),
                    Some(b"before second".to_vec()),
                    b"after second".to_vec(),
                ),
            ]
        };
        let batch = MutationBatch::prepare(&project, "decision.superseded", files()).unwrap();
        batch.apply(&project).unwrap();
        recover(&project).unwrap();
        assert_eq!(fs::read(&first).unwrap(), b"before first");
        assert_eq!(fs::read(&second).unwrap(), b"before second");

        let batch = MutationBatch::prepare(&project, "decision.superseded", files()).unwrap();
        batch.apply(&project).unwrap();
        fs::write(&second, b"before second").unwrap();
        storage::write_event(
            &project,
            &Event {
                schema_version: SCHEMA_VERSION,
                timestamp: "2026-08-09T00:00:00.000Z".into(),
                actor: ActorSnapshot {
                    id: "test".into(),
                    name: "Test".into(),
                },
                action: "decision.superseded".into(),
                task_id: None,
                task_revision: None,
                changes: json!({"transaction_id": batch.id()}),
            },
        )
        .unwrap();
        recover(&project).unwrap();
        assert_eq!(fs::read(&first).unwrap(), b"after first");
        assert_eq!(fs::read(&second).unwrap(), b"after second");
    }

    #[test]
    fn uncommitted_created_file_is_removed_during_recovery() {
        let (_temp, project, _first, _second) = fixture();
        let created = project.path.join("resources/DEM-R1-created.md");
        let batch = MutationBatch::prepare(
            &project,
            "resource.created",
            vec![(created.clone(), None, b"created".to_vec())],
        )
        .unwrap();
        batch.apply(&project).unwrap();
        assert!(created.is_file());
        recover(&project).unwrap();
        assert!(!created.exists());
    }

    #[test]
    fn recovery_refuses_to_overwrite_unexpected_manual_edits() {
        let (_temp, project, first, second) = fixture();
        let batch = MutationBatch::prepare(
            &project,
            "decision.superseded",
            vec![(
                first.clone(),
                Some(b"before first".to_vec()),
                b"after first".to_vec(),
            )],
        )
        .unwrap();
        batch.apply(&project).unwrap();
        fs::write(&first, b"manual edit").unwrap();
        let error = recover(&project).unwrap_err();
        assert_eq!(error.code, "transaction_recovery_conflict");
        assert_eq!(fs::read(&first).unwrap(), b"manual edit");
        assert_eq!(fs::read(&second).unwrap(), b"before second");
    }

    #[test]
    fn failed_apply_rolls_back_allocator_and_removes_transaction_artifacts() {
        let (_temp, project, _first, _second) = fixture();
        let meta_path = project.path.join("meta.json");
        let before = serde_json::to_vec_pretty(&Meta {
            schema_version: SCHEMA_VERSION,
            next_task_number: 1,
            next_resource_number: 1,
            next_decision_number: 1,
        })
        .unwrap();
        fs::write(&meta_path, &before).unwrap();
        let created = project.path.join("resources/DEM-R1-created.md");
        let batch = MutationBatch::prepare(
            &project,
            "resource.created",
            vec![
                (meta_path.clone(), Some(before.clone()), b"changed".to_vec()),
                (created.clone(), None, b"created".to_vec()),
            ],
        )
        .unwrap();
        fs::write(batch.root.join("1.after"), b"corrupt").unwrap();
        let error = batch.apply(&project).unwrap_err();
        assert_eq!(error.code, "transaction_recovery_conflict");
        assert_eq!(fs::read(meta_path).unwrap(), before);
        assert!(!created.exists());
        assert!(!project.path.join(TRANSACTION_DIR).exists());
    }

    #[test]
    fn malformed_manifest_paths_and_metadata_are_structured_conflicts() {
        let (_temp, project, _first, _second) = fixture();
        let outside = project
            .path
            .parent()
            .unwrap()
            .join(format!("outside-{}.txt", Uuid::new_v4()));
        fs::write(&outside, b"safe").unwrap();
        let id = Uuid::new_v4().to_string();
        let root = project.path.join(TRANSACTION_DIR).join(&id);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("0.after"), b"owned").unwrap();
        storage::write_json(
            &root.join("manifest.json"),
            &Manifest {
                transaction_id: id,
                action: "resource.created".into(),
                files: vec![ManifestFile {
                    path: format!("../{}", outside.file_name().unwrap().to_string_lossy()),
                    before_sha256: None,
                    after_sha256: sha256(b"owned"),
                    before_image: None,
                    after_image: "0.after".into(),
                }],
            },
        )
        .unwrap();
        let error = recover(&project).unwrap_err();
        assert_eq!(error.code, "transaction_recovery_conflict");
        assert_eq!(fs::read(&outside).unwrap(), b"safe");
        fs::remove_file(&outside).unwrap();

        fs::remove_dir_all(project.path.join(TRANSACTION_DIR)).unwrap();
        let id = Uuid::new_v4().to_string();
        let root = project.path.join(TRANSACTION_DIR).join(&id);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("0.after"), b"owned").unwrap();
        storage::write_json(
            &root.join("manifest.json"),
            &Manifest {
                transaction_id: id,
                action: "resource.created".into(),
                files: vec![ManifestFile {
                    path: "resources/DEM-R9-owned.md".into(),
                    before_sha256: Some(sha256(b"before")),
                    after_sha256: sha256(b"owned"),
                    before_image: None,
                    after_image: "0.after".into(),
                }],
            },
        )
        .unwrap();
        let error = recover(&project).unwrap_err();
        assert_eq!(error.code, "transaction_recovery_conflict");
    }
}
