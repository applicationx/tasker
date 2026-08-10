use fs4::fs_std::FileExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;
use tempfile::TempDir;
use uuid::Uuid;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_tasker")
}
fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(bin())
        .args(args)
        .env("TASKER_ROOT", root)
        .env("TASKER_ACTOR", "knowledge-tester")
        .env("TASKER_OUTPUT", "human")
        .env_remove("TASKER_PROJECT")
        .output()
        .unwrap()
}
fn ok(root: &Path, args: &[&str]) -> Value {
    let output = run(root, args);
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn error(root: &Path, args: &[&str], code: &str) -> Value {
    let output = run(root, args);
    assert!(!output.status.success(), "{args:?} unexpectedly passed");
    let value: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(value["error"]["code"], code, "{args:?}: {value}");
    value
}
fn fixture() -> (TempDir, PathBuf) {
    let temp = TempDir::new().unwrap();
    ok(
        temp.path(),
        &[
            "create",
            "project",
            "Example App",
            "--prefix",
            "APP",
            "-o",
            "json",
        ],
    );
    let project = temp.path().join("example-app");
    (temp, project)
}
fn decision_body() -> &'static str {
    "# Context\nThe filesystem is authoritative.\n\n# Options considered\n\n## Database\nStore hidden state.\n\n# Decision\nScan files.\n\n# Rationale\nManual edits stay visible.\n\n# Consequences\nSearch cost scales with project size.\n"
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn json_bytes(value: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(value).unwrap();
    bytes.push(b'\n');
    bytes
}
fn stage_uncommitted_task_batch(project: &Path, before: &[u8], after: &[u8]) {
    let id = Uuid::new_v4().to_string();
    let root = project.join(".tasker-transactions").join(&id);
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("0.before"), before).unwrap();
    fs::write(root.join("0.after"), after).unwrap();
    let manifest = json!({
        "transaction_id": id,
        "action": "task.context_ref.added",
        "files": [{
            "path": "tasks/APP-1.json",
            "before_sha256": hash(before),
            "after_sha256": hash(after),
            "before_image": "0.before",
            "after_image": "0.after"
        }]
    });
    fs::write(root.join("manifest.json"), json_bytes(&manifest)).unwrap();
    fs::write(project.join("tasks/APP-1.json"), after).unwrap();
}
fn create_decision(root: &Path, title: &str) -> Value {
    ok(
        root,
        &[
            "create",
            "decision",
            title,
            "-p",
            "APP",
            "--content",
            decision_body(),
            "-o",
            "json",
        ],
    )
}

#[cfg(unix)]
fn link_directory(target: &Path, link: &Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[cfg(windows)]
fn link_directory(target: &Path, link: &Path) {
    let output = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "cannot create test junction: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
fn remove_directory_link(link: &Path) {
    fs::remove_file(link).unwrap();
}

#[cfg(windows)]
fn remove_directory_link(link: &Path) {
    fs::remove_dir(link).unwrap();
}

#[test]
fn project_brief_template_paths_mutation_noop_and_history_are_exact() {
    let (temp, project) = fixture();
    let root = temp.path();
    let expected = "# Example App\n\n## Purpose\n\nDescribe why this project exists.\n\n## Goals\n\n- Define the outcomes the project should achieve.\n\n## Non-goals\n\n- Define what is deliberately outside the project.\n\n## Constraints\n\n- Record technical, product, operational, or compatibility constraints.\n\n## Architecture\n\nDescribe the current high-level design.\n\n## Current direction\n\nDescribe the currently intended implementation direction.\n\n## Open questions\n\n- Record important unresolved questions.\n";
    assert_eq!(
        fs::read_to_string(project.join("PROJECT.md")).unwrap(),
        expected
    );
    assert!(project.join("resources").is_dir() && project.join("decisions").is_dir());
    let shown = run(
        root,
        &["project", "brief", "show", "-p", "APP", "-o", "markdown"],
    );
    assert!(shown.status.success());
    assert_eq!(String::from_utf8(shown.stdout).unwrap(), expected);
    let path = ok(
        root,
        &["project", "brief", "path", "-p", "APP", "-o", "json"],
    );
    assert_eq!(path["project_relative_path"], "PROJECT.md");
    let before = ok(root, &["changelog", "-p", "APP", "-o", "json"])
        .as_array()
        .unwrap()
        .len();
    ok(
        root,
        &[
            "project",
            "brief",
            "set",
            "-p",
            "APP",
            "--content",
            "# Changed",
            "-o",
            "json",
        ],
    );
    ok(
        root,
        &[
            "project",
            "brief",
            "set",
            "-p",
            "APP",
            "--content",
            "# Changed",
            "-o",
            "json",
        ],
    );
    let events = ok(
        root,
        &[
            "changelog",
            "-p",
            "APP",
            "--action",
            "project.brief.updated",
            "-o",
            "json",
        ],
    );
    assert_eq!(events.as_array().unwrap().len(), 1);
    assert!(events[0]["changes"]["after_sha256"].is_string());
    assert_eq!(
        ok(root, &["changelog", "-p", "APP", "-o", "json"])
            .as_array()
            .unwrap()
            .len(),
        before + 1
    );
    error(
        root,
        &["project", "brief", "init", "-p", "APP", "-o", "json"],
        "project_brief_exists",
    );
    ok(
        root,
        &[
            "project", "brief", "init", "-p", "APP", "--force", "-o", "json",
        ],
    );
    assert_eq!(
        fs::read_to_string(project.join("PROJECT.md")).unwrap(),
        expected
    );
}

#[test]
fn managed_and_referenced_resources_are_live_revisioned_searchable_and_archived() {
    let (temp, project) = fixture();
    let root = temp.path();
    fs::create_dir(project.join("docs")).unwrap();
    fs::write(
        project.join("docs/architecture.md"),
        "# Architecture\nVersion one\n",
    )
    .unwrap();
    let managed = ok(
        root,
        &[
            "create",
            "resource",
            "Product brief",
            "-p",
            "APP",
            "--kind",
            "brief",
            "--status",
            "active",
            "--content",
            "# Product\nBody",
            "--tag",
            "Product",
            "--include-in-project-brief",
            "-o",
            "json",
        ],
    );
    assert_eq!(managed["id"], "APP-R1");
    assert_eq!(managed["tags"], serde_json::json!(["product"]));
    let referenced = ok(
        root,
        &[
            "create",
            "resource",
            "Architecture",
            "-p",
            "APP",
            "--kind",
            "design",
            "--status",
            "active",
            "--path",
            "docs\\architecture.md",
            "-o",
            "json",
        ],
    );
    assert_eq!(referenced["id"], "APP-R2");
    assert_eq!(referenced["source_path"], "docs/architecture.md");
    assert!(
        referenced["content"]
            .as_str()
            .unwrap()
            .contains("Version one")
    );
    fs::write(
        project.join("docs/architecture.md"),
        "# Architecture\nVersion two live\n",
    )
    .unwrap();
    assert!(
        ok(root, &["get", "resource", "app-r2", "-o", "json"])["content"]
            .as_str()
            .unwrap()
            .contains("two live")
    );
    let file = fs::read_dir(project.join("resources"))
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| entry.file_name().to_string_lossy().starts_with("APP-R1-"))
        .unwrap()
        .path();
    let filename = file.file_name().unwrap().to_owned();
    let updated = ok(
        root,
        &[
            "update",
            "resource",
            "APP-R1",
            "--title",
            "Renamed product",
            "--if-revision",
            "1",
            "-o",
            "json",
        ],
    );
    assert_eq!(updated["revision"], 2);
    assert!(project.join("resources").join(filename).is_file());
    let unchanged = ok(
        root,
        &[
            "update",
            "resource",
            "APP-R1",
            "--title",
            "Renamed product",
            "-o",
            "json",
        ],
    );
    assert_eq!(unchanged["revision"], 2);
    error(
        root,
        &[
            "update",
            "resource",
            "APP-R2",
            "--content",
            "not allowed",
            "-o",
            "json",
        ],
        "resource_source_type_conflict",
    );
    error(
        root,
        &[
            "create",
            "resource",
            "Escape",
            "-p",
            "APP",
            "--kind",
            "reference",
            "--path",
            "../outside.md",
            "-o",
            "json",
        ],
        "unsafe_resource_path",
    );
    let archived = ok(
        root,
        &[
            "archive",
            "resource",
            "APP-R1",
            "--if-revision",
            "2",
            "-o",
            "json",
        ],
    );
    assert_eq!(archived["status"], "archived");
    assert_eq!(archived["revision"], 3);
    assert!(
        ok(root, &["list", "resources", "-p", "APP", "-o", "json"])
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["id"] != "APP-R1")
    );
    assert_eq!(
        ok(
            root,
            &["search", "resources", "Renamed", "-p", "APP", "-o", "json"]
        )[0]["id"],
        "APP-R1"
    );
    let actions: Vec<String> = ok(root, &["changelog", "-p", "APP", "-o", "json"])
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["action"].as_str().unwrap().to_string())
        .collect();
    for action in ["resource.created", "resource.updated", "resource.archived"] {
        assert!(actions.contains(&action.to_string()));
    }
}

#[test]
fn decisions_enforce_sections_lifecycle_immutability_and_atomic_supersession() {
    let (temp, project) = fixture();
    let root = temp.path();
    error(
        root,
        &[
            "create",
            "decision",
            "Bad",
            "-p",
            "APP",
            "--content",
            "# Context\nOnly one section",
            "-o",
            "json",
        ],
        "invalid_decision",
    );
    let first = create_decision(root, "Filesystem authority");
    assert_eq!(first["status"], "proposed");
    assert_eq!(first["scope"], "project");
    let accepted = ok(
        root,
        &[
            "decision",
            "accept",
            "APP-D1",
            "--if-revision",
            "1",
            "-o",
            "json",
        ],
    );
    assert_eq!(accepted["status"], "accepted");
    assert_eq!(accepted["revision"], 2);
    assert!(accepted["decided_at"].is_string());
    error(
        root,
        &[
            "update",
            "decision",
            "APP-D1",
            "--title",
            "Material change",
            "-o",
            "json",
        ],
        "decision_immutable",
    );
    let tagged = ok(
        root,
        &[
            "update",
            "decision",
            "APP-D1",
            "--tag",
            "architecture",
            "-o",
            "json",
        ],
    );
    assert_eq!(tagged["revision"], 3);
    create_decision(root, "Replacement");
    ok(root, &["decision", "accept", "APP-D2", "-o", "json"]);
    let result = ok(
        root,
        &[
            "decision",
            "supersede",
            "APP-D1",
            "--with",
            "APP-D2",
            "--if-revision",
            "3",
            "--with-if-revision",
            "2",
            "-o",
            "json",
        ],
    );
    assert_eq!(result["superseded"]["status"], "superseded");
    assert_eq!(result["superseded"]["superseded_by"], "APP-D2");
    assert_eq!(
        result["replacement"]["supersedes"],
        serde_json::json!(["APP-D1"])
    );
    error(
        root,
        &[
            "decision",
            "supersede",
            "APP-D1",
            "--with",
            "APP-D2",
            "-o",
            "json",
        ],
        "invalid_decision_transition",
    );
    create_decision(root, "Rejected option");
    let rejected = ok(root, &["decision", "reject", "APP-D3", "-o", "json"]);
    assert_eq!(rejected["status"], "rejected");
    assert_eq!(
        ok(
            root,
            &[
                "search",
                "decisions",
                "authoritative",
                "-p",
                "APP",
                "--full",
                "-o",
                "json"
            ]
        )
        .as_array()
        .unwrap()
        .len(),
        3
    );
    assert!(fs::read_dir(project.join("decisions")).unwrap().count() == 3);
    let validation = ok(root, &["validate", "-p", "APP", "-o", "json"]);
    assert_eq!(validation["valid"], true);
}

#[test]
fn context_refs_task_creation_reverse_lookup_and_brief_shapes_are_deterministic() {
    let (temp, _) = fixture();
    let root = temp.path();
    ok(
        root,
        &[
            "create",
            "resource",
            "Requirements",
            "-p",
            "APP",
            "--kind",
            "requirement",
            "--status",
            "active",
            "-o",
            "json",
        ],
    );
    create_decision(root, "Constraint");
    ok(root, &["decision", "accept", "APP-D1", "-o", "json"]);
    let task = ok(
        root,
        &[
            "create",
            "task",
            "Implement knowledge",
            "-p",
            "APP",
            "--description",
            "Build deterministic context",
            "--acceptance-criterion",
            "Brief is bounded",
            "--context-ref",
            "APP-R1:implements",
            "--from",
            "APP-D1",
            "-o",
            "json",
        ],
    );
    assert_eq!(task["context_refs"].as_array().unwrap().len(), 2);
    assert_eq!(task["revision"], 1);
    let no_op = ok(
        root,
        &[
            "context-ref",
            "add",
            "APP-1",
            "APP-R1",
            "--role",
            "implements",
            "-o",
            "json",
        ],
    );
    assert_eq!(no_op["revision"], 1);
    error(
        root,
        &[
            "context-ref",
            "add",
            "APP-1",
            "APP-R1",
            "--role",
            "verifies",
            "-o",
            "json",
        ],
        "context_ref_role_conflict",
    );
    let added = ok(
        root,
        &[
            "context-ref",
            "add",
            "APP-1",
            "APP-D1",
            "--role",
            "informed-by",
            "-o",
            "json",
        ],
    );
    assert_eq!(added["revision"], 1, "same decision role is idempotent");
    let reverse = ok(root, &["context-ref", "reverse", "APP-D1", "-o", "json"]);
    assert_eq!(reverse[0]["task_id"], "APP-1");
    let first = run(
        root,
        &["brief", "APP-1", "--max-bytes", "50000", "-o", "json"],
    );
    let second = run(
        root,
        &["brief", "APP-1", "--max-bytes", "50000", "-o", "json"],
    );
    assert_eq!(first.stdout, second.stdout);
    let brief: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(brief["task"]["id"], "APP-1");
    assert_eq!(brief["dependency_status"]["blocked"], false);
    assert!(
        brief["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "APP-R1" && item["content"].is_string())
    );
    assert!(
        brief["decisions"]["linked"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "APP-D1")
    );
    assert_eq!(
        brief["truncation"]["included_bytes"].as_u64().unwrap() as usize,
        first.stdout.len()
    );
    let markdown = run(root, &["brief", "APP-1", "-o", "markdown"]);
    assert!(markdown.status.success());
    assert!(
        String::from_utf8(markdown.stdout)
            .unwrap()
            .contains("## Task APP-1")
    );
    for format in ["yaml", "json"] {
        let mut args = vec!["brief", "APP-1", "--max-bytes", "50000", "-o", format];
        if format == "json" {
            args.push("--pretty");
        }
        let output = run(root, &args);
        assert!(output.status.success());
        let parsed: Value = if format == "yaml" {
            serde_yaml_ng::from_slice(&output.stdout).unwrap()
        } else {
            serde_json::from_slice(&output.stdout).unwrap()
        };
        assert_eq!(
            parsed["truncation"]["included_bytes"].as_u64().unwrap() as usize,
            output.stdout.len()
        );
    }
    error(
        root,
        &["brief", "APP-1", "--max-bytes", "10", "-o", "json"],
        "brief_limit_too_small",
    );
    let removed = ok(
        root,
        &[
            "context-ref",
            "remove",
            "APP-1",
            "APP-R1",
            "--if-revision",
            "1",
            "-o",
            "json",
        ],
    );
    assert_eq!(removed["revision"], 2);
}

#[test]
fn structured_knowledge_input_and_project_summary_are_machine_friendly() {
    let (temp, _) = fixture();
    let root = temp.path();
    let mut child = Command::new(bin())
        .args([
            "create", "resource", "-p", "APP", "-i", "json", "-o", "json",
        ])
        .env("TASKER_ROOT", root)
        .env("TASKER_ACTOR", "knowledge-tester")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(br##"{"title":"Structured plan","kind":"plan","status":"active","content":"# Plan","tags":["CLI"]}"##).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["id"], "APP-R1");
    assert_eq!(value["tags"], serde_json::json!(["cli"]));
    create_decision(root, "Project choice");
    ok(root, &["decision", "accept", "APP-D1", "-o", "json"]);
    let project = ok(root, &["get", "project", "APP", "-o", "json"]);
    assert_eq!(project["project_brief"]["exists"], true);
    assert_eq!(
        project["knowledge"]["resource_counts"]["by_kind"]["plan"],
        1
    );
    assert_eq!(
        project["knowledge"]["accepted_project_decision_ids"],
        serde_json::json!(["APP-D1"])
    );
    assert_eq!(project["knowledge"]["broken_reference_count"], 0);
    assert!(project["knowledge"].get("content").is_none());
}

#[test]
fn legacy_missing_knowledge_warns_without_writes_and_corrupt_knowledge_fails_validation() {
    let (temp, project) = fixture();
    let root = temp.path();
    fs::remove_file(project.join("PROJECT.md")).unwrap();
    fs::remove_dir(project.join("resources")).unwrap();
    fs::remove_dir(project.join("decisions")).unwrap();
    let validation = ok(root, &["validate", "-p", "APP", "-o", "json"]);
    assert_eq!(validation["valid"], true);
    assert_eq!(
        validation["warnings"][0]["code"],
        "project_knowledge_not_initialized"
    );
    assert!(!project.join("PROJECT.md").exists() && !project.join("resources").exists());
    ok(
        root,
        &["project", "brief", "init", "-p", "APP", "-o", "json"],
    );
    fs::create_dir(project.join("resources")).unwrap();
    fs::create_dir(project.join("decisions")).unwrap();
    fs::write(
        project.join("resources/APP-R1-bad.md"),
        "---\nid: APP-R1\ntitle: Bad\nkind: imaginary\n---\nbody\n",
    )
    .unwrap();
    let output = run(root, &["validate", "-p", "APP", "-o", "json"]);
    assert_eq!(output.status.code(), Some(5));
    let value: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(value["valid"], false);
    assert!(
        value["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["code"] == "invalid_resource")
    );
}

#[test]
fn resource_and_project_brief_error_filter_and_update_branches_are_stable() {
    let (temp, project) = fixture();
    let root = temp.path();
    fs::remove_file(project.join("PROJECT.md")).unwrap();
    error(
        root,
        &["project", "brief", "show", "-p", "APP", "-o", "json"],
        "project_brief_not_found",
    );
    ok(
        root,
        &["project", "brief", "init", "-p", "APP", "-o", "json"],
    );
    assert_eq!(
        ok(
            root,
            &[
                "changelog",
                "-p",
                "APP",
                "--action",
                "project.brief.created",
                "-o",
                "json"
            ]
        )
        .as_array()
        .unwrap()
        .len(),
        1
    );
    error(
        root,
        &[
            "create",
            "resource",
            "Missing kind",
            "-p",
            "APP",
            "-o",
            "json",
        ],
        "invalid_input",
    );
    ok(
        root,
        &[
            "create",
            "resource",
            "Mutable",
            "-p",
            "APP",
            "--kind",
            "plan",
            "--content",
            "# Initial",
            "--tag",
            "one",
            "--include-in-project-brief",
            "-o",
            "json",
        ],
    );
    error(
        root,
        &["update", "resource", "APP-R1", "-o", "json"],
        "invalid_input",
    );
    error(
        root,
        &[
            "update",
            "resource",
            "APP-R1",
            "--title",
            "stale",
            "--if-revision",
            "99",
            "-o",
            "json",
        ],
        "revision_conflict",
    );
    let updated = ok(
        root,
        &[
            "update",
            "resource",
            "APP-R1",
            "--title",
            "Everything",
            "--kind",
            "research",
            "--status",
            "active",
            "--content",
            "# Replaced",
            "--clear-tags",
            "--exclude-from-project-brief",
            "-o",
            "json",
        ],
    );
    assert_eq!(updated["kind"], "research");
    assert_eq!(updated["tags"], serde_json::json!([]));
    assert_eq!(updated["include_in_project_brief"], false);
    error(
        root,
        &[
            "update",
            "resource",
            "APP-R1",
            "--path",
            "docs/x.md",
            "-o",
            "json",
        ],
        "resource_source_type_conflict",
    );
    fs::create_dir(project.join("docs")).unwrap();
    fs::write(project.join("docs/a.md"), "A").unwrap();
    fs::write(project.join("docs/b.md"), "B").unwrap();
    ok(
        root,
        &[
            "create",
            "resource",
            "Reference",
            "-p",
            "APP",
            "--kind",
            "reference",
            "--path",
            "docs/a.md",
            "-o",
            "json",
        ],
    );
    let changed = ok(
        root,
        &[
            "update",
            "resource",
            "APP-R2",
            "--path",
            "docs/b.md",
            "-o",
            "json",
        ],
    );
    assert_eq!(changed["content"], "B");
    let filtered = ok(
        root,
        &[
            "list",
            "resources",
            "-p",
            "APP",
            "--kind",
            "research",
            "--status",
            "active",
            "--tag",
            "missing",
            "--full",
            "--limit",
            "1",
            "--offset",
            "0",
            "-o",
            "json",
        ],
    );
    assert!(filtered.as_array().unwrap().is_empty());
    error(
        root,
        &["get", "resource", "APP-R99", "-o", "json"],
        "resource_not_found",
    );
    let first = ok(root, &["archive", "resource", "APP-R1", "-o", "json"]);
    let second = ok(root, &["archive", "resource", "APP-R1", "-o", "json"]);
    assert_eq!(first["revision"], second["revision"]);
    fs::create_dir(project.join("docs/directory")).unwrap();
    error(
        root,
        &[
            "create",
            "resource",
            "Directory",
            "-p",
            "APP",
            "--kind",
            "reference",
            "--path",
            "docs/directory",
            "-o",
            "json",
        ],
        "referenced_source_not_file",
    );
    fs::write(project.join("docs/binary.md"), [0xff, 0xfe]).unwrap();
    error(
        root,
        &[
            "create",
            "resource",
            "Binary",
            "-p",
            "APP",
            "--kind",
            "reference",
            "--path",
            "docs/binary.md",
            "-o",
            "json",
        ],
        "referenced_source_not_utf8",
    );
    fs::write(project.join("docs/large.md"), vec![b'x'; 1_048_577]).unwrap();
    error(
        root,
        &[
            "create",
            "resource",
            "Large",
            "-p",
            "APP",
            "--kind",
            "reference",
            "--path",
            "docs/large.md",
            "-o",
            "json",
        ],
        "referenced_source_too_large",
    );
}

#[test]
fn decision_context_and_rich_markdown_brief_cover_mutation_and_render_contracts() {
    let (temp, _project) = fixture();
    let root = temp.path();
    create_decision(root, "Mutable decision");
    error(
        root,
        &["update", "decision", "APP-D1", "-o", "json"],
        "invalid_input",
    );
    error(
        root,
        &[
            "update",
            "decision",
            "APP-D1",
            "--title",
            "stale",
            "--if-revision",
            "99",
            "-o",
            "json",
        ],
        "revision_conflict",
    );
    let revised = decision_body().replace("Scan files.", "Scan every current file.");
    let updated = ok(
        root,
        &[
            "update",
            "decision",
            "APP-D1",
            "--title",
            "Revised",
            "--scope",
            "linked",
            "--content",
            &revised,
            "--tag",
            "Storage",
            "-o",
            "json",
        ],
    );
    assert_eq!(updated["scope"], "linked");
    assert_eq!(updated["tags"], serde_json::json!(["storage"]));
    let same = ok(
        root,
        &[
            "update", "decision", "APP-D1", "--title", "Revised", "-o", "json",
        ],
    );
    assert_eq!(same["revision"], updated["revision"]);
    let listed = ok(
        root,
        &[
            "list",
            "decisions",
            "-p",
            "APP",
            "--status",
            "proposed",
            "--scope",
            "linked",
            "--tag",
            "storage",
            "--full",
            "--limit",
            "1",
            "--offset",
            "0",
            "-o",
            "json",
        ],
    );
    assert_eq!(listed[0]["id"], "APP-D1");
    error(
        root,
        &["get", "decision", "APP-D99", "-o", "json"],
        "decision_not_found",
    );
    error(
        root,
        &[
            "decision",
            "supersede",
            "APP-D1",
            "--with",
            "APP-D1",
            "-o",
            "json",
        ],
        "self_supersession",
    );
    ok(root, &["decision", "accept", "APP-D1", "-o", "json"]);
    error(
        root,
        &["decision", "accept", "APP-D1", "-o", "json"],
        "invalid_decision_transition",
    );
    ok(
        root,
        &[
            "create",
            "resource",
            "Linked",
            "-p",
            "APP",
            "--kind",
            "design",
            "--status",
            "active",
            "--content",
            "# Linked resource",
            "-o",
            "json",
        ],
    );
    ok(
        root,
        &["create", "task", "Dependency", "-p", "APP", "-o", "json"],
    );
    ok(
        root,
        &[
            "create",
            "task",
            "Rich task",
            "-p",
            "APP",
            "--description",
            "Rich description",
            "--acceptance-criterion",
            "One check",
            "--context-ref",
            "APP-R1:verifies",
            "--context-ref",
            "APP-D1:constrained-by",
            "-o",
            "json",
        ],
    );
    ok(root, &["dependency", "add", "APP-2", "APP-1", "-o", "json"]);
    ok(
        root,
        &[
            "relation",
            "add",
            "APP-2",
            "relates_to",
            "APP-1",
            "-o",
            "json",
        ],
    );
    ok(
        root,
        &[
            "context",
            "add",
            "APP-2",
            "Useful progress",
            "--kind",
            "progress",
            "-o",
            "json",
        ],
    );
    let markdown = run(root, &["brief", "APP-2", "-o", "markdown"]);
    let text = String::from_utf8(markdown.stdout).unwrap();
    for heading in [
        "### Acceptance criteria",
        "## Dependency status",
        "## Relations",
        "## Task context",
        "## Decisions",
        "## Resources",
    ] {
        assert!(text.contains(heading), "missing {heading}: {text}");
    }
    let huge = format!("# Linked resource\n\n{}", "knowledge ".repeat(1200));
    ok(
        root,
        &[
            "update",
            "resource",
            "APP-R1",
            "--content",
            &huge,
            "-o",
            "json",
        ],
    );
    let bounded = run(
        root,
        &["brief", "APP-2", "--max-bytes", "7000", "-o", "json"],
    );
    assert!(
        bounded.status.success(),
        "{}",
        String::from_utf8_lossy(&bounded.stderr)
    );
    let value: Value = serde_json::from_slice(&bounded.stdout).unwrap();
    assert!(value["truncation"]["truncated"].as_bool().unwrap());
    assert!(value["truncation"]["included_bytes"].as_u64().unwrap() <= 7000);
    assert!(
        value["truncation"]["omitted"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "APP-R1" && item["field"] == "content")
    );
    error(
        root,
        &["context-ref", "remove", "APP-2", "APP-R99", "-o", "json"],
        "knowledge_target_not_found",
    );
    error(
        root,
        &[
            "create",
            "task",
            "Bad role",
            "-p",
            "APP",
            "--context-ref",
            "APP-R1:unknown",
            "-o",
            "json",
        ],
        "invalid_input",
    );
    error(
        root,
        &[
            "create",
            "task",
            "Bad ID",
            "-p",
            "APP",
            "--context-ref",
            "not-knowledge:implements",
            "-o",
            "json",
        ],
        "invalid_input",
    );
}

#[test]
fn competing_context_mutations_and_supersession_serialize_under_the_project_lock() {
    let (temp, _) = fixture();
    let root = temp.path();
    for title in ["One", "Two", "Three"] {
        ok(
            root,
            &[
                "create",
                "resource",
                title,
                "-p",
                "APP",
                "--kind",
                "reference",
                "-o",
                "json",
            ],
        );
    }
    ok(
        root,
        &[
            "create",
            "task",
            "Concurrent refs",
            "-p",
            "APP",
            "-o",
            "json",
        ],
    );
    let spawn_ref = |id: &'static str| {
        Command::new(bin())
            .args([
                "context-ref",
                "add",
                "APP-1",
                id,
                "--role",
                "verifies",
                "--if-revision",
                "1",
                "-o",
                "json",
            ])
            .env("TASKER_ROOT", root)
            .env("TASKER_ACTOR", id)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    };
    let a = spawn_ref("APP-R1").wait_with_output().unwrap();
    let b = spawn_ref("APP-R2").wait_with_output().unwrap();
    assert_eq!(
        usize::from(a.status.success()) + usize::from(b.status.success()),
        1
    );
    let failed = if a.status.success() { b } else { a };
    assert_eq!(
        serde_json::from_slice::<Value>(&failed.stderr).unwrap()["error"]["code"],
        "revision_conflict"
    );
    for title in ["Old", "New"] {
        create_decision(root, title);
    }
    ok(root, &["decision", "accept", "APP-D1", "-o", "json"]);
    ok(root, &["decision", "accept", "APP-D2", "-o", "json"]);
    let spawn = || {
        Command::new(bin())
            .args([
                "decision",
                "supersede",
                "APP-D1",
                "--with",
                "APP-D2",
                "-o",
                "json",
            ])
            .env("TASKER_ROOT", root)
            .env("TASKER_ACTOR", "superseder")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    };
    let a = spawn().wait_with_output().unwrap();
    let b = spawn().wait_with_output().unwrap();
    assert_eq!(
        usize::from(a.status.success()) + usize::from(b.status.success()),
        1
    );
    assert_eq!(
        ok(
            root,
            &[
                "changelog",
                "-p",
                "APP",
                "--action",
                "decision.superseded",
                "-o",
                "json"
            ]
        )
        .as_array()
        .unwrap()
        .len(),
        1
    );
}

#[test]
fn legacy_creation_is_lazy_bare_decisions_are_valid_and_path_content_is_rejected() {
    let (temp, project) = fixture();
    let root = temp.path();
    fs::remove_dir(project.join("resources")).unwrap();
    fs::remove_dir(project.join("decisions")).unwrap();

    let resource = ok(
        root,
        &[
            "create",
            "resource",
            "First resource",
            "-p",
            "APP",
            "--kind",
            "research",
            "-o",
            "json",
        ],
    );
    let decision = ok(
        root,
        &[
            "create",
            "decision",
            "Keep files authoritative",
            "-p",
            "APP",
            "-o",
            "json",
        ],
    );
    assert_eq!(resource["id"], "APP-R1");
    assert_eq!(decision["id"], "APP-D1");
    for heading in [
        "# Context",
        "# Options considered",
        "# Decision",
        "# Rationale",
        "# Consequences",
    ] {
        assert!(decision["content"].as_str().unwrap().contains(heading));
    }
    assert!(project.join("resources").is_dir());
    assert!(project.join("decisions").is_dir());
    assert!(!project.join(".tasker-transactions").exists());
    let meta: Value =
        serde_json::from_slice(&fs::read(project.join("meta.json")).unwrap()).unwrap();
    assert_eq!(meta["next_resource_number"], 2);
    assert_eq!(meta["next_decision_number"], 2);

    fs::create_dir(project.join("docs")).unwrap();
    fs::write(project.join("docs/source.md"), "live").unwrap();
    error(
        root,
        &[
            "create",
            "resource",
            "Ambiguous",
            "-p",
            "APP",
            "--kind",
            "design",
            "--path",
            "docs/source.md",
            "--content",
            "managed",
            "-o",
            "json",
        ],
        "invalid_input",
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(project.join("meta.json")).unwrap()).unwrap()["next_resource_number"],
        2
    );
}

#[test]
fn brief_init_rechecks_under_lock_and_show_preserves_authored_bytes() {
    let (temp, project) = fixture();
    let root = temp.path();
    let exact = b"# Manual\n\n\n";
    fs::write(project.join("PROJECT.md"), exact).unwrap();
    let shown = run(
        root,
        &["project", "brief", "show", "-p", "APP", "-o", "markdown"],
    );
    assert!(shown.status.success());
    assert_eq!(shown.stdout, exact);

    fs::remove_file(project.join("PROJECT.md")).unwrap();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(project.join(".tasker.lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();
    let mut child = Command::new(bin())
        .args(["project", "brief", "init", "-p", "APP", "-o", "json"])
        .env("TASKER_ROOT", root)
        .env("TASKER_ACTOR", "knowledge-tester")
        .env("TASKER_OUTPUT", "human")
        .env_remove("TASKER_PROJECT")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(child.try_wait().unwrap().is_none());
    fs::write(project.join("PROJECT.md"), exact).unwrap();
    FileExt::unlock(&lock).unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(4));
    let value: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(value["error"]["code"], "project_brief_exists");
    assert_eq!(fs::read(project.join("PROJECT.md")).unwrap(), exact);
}

#[test]
fn validation_reads_project_brief_and_forbids_partial_proposed_attribution() {
    let (temp, project) = fixture();
    let root = temp.path();

    fs::remove_file(project.join("PROJECT.md")).unwrap();
    fs::create_dir(project.join("PROJECT.md")).unwrap();
    let output = run(root, &["validate", "-p", "APP", "-o", "json"]);
    assert_eq!(output.status.code(), Some(5));
    let value: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert!(
        value["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["code"] == "project_brief_not_file")
    );

    fs::remove_dir(project.join("PROJECT.md")).unwrap();
    fs::write(project.join("PROJECT.md"), [0xff, 0xfe]).unwrap();
    let output = run(root, &["validate", "-p", "APP", "-o", "json"]);
    assert_eq!(output.status.code(), Some(5));
    let value: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert!(
        value["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["code"] == "project_brief_not_utf8")
    );

    fs::write(project.join("PROJECT.md"), "# Valid\n").unwrap();
    create_decision(root, "Partial attribution");
    let decision_path = fs::read_dir(project.join("decisions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let original = fs::read_to_string(&decision_path).unwrap();
    let changed = original.replace("decided_at: null", "decided_at: 2026-08-09T00:00:00.000Z");
    assert_ne!(changed, original);
    fs::write(decision_path, changed).unwrap();
    let output = run(root, &["validate", "-p", "APP", "-o", "json"]);
    assert_eq!(output.status.code(), Some(5));
    let value: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert!(
        value["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["code"] == "invalid_decision")
    );
}

#[test]
fn managed_knowledge_roots_cannot_escape_validation_or_mutation() {
    let (temp, project) = fixture();
    let root = temp.path();
    ok(
        root,
        &[
            "create",
            "resource",
            "Contained resource",
            "-p",
            "APP",
            "--kind",
            "design",
            "--content",
            "# Safe",
            "-o",
            "json",
        ],
    );
    create_decision(root, "Contained decision");

    let outside_resources = root.join("outside-resources");
    let outside_decisions = root.join("outside-decisions");
    fs::rename(project.join("resources"), &outside_resources).unwrap();
    fs::rename(project.join("decisions"), &outside_decisions).unwrap();
    link_directory(&outside_resources, &project.join("resources"));
    link_directory(&outside_decisions, &project.join("decisions"));
    let resource_path = fs::read_dir(&outside_resources)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let decision_path = fs::read_dir(&outside_decisions)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let resource_before = fs::read(&resource_path).unwrap();
    let decision_before = fs::read(&decision_path).unwrap();

    let output = run(root, &["validate", "-p", "APP", "-o", "json"]);
    assert_eq!(output.status.code(), Some(5));
    let result: Value = serde_json::from_slice(&output.stderr).unwrap();
    let unsafe_paths: Vec<&str> = result["errors"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["code"] == "unsafe_managed_knowledge_path")
        .filter_map(|item| item["path"].as_str())
        .collect();
    assert!(unsafe_paths.iter().any(|path| path.ends_with("resources")));
    assert!(unsafe_paths.iter().any(|path| path.ends_with("decisions")));

    error(
        root,
        &[
            "update",
            "resource",
            "APP-R1",
            "--content",
            "# Escaped",
            "-o",
            "json",
        ],
        "unsafe_managed_knowledge_path",
    );
    error(
        root,
        &[
            "update", "decision", "APP-D1", "--tag", "escaped", "-o", "json",
        ],
        "unsafe_managed_knowledge_path",
    );
    assert_eq!(fs::read(&resource_path).unwrap(), resource_before);
    assert_eq!(fs::read(&decision_path).unwrap(), decision_before);

    remove_directory_link(&project.join("resources"));
    remove_directory_link(&project.join("decisions"));
}

#[test]
fn validation_inspects_empty_transaction_path_metadata() {
    let (temp, project) = fixture();
    let root = temp.path();
    let outside = root.join("outside-transactions");
    fs::create_dir(&outside).unwrap();
    let transactions = project.join(".tasker-transactions");
    link_directory(&outside, &transactions);

    let output = run(root, &["validate", "-p", "APP", "-o", "json"]);
    assert_eq!(output.status.code(), Some(5));
    let result: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert!(result["errors"].as_array().unwrap().iter().any(|item| {
        item["code"] == "transaction_recovery_conflict"
            && item["path"]
                .as_str()
                .is_some_and(|path| path.ends_with(".tasker-transactions"))
    }));

    remove_directory_link(&transactions);
    fs::write(&transactions, b"not a directory").unwrap();
    let output = run(root, &["validate", "-p", "APP", "-o", "json"]);
    assert_eq!(output.status.code(), Some(5));
    let result: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert!(
        result["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["code"] == "transaction_recovery_conflict")
    );
}

#[test]
fn proposed_and_rejected_decisions_cannot_supersede_predecessors() {
    let (temp, project) = fixture();
    let root = temp.path();
    for title in ["Predecessor", "Proposed successor", "Rejected successor"] {
        create_decision(root, title);
    }
    ok(root, &["decision", "accept", "APP-D1", "-o", "json"]);
    ok(root, &["decision", "reject", "APP-D3", "-o", "json"]);
    let decision_path = |prefix: &str| {
        fs::read_dir(project.join("decisions"))
            .unwrap()
            .filter_map(Result::ok)
            .find(|entry| entry.file_name().to_string_lossy().starts_with(prefix))
            .unwrap()
            .path()
    };
    for id in ["APP-D2-", "APP-D3-"] {
        let path = decision_path(id);
        let original = fs::read_to_string(&path).unwrap();
        let changed = original.replace("supersedes: []", "supersedes:\n- APP-D1");
        assert_ne!(changed, original);
        fs::write(path, changed).unwrap();
    }

    let output = run(root, &["validate", "-p", "APP", "-o", "json"]);
    assert_eq!(output.status.code(), Some(5));
    let result: Value = serde_json::from_slice(&output.stderr).unwrap();
    let invalid_paths: Vec<&str> = result["errors"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["code"] == "invalid_decision")
        .filter_map(|item| item["path"].as_str())
        .collect();
    assert!(invalid_paths.iter().any(|path| path.contains("APP-D2-")));
    assert!(invalid_paths.iter().any(|path| path.contains("APP-D3-")));
}

#[test]
fn knowledge_validation_aggregates_malformed_files_and_checks_valid_graphs() {
    let (temp, project) = fixture();
    let root = temp.path();
    for title in ["Cycle predecessor", "Cycle replacement"] {
        create_decision(root, title);
    }
    for id in ["APP-D1", "APP-D2"] {
        ok(root, &["decision", "accept", id, "-o", "json"]);
    }
    ok(
        root,
        &[
            "decision",
            "supersede",
            "APP-D1",
            "--with",
            "APP-D2",
            "-o",
            "json",
        ],
    );
    let d1_path = fs::read_dir(project.join("decisions"))
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| entry.file_name().to_string_lossy().starts_with("APP-D1-"))
        .unwrap()
        .path();
    let d1 = fs::read_to_string(&d1_path)
        .unwrap()
        .replace("supersedes: []", "supersedes:\n- APP-D2");
    fs::write(d1_path, d1).unwrap();

    let malformed = [
        project.join("resources/APP-R91-invalid-one.md"),
        project.join("resources/APP-R92-invalid-two.md"),
        project.join("decisions/APP-D91-invalid-one.md"),
        project.join("decisions/APP-D92-invalid-two.md"),
    ];
    for (index, path) in malformed.iter().enumerate() {
        fs::write(path, format!("invalid document {index}\n")).unwrap();
    }

    let output = run(root, &["validate", "-p", "APP", "-o", "json"]);
    assert_eq!(output.status.code(), Some(5));
    let result: Value = serde_json::from_slice(&output.stderr).unwrap();
    let errors = result["errors"].as_array().unwrap();
    for path in &malformed {
        let filename = path.file_name().unwrap().to_string_lossy();
        assert!(
            errors.iter().any(|item| {
                item["path"]
                    .as_str()
                    .is_some_and(|value| value.ends_with(filename.as_ref()))
                    && matches!(
                        item["code"].as_str(),
                        Some("invalid_resource" | "invalid_decision")
                    )
            }),
            "missing independent validation error for {filename}: {result}"
        );
    }
    assert!(
        errors
            .iter()
            .any(|item| item["code"] == "supersession_cycle")
    );
}

#[test]
fn one_accepted_replacement_can_supersede_multiple_predecessors() {
    let (temp, project) = fixture();
    let root = temp.path();
    for title in ["First predecessor", "Second predecessor", "Replacement"] {
        create_decision(root, title);
    }
    for id in ["APP-D1", "APP-D2", "APP-D3"] {
        ok(root, &["decision", "accept", id, "-o", "json"]);
    }
    ok(
        root,
        &[
            "decision",
            "supersede",
            "APP-D1",
            "--with",
            "APP-D3",
            "--with-if-revision",
            "2",
            "-o",
            "json",
        ],
    );
    let fan_in = ok(
        root,
        &[
            "decision",
            "supersede",
            "APP-D2",
            "--with",
            "APP-D3",
            "--with-if-revision",
            "3",
            "-o",
            "json",
        ],
    );
    assert_eq!(
        fan_in["replacement"]["supersedes"],
        json!(["APP-D1", "APP-D2"])
    );
    assert_eq!(
        ok(root, &["get", "decision", "APP-D1", "-o", "json"])["superseded_by"],
        "APP-D3"
    );
    assert_eq!(
        ok(root, &["get", "decision", "APP-D2", "-o", "json"])["superseded_by"],
        "APP-D3"
    );
    assert_eq!(
        ok(root, &["validate", "-p", "APP", "-o", "json"])["valid"],
        true
    );

    // Manual corruption still cannot give one predecessor two successors or
    // introduce a cycle in the authoritative supersession graph.
    create_decision(root, "Conflicting successor");
    ok(root, &["decision", "accept", "APP-D4", "-o", "json"]);
    let decision_path = |id: &str| {
        fs::read_dir(project.join("decisions"))
            .unwrap()
            .filter_map(Result::ok)
            .find(|entry| entry.file_name().to_string_lossy().starts_with(id))
            .unwrap()
            .path()
    };
    let d4_path = decision_path("APP-D4-");
    let d4 = fs::read_to_string(&d4_path)
        .unwrap()
        .replace("supersedes: []", "supersedes:\n- APP-D1");
    fs::write(d4_path, d4).unwrap();
    let d1_path = decision_path("APP-D1-");
    let d1 = fs::read_to_string(&d1_path)
        .unwrap()
        .replace("supersedes: []", "supersedes:\n- APP-D3");
    fs::write(d1_path, d1).unwrap();
    let output = run(root, &["validate", "-p", "APP", "-o", "json"]);
    assert_eq!(output.status.code(), Some(5));
    let result: Value = serde_json::from_slice(&output.stderr).unwrap();
    let codes: Vec<&str> = result["errors"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|error| error["code"].as_str())
        .collect();
    assert!(codes.contains(&"multiple_decision_replacements"));
    assert!(codes.contains(&"supersession_cycle"));
}

#[test]
fn ordinary_task_reads_and_mutations_recover_pending_context_batches() {
    let (temp, project) = fixture();
    let root = temp.path();
    ok(
        root,
        &["create", "task", "Stable task", "-p", "APP", "-o", "json"],
    );
    let task_path = project.join("tasks/APP-1.json");
    let before = fs::read(&task_path).unwrap();
    let mut after_value: Value = serde_json::from_slice(&before).unwrap();
    after_value["header"] = json!("Uncommitted header");
    after_value["revision"] = json!(2);
    let after = json_bytes(&after_value);
    stage_uncommitted_task_batch(&project, &before, &after);

    let listed = ok(
        root,
        &["list", "tasks", "-p", "APP", "--full", "-o", "json"],
    );
    assert_eq!(listed[0]["header"], "Stable task");
    assert_eq!(listed[0]["revision"], 1);
    assert!(!project.join(".tasker-transactions").exists());

    stage_uncommitted_task_batch(&project, &before, &after);
    let updated = ok(
        root,
        &[
            "update",
            "task",
            "APP-1",
            "--description",
            "Recovered before mutation",
            "--if-revision",
            "1",
            "-o",
            "json",
        ],
    );
    assert_eq!(updated["header"], "Stable task");
    assert_eq!(updated["description"], "Recovered before mutation");
    assert_eq!(updated["revision"], 2);
    assert!(!project.join(".tasker-transactions").exists());
}

#[test]
fn brief_warnings_archives_truncation_priority_and_help_are_explicit() {
    let (temp, project) = fixture();
    let root = temp.path();
    ok(
        root,
        &[
            "create",
            "resource",
            "Active linked",
            "-p",
            "APP",
            "--kind",
            "design",
            "--status",
            "active",
            "--content",
            &format!("# Active\n\n{}", "large body ".repeat(1500)),
            "-o",
            "json",
        ],
    );
    ok(
        root,
        &[
            "create",
            "resource",
            "Archived linked",
            "-p",
            "APP",
            "--kind",
            "research",
            "-o",
            "json",
        ],
    );
    ok(
        root,
        &[
            "create",
            "task",
            "Bounded task",
            "-p",
            "APP",
            "--context-ref",
            "APP-R1:implements",
            "--context-ref",
            "APP-R2:informed_by",
            "-o",
            "json",
        ],
    );
    ok(root, &["archive", "resource", "APP-R2", "-o", "json"]);

    let default_brief = ok(root, &["brief", "APP-1", "-o", "json"]);
    assert_eq!(default_brief["resources"].as_array().unwrap().len(), 1);
    assert_eq!(default_brief["resources"][0]["id"], "APP-R1");
    let archived = ok(
        root,
        &["brief", "APP-1", "--include-archived", "-o", "json"],
    );
    assert_eq!(archived["resources"].as_array().unwrap().len(), 2);

    let task_path = project.join("tasks/APP-1.json");
    let mut task: Value = serde_json::from_slice(&fs::read(&task_path).unwrap()).unwrap();
    task["context_refs"] = json!([
        {"kind":"resource","id":"APP-R1","role":"implements"},
        {"kind":"resource","id":"APP-R2","role":"informed_by"},
        {"kind":"resource","id":"APP-R7","role":"verifies"},
        {"kind":"resource","id":"APP-R8","role":"verifies"},
        {"kind":"resource","id":"APP-R9","role":"verifies"}
    ]);
    fs::write(&task_path, json_bytes(&task)).unwrap();
    let first = ok(root, &["brief", "APP-1", "-o", "json"]);
    let second = ok(root, &["brief", "APP-1", "-o", "json"]);
    assert_eq!(first["warnings"], second["warnings"]);
    assert_eq!(
        first["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|warning| warning["code"] == "broken_context_ref")
            .map(|warning| warning["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["APP-R7", "APP-R8", "APP-R9"]
    );

    let markdown = run(
        root,
        &["brief", "APP-1", "--max-bytes", "2500", "-o", "markdown"],
    );
    assert!(
        markdown.status.success(),
        "{}",
        String::from_utf8_lossy(&markdown.stderr)
    );
    let markdown = String::from_utf8(markdown.stdout).unwrap();
    assert!(markdown.contains("## Truncation"));
    assert!(markdown.contains("Truncated:"));
    let human = run(root, &["brief", "APP-1", "--max-bytes", "2500"]);
    assert!(human.status.success());
    assert!(
        String::from_utf8(human.stdout)
            .unwrap()
            .contains("## Truncation")
    );
    let bounded = ok(
        root,
        &["brief", "APP-1", "--max-bytes", "3000", "-o", "json"],
    );
    assert!(bounded["truncation"]["truncated"].as_bool().unwrap());
    assert!(
        bounded["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|resource| resource["id"] == "APP-R1")
    );
    assert!(
        !bounded["truncation"]["omitted"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "APP-R1" && item["field"] == "metadata")
    );

    let root_help = run(root, &["--help"]);
    let root_help = String::from_utf8(root_help.stdout).unwrap();
    assert!(root_help.contains("PLANNING-TO-EXECUTION WORKFLOW"));
    assert!(root_help.contains("KNOWLEDGE CONCEPTS"));
    let task_help = String::from_utf8(run(root, &["create", "task", "--help"]).stdout).unwrap();
    assert!(task_help.contains("context_refs"));
    let brief_help = String::from_utf8(run(root, &["brief", "--help"]).stdout).unwrap();
    assert!(brief_help.contains("Inclusion priority"));
    assert!(brief_help.contains("truncation notice"));
}

#[test]
fn brief_metadata_omission_follows_the_documented_priority() {
    let (temp, _) = fixture();
    let root = temp.path();
    for index in 1..=6 {
        let decision = create_decision(root, &format!("Project decision {index}"));
        ok(
            root,
            &[
                "decision",
                "accept",
                decision["id"].as_str().unwrap(),
                "-o",
                "json",
            ],
        );
    }
    ok(
        root,
        &["create", "task", "Priority task", "-p", "APP", "-o", "json"],
    );
    for index in 1..=12 {
        ok(
            root,
            &[
                "context",
                "add",
                "APP-1",
                &format!("Curated context entry {index} with useful execution details"),
                "--kind",
                "context",
                "-o",
                "json",
            ],
        );
    }

    let mut exercised = false;
    for max_bytes in [7000, 6000, 5500, 5000, 4500, 4000, 3500, 3000] {
        let max_bytes = max_bytes.to_string();
        let output = run(
            root,
            &["brief", "APP-1", "--max-bytes", &max_bytes, "-o", "json"],
        );
        if !output.status.success() {
            continue;
        }
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        let omissions = value["truncation"]["omitted"].as_array().unwrap();
        let project_position = omissions.iter().rposition(|item| {
            item["section"] == "decisions.project" && item["field"] == "metadata"
        });
        let context_position = omissions
            .iter()
            .position(|item| item["section"] == "task_context" && item["field"] == "metadata");
        if let (Some(project_position), Some(context_position)) =
            (project_position, context_position)
        {
            assert!(project_position < context_position);
            assert!(value["decisions"]["project"].as_array().unwrap().is_empty());
            exercised = true;
            break;
        }
    }
    assert!(
        exercised,
        "test fixture did not reach metadata priority boundary"
    );
}

#[test]
fn concurrent_resource_and_decision_allocations_are_unique() {
    let (temp, _) = fixture();
    let root = temp.path();
    let spawn_resource = |title: &'static str| {
        Command::new(bin())
            .args([
                "create", "resource", title, "-p", "APP", "--kind", "research", "-o", "json",
            ])
            .env("TASKER_ROOT", root)
            .env("TASKER_ACTOR", title)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    };
    let a = spawn_resource("Research A").wait_with_output().unwrap();
    let b = spawn_resource("Research B").wait_with_output().unwrap();
    assert!(a.status.success() && b.status.success());
    let ai: Value = serde_json::from_slice(&a.stdout).unwrap();
    let bi: Value = serde_json::from_slice(&b.stdout).unwrap();
    assert_ne!(ai["id"], bi["id"]);
    let spawn_decision = |title: &'static str| {
        Command::new(bin())
            .args([
                "create",
                "decision",
                title,
                "-p",
                "APP",
                "--content",
                decision_body(),
                "-o",
                "json",
            ])
            .env("TASKER_ROOT", root)
            .env("TASKER_ACTOR", title)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    };
    let a = spawn_decision("Decision A").wait_with_output().unwrap();
    let b = spawn_decision("Decision B").wait_with_output().unwrap();
    assert!(a.status.success() && b.status.success());
    let ai: Value = serde_json::from_slice(&a.stdout).unwrap();
    let bi: Value = serde_json::from_slice(&b.stdout).unwrap();
    assert_ne!(ai["id"], bi["id"]);
}
