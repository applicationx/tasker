use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_tasker")
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(bin())
        .args(args)
        .env("TASKER_ROOT", root)
        .env("TASKER_ACTOR", "tester")
        .env("TASKER_OUTPUT", "human")
        .env_remove("TASKER_PROJECT")
        .output()
        .unwrap()
}

fn ok(root: &Path, args: &[&str]) -> Value {
    let output = run(root, args);
    assert!(
        output.status.success(),
        "command {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn error(root: &Path, args: &[&str], code: &str) -> Value {
    let output = run(root, args);
    assert!(
        !output.status.success(),
        "command {args:?} unexpectedly passed"
    );
    let value: Value = serde_json::from_slice(&output.stderr).unwrap_or_else(|_| {
        panic!(
            "non-JSON error for {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert_eq!(value["error"]["code"], code, "command {args:?}");
    value
}

fn create_project(root: &Path) -> PathBuf {
    ok(
        root,
        &[
            "create",
            "project",
            "Coverage Demo",
            "--prefix",
            "DEM",
            "-o",
            "json",
        ],
    );
    root.join("coverage-demo")
}

fn create_task(root: &Path, header: &str) -> Value {
    ok(root, &["create", "task", header, "-p", "DEM", "-o", "json"])
}

fn write_json(path: &Path, value: &Value) {
    fs::write(
        path,
        format!("{}\n", serde_json::to_string_pretty(value).unwrap()),
    )
    .unwrap();
}

fn project_fixture() -> (TempDir, PathBuf) {
    let temp = TempDir::new().unwrap();
    let project = create_project(temp.path());
    (temp, project)
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn config_and_user_commands_persist_expected_users() {
    let (temp, project) = project_fixture();
    let root = temp.path();

    assert_eq!(
        ok(root, &["config", "get", "projects-root", "-o", "json"]),
        json!(root)
    );
    assert!(ok(root, &["config", "show", "-o", "json"])["config_file"].is_string());
    error(
        root,
        &["config", "get", "not-a-key", "-o", "json"],
        "invalid_input",
    );

    let alice = ok(
        root,
        &[
            "create",
            "user",
            "Alice Agent",
            "--kind",
            "agent",
            "-p",
            "DEM",
            "--actor",
            "Alice Agent",
            "-o",
            "json",
        ],
    );
    assert_eq!(alice["id"], "alice-agent");
    error(
        root,
        &["create", "user", "alice agent", "-p", "DEM", "-o", "json"],
        "user_name_exists",
    );
    ok(
        root,
        &[
            "create", "user", "Bob", "--kind", "human", "-p", "DEM", "-o", "json",
        ],
    );
    assert_eq!(
        ok(
            root,
            &["get", "user", "Alice Agent", "-p", "DEM", "-o", "json"]
        )["kind"],
        "agent"
    );
    assert!(
        ok(root, &["list", "users", "-p", "DEM", "-o", "json"])
            .as_array()
            .unwrap()
            .len()
            >= 3
    );

    let robert = ok(
        root,
        &[
            "update", "user", "bob", "--name", "Robert", "-p", "DEM", "-o", "json",
        ],
    );
    assert_eq!(robert["name"], "Robert");
    assert_eq!(
        read_json(&project.join("users").join("bob.json"))["name"],
        "Robert"
    );
    error(
        root,
        &[
            "update",
            "user",
            "robert",
            "--name",
            "Alice Agent",
            "-p",
            "DEM",
            "-o",
            "json",
        ],
        "user_name_exists",
    );
    error(
        root,
        &[
            "update", "user", "robert", "--name", " ", "-p", "DEM", "-o", "json",
        ],
        "invalid_input",
    );
}

#[test]
fn task_updates_assignment_and_revision_guards_persist_exactly() {
    let (temp, project) = project_fixture();
    let root = temp.path();
    let description = root.join("description.txt");
    fs::write(&description, "description from file").unwrap();
    let created = ok(
        root,
        &[
            "create",
            "task",
            "First searchable task",
            "-p",
            "DEM",
            "--description-file",
            description.to_str().unwrap(),
            "--tag",
            "API",
            "--acceptance-criterion",
            "Response is stable",
            "-o",
            "json",
        ],
    );
    assert_eq!(created["description"], "description from file");
    assert_eq!(created["revision"], 1);

    let patch = root.join("patch.yaml");
    fs::write(
        &patch,
        "header: Updated searchable task\ntags: [api, rust]\n",
    )
    .unwrap();
    let updated = ok(
        root,
        &[
            "update",
            "task",
            "DEM-1",
            "-i",
            "yaml",
            "--file",
            patch.to_str().unwrap(),
            "--description",
            "updated body",
            "-o",
            "json",
        ],
    );
    assert_eq!(updated["tags"], json!(["api", "rust"]));
    assert_eq!(updated["revision"], 2);
    let unchanged = ok(
        root,
        &[
            "update",
            "task",
            "DEM-1",
            "--header",
            "Updated searchable task",
            "-o",
            "json",
        ],
    );
    assert_eq!(unchanged["revision"], 2);
    error(
        root,
        &["update", "task", "DEM-1", "-o", "json"],
        "invalid_input",
    );
    error(
        root,
        &["update", "task", "DEM-1", "--header", " ", "-o", "json"],
        "invalid_input",
    );
    let protected = root.join("protected.json");
    fs::write(&protected, r#"{"state":"done"}"#).unwrap();
    error(
        root,
        &[
            "update",
            "task",
            "DEM-1",
            "-i",
            "json",
            "--file",
            protected.to_str().unwrap(),
            "-o",
            "json",
        ],
        "protected_task_field",
    );

    ok(
        root,
        &[
            "create",
            "user",
            "Alice Agent",
            "--kind",
            "agent",
            "-p",
            "DEM",
            "-o",
            "json",
        ],
    );
    let assigned = ok(
        root,
        &[
            "assign",
            "DEM-1",
            "Alice Agent",
            "--if-revision",
            "2",
            "-o",
            "json",
        ],
    );
    assert_eq!(assigned["assignee"], "alice-agent");
    assert_eq!(assigned["revision"], 3);
    let same = ok(root, &["assign", "DEM-1", "alice-agent", "-o", "json"]);
    assert_eq!(same["revision"], 3);
    let unassigned = ok(root, &["unassign", "DEM-1", "-o", "json"]);
    assert_eq!(unassigned["revision"], 4);
    let unchanged_unassigned = ok(root, &["unassign", "DEM-1", "-o", "json"]);
    assert_eq!(unchanged_unassigned["revision"], 4);
    assert!(unchanged_unassigned["assignee"].is_null());
    error(
        root,
        &[
            "update",
            "task",
            "DEM-1",
            "--header",
            "stale",
            "--if-revision",
            "1",
            "-o",
            "json",
        ],
        "revision_conflict",
    );

    let persisted = read_json(&project.join("tasks").join("DEM-1.json"));
    assert_eq!(persisted["revision"], 4);
    assert!(persisted["assignee"].is_null());
    assert_eq!(persisted["header"], "Updated searchable task");
}

#[test]
fn dependency_commands_report_graph_state_and_preserve_revision() {
    let (temp, project) = project_fixture();
    let root = temp.path();
    for header in ["Root", "Middle", "Leaf"] {
        create_task(root, header);
    }

    let root_task = ok(root, &["dependency", "add", "DEM-1", "DEM-2", "-o", "json"]);
    assert_eq!(root_task["revision"], 2);
    ok(root, &["dependency", "add", "DEM-2", "DEM-3", "-o", "json"]);
    assert_eq!(
        ok(
            root,
            &["dependency", "list", "DEM-1", "--recursive", "-o", "json"]
        ),
        json!(["DEM-2", "DEM-3"])
    );
    assert_eq!(
        ok(
            root,
            &["dependency", "list", "DEM-3", "--reverse", "-o", "json"]
        ),
        json!(["DEM-2"])
    );
    let check = ok(root, &["dependency", "check", "DEM-1", "-o", "json"]);
    assert_eq!(check["dependency_blocked"], true);
    assert_eq!(check["unresolved_dependencies"], json!(["DEM-2"]));

    error(
        root,
        &["dependency", "add", "DEM-1", "DEM-1", "-o", "json"],
        "self_dependency",
    );
    error(
        root,
        &["dependency", "add", "DEM-1", "DEM-2", "-o", "json"],
        "dependency_exists",
    );
    error(
        root,
        &["dependency", "remove", "DEM-1", "DEM-3", "-o", "json"],
        "dependency_not_found",
    );

    let persisted = read_json(&project.join("tasks").join("DEM-1.json"));
    assert_eq!(persisted["dependencies"], json!(["DEM-2"]));
    assert_eq!(persisted["revision"], 2);
}

#[test]
fn acceptance_and_context_commands_are_idempotent_and_append_only() {
    let (temp, project) = project_fixture();
    let root = temp.path();
    ok(
        root,
        &[
            "create",
            "task",
            "Acceptance task",
            "-p",
            "DEM",
            "--acceptance-criterion",
            "Response is stable",
            "-o",
            "json",
        ],
    );

    error(
        root,
        &[
            "acceptance",
            "add",
            "DEM-1",
            "response IS stable",
            "-o",
            "json",
        ],
        "acceptance_criterion_exists",
    );
    error(
        root,
        &["acceptance", "add", "DEM-1", " ", "-o", "json"],
        "invalid_input",
    );
    let checked = ok(
        root,
        &["acceptance", "check", "DEM-1", "ac-1", "-o", "json"],
    );
    assert_eq!(checked["revision"], 2);
    let checked_twice = ok(
        root,
        &["acceptance", "check", "DEM-1", "AC-1", "-o", "json"],
    );
    assert_eq!(checked_twice["revision"], 2);
    let unchecked = ok(
        root,
        &["acceptance", "uncheck", "DEM-1", "AC-1", "-o", "json"],
    );
    assert_eq!(unchecked["revision"], 3);
    let unchecked_twice = ok(
        root,
        &["acceptance", "uncheck", "DEM-1", "AC-1", "-o", "json"],
    );
    assert_eq!(unchecked_twice["revision"], 3);
    assert_eq!(
        ok(root, &["acceptance", "list", "DEM-1", "-o", "json"])[0]["completed"],
        false
    );
    error(
        root,
        &["acceptance", "check", "DEM-1", "AC-404", "-o", "json"],
        "acceptance_criterion_not_found",
    );

    let progress = ok(
        root,
        &[
            "context",
            "add",
            "DEM-1",
            "Implemented token parser",
            "--kind",
            "progress",
            "-o",
            "json",
        ],
    );
    assert_eq!(progress["revision"], 4);
    let decision = ok(
        root,
        &[
            "context",
            "add",
            "DEM-1",
            "Keep filesystem authority",
            "--kind",
            "decision",
            "-o",
            "json",
        ],
    );
    assert_eq!(decision["revision"], 5);
    assert_eq!(
        ok(
            root,
            &[
                "context", "list", "DEM-1", "--kind", "decision", "-o", "json"
            ]
        )[0]["kind"],
        "decision"
    );
    error(
        root,
        &["context", "add", "DEM-1", " ", "-o", "json"],
        "invalid_input",
    );

    let persisted = read_json(&project.join("tasks").join("DEM-1.json"));
    assert_eq!(persisted["revision"], 5);
    assert_eq!(persisted["context"].as_array().unwrap().len(), 2);
    assert_eq!(persisted["acceptance_criteria"][0]["completed"], false);
}

#[test]
fn relation_commands_enforce_symmetric_identity_and_persist_removal() {
    let (temp, project) = project_fixture();
    let root = temp.path();
    for header in ["Source", "Unused", "Target"] {
        create_task(root, header);
    }

    let related = ok(
        root,
        &[
            "relation",
            "add",
            "DEM-1",
            "relates_to",
            "DEM-3",
            "-o",
            "json",
        ],
    );
    assert_eq!(related["revision"], 2);
    let both = ok(
        root,
        &[
            "relation",
            "list",
            "DEM-1",
            "--direction",
            "both",
            "-o",
            "json",
        ],
    );
    assert_eq!(both[0]["direction"], "outgoing");
    error(
        root,
        &[
            "relation",
            "add",
            "DEM-3",
            "relates_to",
            "DEM-1",
            "-o",
            "json",
        ],
        "relation_exists",
    );
    error(
        root,
        &["relation", "add", "DEM-1", "unknown", "DEM-2", "-o", "json"],
        "unknown_relation_type",
    );
    error(
        root,
        &[
            "relation",
            "add",
            "DEM-1",
            "relates_to",
            "DEM-1",
            "-o",
            "json",
        ],
        "self_relation",
    );
    let removed = ok(
        root,
        &[
            "relation",
            "remove",
            "DEM-1",
            "relates_to",
            "DEM-3",
            "-o",
            "json",
        ],
    );
    assert_eq!(removed["revision"], 3);
    error(
        root,
        &[
            "relation",
            "remove",
            "DEM-1",
            "relates_to",
            "DEM-3",
            "-o",
            "json",
        ],
        "relation_not_found",
    );

    let persisted = read_json(&project.join("tasks").join("DEM-1.json"));
    assert_eq!(persisted["revision"], 3);
    assert_eq!(persisted["relations"], json!([]));
}

#[test]
fn task_filters_search_changelog_and_next_cover_selection_errors() {
    let (temp, _) = project_fixture();
    let root = temp.path();
    for header in [
        "First searchable task",
        "Second dependency",
        "Third dependency",
    ] {
        create_task(root, header);
    }
    ok(
        root,
        &[
            "update",
            "task",
            "DEM-1",
            "--description",
            "token parser",
            "--tag",
            "api",
            "-o",
            "json",
        ],
    );
    ok(
        root,
        &[
            "context",
            "add",
            "DEM-1",
            "Keep filesystem authority",
            "--kind",
            "decision",
            "-o",
            "json",
        ],
    );
    ok(root, &["dependency", "add", "DEM-1", "DEM-2", "-o", "json"]);

    let filtered = ok(
        root,
        &[
            "search",
            "tasks",
            "token filesystem",
            "-p",
            "DEM",
            "--state",
            "backlog",
            "--tag",
            "api",
            "--unassigned",
            "--dependency-blocked",
            "--full",
            "-o",
            "json",
        ],
    );
    assert_eq!(filtered.as_array().unwrap().len(), 1);
    assert_eq!(filtered[0]["id"], "DEM-1");
    let paged_ready = ok(
        root,
        &[
            "list", "tasks", "-p", "DEM", "--ready", "--offset", "1", "--limit", "1", "-o", "json",
        ],
    );
    assert_eq!(paged_ready.as_array().unwrap().len(), 1);
    assert_eq!(paged_ready[0]["id"], "DEM-3");
    error(
        root,
        &[
            "list",
            "tasks",
            "-p",
            "DEM",
            "--state",
            "imaginary",
            "-o",
            "json",
        ],
        "unknown_state",
    );
    let searched_events = ok(
        root,
        &[
            "search",
            "changelog",
            "task updated",
            "-p",
            "DEM",
            "--limit",
            "2",
            "-o",
            "json",
        ],
    );
    let searched_events = searched_events.as_array().unwrap();
    assert_eq!(searched_events.len(), 2, "the requested limit is enforced");
    assert!(
        searched_events
            .iter()
            .any(|event| event["action"] == "task.updated" && event["task_id"] == "DEM-1")
    );

    let filtered_events = ok(
        root,
        &[
            "changelog",
            "-p",
            "DEM",
            "--actor",
            "tester",
            "--action",
            "task.updated",
            "-o",
            "json",
        ],
    );
    assert_eq!(filtered_events.as_array().unwrap().len(), 1);
    assert_eq!(filtered_events[0]["actor"]["id"], "tester");
    assert_eq!(filtered_events[0]["action"], "task.updated");
    assert_eq!(
        ok(root, &["list", "projects", "--stats", "-o", "json"])[0]["stats"]["backlog"],
        3
    );
    error(
        root,
        &["transition", "DEM-3", "imaginary", "-o", "json"],
        "unknown_state",
    );
    error(
        root,
        &["transition", "DEM-3", "done", "-o", "json"],
        "transition_not_allowed",
    );
    error(
        root,
        &["next", "-p", "DEM", "--as", "missing-user", "-o", "json"],
        "user_not_found",
    );
}

#[test]
fn managed_storage_is_pretty_and_project_errors_are_structured() {
    let (temp, project) = project_fixture();
    let root = temp.path();
    create_task(root, "Stored task");

    assert_eq!(
        ok(root, &["get", "project", "DEM", "-o", "json"])["name"],
        "Coverage Demo"
    );
    error(
        root,
        &["get", "task", "DEM-1", "-p", "OTHER", "-o", "json"],
        "project_not_found",
    );

    let task_text = fs::read_to_string(project.join("tasks").join("DEM-1.json")).unwrap();
    assert!(task_text.starts_with("{\n  \"schema_version\"") && task_text.ends_with('\n'));
    let persisted: Value = serde_json::from_str(&task_text).unwrap();
    assert_eq!(persisted["id"], "DEM-1");
    assert_eq!(persisted["revision"], 1);
}

#[test]
fn validate_reports_each_persisted_invariant_and_graph_corruption() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let project = create_project(root);
    for header in ["One", "Two", "Three"] {
        create_task(root, header);
    }

    let config_path = project.join("tasker.yaml");
    let original_config = fs::read_to_string(&config_path).unwrap();
    let base: serde_yaml_ng::Value = serde_yaml_ng::from_str(&original_config).unwrap();
    let project_selector = project.to_str().unwrap();
    let invalid_configs = [("schema_version", json!(2)), ("prefix", json!("bad"))];
    for (field, replacement) in invalid_configs {
        let mut config_json = serde_json::to_value(&base).unwrap();
        config_json[field] = replacement;
        fs::write(
            &config_path,
            serde_yaml_ng::to_string(&config_json).unwrap(),
        )
        .unwrap();
        let output = run(root, &["validate", "-p", project_selector, "-o", "json"]);
        assert_eq!(output.status.code(), Some(5));
    }
    for (field, replacement) in [("initial_state", "ghost"), ("claim_state", "ghost")] {
        let mut config_json = serde_json::to_value(&base).unwrap();
        config_json["workflow"][field] = json!(replacement);
        fs::write(
            &config_path,
            serde_yaml_ng::to_string(&config_json).unwrap(),
        )
        .unwrap();
        let output = run(root, &["validate", "-p", project_selector, "-o", "json"]);
        assert_eq!(output.status.code(), Some(5));
    }
    let mut config_json = serde_json::to_value(&base).unwrap();
    config_json["workflow"]["transitions"]["backlog"] = json!(["ghost"]);
    fs::write(
        &config_path,
        serde_yaml_ng::to_string(&config_json).unwrap(),
    )
    .unwrap();
    assert_eq!(
        run(root, &["validate", "-p", project_selector, "-o", "json"])
            .status
            .code(),
        Some(5)
    );

    let mut config_json = serde_json::to_value(&base).unwrap();
    config_json["relations"]["peer"] = json!({
        "inverse_label": "peer",
        "symmetric": true,
        "acyclic": true
    });
    fs::write(
        &config_path,
        serde_yaml_ng::to_string(&config_json).unwrap(),
    )
    .unwrap();

    let tasks = project.join("tasks");
    let mut one: Value =
        serde_json::from_str(&fs::read_to_string(tasks.join("DEM-1.json")).unwrap()).unwrap();
    one["schema_version"] = json!(2);
    one["id"] = json!("BAD-1");
    one["state"] = json!("ghost");
    one["assignee"] = json!("missing-user");
    one["acceptance_criteria"] = json!([
        {"id":"AC-X","text":"","completed":true,"completed_at":null,"completed_by":null},
        {"id":"AC-X","text":"duplicate","completed":false}
    ]);
    one["context"] = json!([
        {"id":"CTX-X","kind":"context","message":"","created_at":"now","created_by":"tester"},
        {"id":"CTX-X","kind":"decision","message":"duplicate","created_at":"now","created_by":"tester"}
    ]);
    one["dependencies"] = json!(["BAD-1", "BAD-1", "MISSING-9"]);
    one["relations"] = json!([
        {"type":"unknown","target":"BAD-1"},
        {"type":"relates_to","target":"MISSING-9"}
    ]);
    write_json(&tasks.join("wrong-name.json"), &one);
    fs::remove_file(tasks.join("DEM-1.json")).unwrap();

    for (id, other) in [("DEM-2", "DEM-3"), ("DEM-3", "DEM-2")] {
        let path = tasks.join(format!("{id}.json"));
        let mut task: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        task["dependencies"] = json!([other]);
        task["relations"] = json!([
            {"type":"subtask_of","target":other},
            {"type":"peer","target":other}
        ]);
        write_json(&path, &task);
    }
    fs::write(tasks.join("malformed.json"), "{not json\n").unwrap();

    let users = project.join("users");
    let tester_path = users.join("tester.json");
    let mut tester: Value =
        serde_json::from_str(&fs::read_to_string(&tester_path).unwrap()).unwrap();
    tester["schema_version"] = json!(2);
    write_json(&tester_path, &tester);
    tester["schema_version"] = json!(1);
    tester["id"] = json!("duplicate-user");
    tester["name"] = json!("TESTER");
    write_json(&users.join("wrong-user-file.json"), &tester);
    fs::write(users.join("malformed.json"), "[]\n").unwrap();

    write_json(
        &project.join("meta.json"),
        &json!({"schema_version": 2, "next_task_number": 1}),
    );
    let changelog = project.join("changelog");
    let event_path = fs::read_dir(&changelog)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut event: Value = serde_json::from_str(&fs::read_to_string(&event_path).unwrap()).unwrap();
    event["schema_version"] = json!(2);
    write_json(&event_path, &event);
    fs::write(changelog.join("malformed.json"), "null\n").unwrap();

    let output = run(root, &["validate", "-p", project_selector, "-o", "json"]);
    assert_eq!(output.status.code(), Some(5));
    let result: Value = serde_json::from_slice(&output.stderr).unwrap();
    let codes: Vec<&str> = result["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|problem| problem["code"].as_str().unwrap())
        .collect();
    for expected in [
        "task_filename_mismatch",
        "invalid_schema_version",
        "task_prefix_mismatch",
        "unknown_task_state",
        "invalid_acceptance_criteria",
        "invalid_task_context",
        "duplicate_dependency",
        "self_dependency",
        "malformed_task_json",
        "user_filename_mismatch",
        "malformed_user_json",
        "duplicate_user_name",
        "missing_assignee_user",
        "dependency_target_missing",
        "unknown_relation_type",
        "self_relation",
        "relation_target_missing",
        "dependency_cycle",
        "relation_cycle",
        "invalid_next_task_number",
        "malformed_changelog_event",
    ] {
        assert!(
            codes.contains(&expected),
            "missing {expected}; got {codes:?}"
        );
    }
}

#[test]
fn human_pretty_and_parse_error_rendering_are_public_cli_behavior() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    create_project(root);
    ok(
        root,
        &[
            "create",
            "task",
            "Human row",
            "-p",
            "DEM",
            "--acceptance-criterion",
            "Visible check",
            "-o",
            "json",
        ],
    );
    ok(
        root,
        &["context", "add", "DEM-1", "Visible context", "-o", "json"],
    );

    for args in [
        vec!["list", "projects"],
        vec!["list", "tasks", "-p", "DEM"],
        vec!["list", "users", "-p", "DEM"],
        vec!["acceptance", "list", "DEM-1"],
        vec!["context", "list", "DEM-1"],
        vec!["status", "DEM-1"],
        vec!["config", "get", "projects-root"],
    ] {
        let output = run(root, &args);
        assert!(output.status.success(), "{args:?}");
        assert!(!output.stdout.is_empty());
    }
    let empty = run(root, &["search", "tasks", "does-not-exist", "-p", "DEM"]);
    assert_eq!(
        String::from_utf8(empty.stdout).unwrap().trim(),
        "No results."
    );
    let human_error = run(root, &["get", "task", "DEM-404"]);
    assert!(
        String::from_utf8(human_error.stderr)
            .unwrap()
            .starts_with("error [task_not_found]:")
    );
    let pretty = run(root, &["get", "task", "DEM-1", "-o", "json", "--pretty"]);
    assert!(
        String::from_utf8(pretty.stdout)
            .unwrap()
            .contains("\n  \"schema_version\"")
    );

    for args in [
        vec!["--output=json", "definitely-not-a-command"],
        vec!["-o", "json", "definitely-not-a-command"],
    ] {
        let output = run(root, &args);
        assert_eq!(output.status.code(), Some(2));
        let parsed: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(parsed["error"]["code"], "invalid_input");
    }
    let env_parse_error = Command::new(bin())
        .arg("definitely-not-a-command")
        .env("TASKER_ROOT", root)
        .env("TASKER_OUTPUT", "yaml")
        .output()
        .unwrap();
    assert_eq!(env_parse_error.status.code(), Some(2));
    let parsed: Value = serde_yaml_ng::from_slice(&env_parse_error.stderr).unwrap();
    assert_eq!(parsed["error"]["code"], "invalid_input");
}
