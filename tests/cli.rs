use serde_json::Value;
use std::path::Path;
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
        .output()
        .unwrap()
}

fn ok(root: &Path, args: &[&str]) -> Value {
    let output = run(root, args);
    assert!(
        output.status.success(),
        "command {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn create_project(root: &Path) {
    let value = ok(
        root,
        &["create", "project", "Demo", "--prefix", "DEM", "-o", "json"],
    );
    assert_eq!(value["prefix"], "DEM");
}

#[test]
fn project_task_workflow_dependencies_relations_search_and_history() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    create_project(root);
    let projects = ok(root, &["list", "projects", "-o", "json"]);
    assert_eq!(projects.as_array().unwrap().len(), 1);
    assert_eq!(
        ok(
            root,
            &[
                "create", "task", "Storage", "-p", "DEM", "--tag", "Backend", "-o", "json"
            ]
        )["id"],
        "DEM-1"
    );
    assert_eq!(
        ok(
            root,
            &["create", "task", "Build CLI", "-p", "DEM", "-o", "json"]
        )["id"],
        "DEM-2"
    );
    ok(root, &["dependency", "add", "DEM-2", "DEM-1", "-o", "json"]);
    assert_eq!(
        ok(root, &["status", "DEM-2", "-o", "json"])["dependency_blocked"],
        true
    );
    let blocked = run(root, &["claim", "DEM-2", "--as", "cli-agent", "-o", "json"]);
    assert_eq!(blocked.status.code(), Some(5));
    assert_eq!(
        serde_json::from_slice::<Value>(&blocked.stderr).unwrap()["error"]["code"],
        "dependencies_unsatisfied"
    );
    let claimed = ok(
        root,
        &["claim", "DEM-1", "--as", "storage-agent", "-o", "json"],
    );
    assert_eq!(claimed["state"], "in_progress");
    assert_eq!(claimed["revision"], 2);
    ok(
        root,
        &[
            "transition",
            "DEM-1",
            "done",
            "--actor",
            "storage-agent",
            "-o",
            "json",
        ],
    );
    assert_eq!(
        ok(root, &["status", "DEM-2", "-o", "json"])["dependency_blocked"],
        false
    );
    ok(root, &["claim", "DEM-2", "--as", "cli-agent", "-o", "json"]);
    ok(
        root,
        &["create", "task", "JSON output", "-p", "DEM", "-o", "json"],
    );
    ok(
        root,
        &[
            "relation",
            "add",
            "DEM-3",
            "subtask_of",
            "DEM-2",
            "-o",
            "json",
        ],
    );
    let incoming = ok(
        root,
        &[
            "relation",
            "list",
            "DEM-2",
            "--direction",
            "incoming",
            "-o",
            "json",
        ],
    );
    assert_eq!(incoming[0]["type"], "parent_of");
    assert_eq!(incoming[0]["target"], "DEM-3");
    let cycle = run(root, &["dependency", "add", "DEM-1", "DEM-2", "-o", "json"]);
    assert_eq!(cycle.status.code(), Some(6));
    assert_eq!(
        serde_json::from_slice::<Value>(&cycle.stderr).unwrap()["error"]["code"],
        "dependency_cycle"
    );
    ok(
        root,
        &[
            "relation",
            "add",
            "DEM-2",
            "subtask_of",
            "DEM-1",
            "-o",
            "json",
        ],
    );
    let relation_cycle = run(
        root,
        &[
            "relation",
            "add",
            "DEM-1",
            "subtask_of",
            "DEM-3",
            "-o",
            "json",
        ],
    );
    assert_eq!(relation_cycle.status.code(), Some(6));
    assert_eq!(
        serde_json::from_slice::<Value>(&relation_cycle.stderr).unwrap()["error"]["code"],
        "relation_cycle"
    );
    ok(
        root,
        &["dependency", "remove", "DEM-2", "DEM-1", "-o", "json"],
    );
    assert_eq!(
        ok(root, &["search", "tasks", "cli", "-p", "DEM", "-o", "json"])[0]["id"],
        "DEM-2"
    );
    let history = ok(root, &["changelog", "--task", "DEM-1", "-o", "json"]);
    let actions: Vec<_> = history
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    assert!(
        actions.contains(&"task.created")
            && actions.contains(&"task.claimed")
            && actions.contains(&"task.transitioned")
    );
    assert_eq!(
        ok(root, &["validate", "-p", "DEM", "-o", "json"])["valid"],
        true
    );
}

#[test]
fn structured_input_update_users_yaml_and_inference() {
    let temp = TempDir::new().unwrap();
    create_project(temp.path());
    let mut json_create = Command::new(bin())
        .args(["create", "task", "-i", "json", "-p", "DEM", "-o", "json"])
        .env("TASKER_ROOT", temp.path())
        .env("TASKER_ACTOR", "tester")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    json_create
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"header":"From JSON","description":"body","tags":["API"]}"#)
        .unwrap();
    let output = json_create.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["tags"][0],
        "api"
    );

    let yaml = temp.path().join("task.yaml");
    std::fs::write(
        &yaml,
        "header: From YAML\ndescription: yaml body\ntags:\n  - docs\n",
    )
    .unwrap();
    let yaml_output = run(
        temp.path(),
        &[
            "create",
            "task",
            "-i",
            "yaml",
            "--file",
            yaml.to_str().unwrap(),
            "-p",
            "DEM",
            "-o",
            "yaml",
        ],
    );
    assert!(yaml_output.status.success());
    assert!(
        serde_yaml_ng::from_slice::<Value>(&yaml_output.stdout)
            .unwrap()
            .is_object()
    );
    let user = ok(
        temp.path(),
        &[
            "ensure", "user", "Pi Agent", "--kind", "agent", "-p", "DEM", "-o", "json",
        ],
    );
    let user2 = ok(
        temp.path(),
        &[
            "ensure", "user", "pi agent", "--kind", "agent", "-p", "DEM", "-o", "json",
        ],
    );
    assert_eq!(user["id"], user2["id"]);
    ok(temp.path(), &["assign", "DEM-1", "Pi Agent", "-o", "json"]);
    let inferred = ok(temp.path(), &["get", "task", "DEM-1", "-o", "json"]);
    assert_eq!(inferred["assignee"], "pi-agent");
    let project_dir = temp.path().join("demo");
    let cwd_output = Command::new(bin())
        .args(["list", "tasks", "-o", "json"])
        .current_dir(project_dir)
        .env("TASKER_ROOT", temp.path())
        .output()
        .unwrap();
    assert!(cwd_output.status.success());
}

#[test]
fn concurrent_claims_are_serialized() {
    let temp = TempDir::new().unwrap();
    create_project(temp.path());
    ok(
        temp.path(),
        &["create", "task", "Only task", "-p", "DEM", "-o", "json"],
    );
    let spawn = |agent: &'static str| {
        Command::new(bin())
            .args(["claim", "DEM-1", "--as", agent, "-o", "json"])
            .env("TASKER_ROOT", temp.path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    };
    let a = spawn("agent-a");
    let b = spawn("agent-b");
    let ao = a.wait_with_output().unwrap();
    let bo = b.wait_with_output().unwrap();
    assert_eq!(
        usize::from(ao.status.success()) + usize::from(bo.status.success()),
        1
    );
    let failed = if ao.status.success() { &bo } else { &ao };
    assert_eq!(failed.status.code(), Some(4));
    assert_eq!(
        serde_json::from_slice::<Value>(&failed.stderr).unwrap()["error"]["code"],
        "task_already_assigned"
    );
    let task = ok(temp.path(), &["get", "task", "DEM-1", "-o", "json"]);
    assert_eq!(task["revision"], 2);
    let history = ok(
        temp.path(),
        &[
            "changelog",
            "--task",
            "DEM-1",
            "--action",
            "task.claimed",
            "-o",
            "json",
        ],
    );
    assert_eq!(history.as_array().unwrap().len(), 1);
}

#[test]
fn concurrent_claim_next_assigns_distinct_tasks() {
    let temp = TempDir::new().unwrap();
    create_project(temp.path());
    ok(
        temp.path(),
        &["create", "task", "First", "-p", "DEM", "-o", "json"],
    );
    ok(
        temp.path(),
        &["create", "task", "Second", "-p", "DEM", "-o", "json"],
    );
    let spawn = |agent: &'static str| {
        Command::new(bin())
            .args(["claim", "next", "-p", "DEM", "--as", agent, "-o", "json"])
            .env("TASKER_ROOT", temp.path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    };
    let first = spawn("agent-a").wait_with_output().unwrap();
    let second = spawn("agent-b").wait_with_output().unwrap();
    assert!(first.status.success() && second.status.success());
    let first_task: Value = serde_json::from_slice(&first.stdout).unwrap();
    let second_task: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_ne!(first_task["id"], second_task["id"]);
}

#[test]
fn corrupted_metadata_cannot_overwrite_an_existing_task() {
    let temp = TempDir::new().unwrap();
    create_project(temp.path());
    ok(
        temp.path(),
        &["create", "task", "Original", "-p", "DEM", "-o", "json"],
    );
    let meta = temp.path().join("demo").join("meta.json");
    std::fs::write(
        &meta,
        "{\n  \"schema_version\": 1,\n  \"next_task_number\": 1\n}\n",
    )
    .unwrap();
    let failed = run(
        temp.path(),
        &["create", "task", "Replacement", "-p", "DEM", "-o", "json"],
    );
    assert_eq!(failed.status.code(), Some(7));
    assert_eq!(
        ok(temp.path(), &["get", "task", "DEM-1", "-o", "json"])["header"],
        "Original"
    );
}

#[test]
fn claim_next_honors_revision_and_validation_checks_user_filename() {
    let temp = TempDir::new().unwrap();
    create_project(temp.path());
    ok(
        temp.path(),
        &["create", "task", "First", "-p", "DEM", "-o", "json"],
    );
    let conflict = run(
        temp.path(),
        &[
            "claim",
            "next",
            "-p",
            "DEM",
            "--as",
            "agent",
            "--if-revision",
            "99",
            "-o",
            "json",
        ],
    );
    assert_eq!(conflict.status.code(), Some(4));
    let users = temp.path().join("demo").join("users");
    std::fs::rename(users.join("agent.json"), users.join("wrong.json")).unwrap();
    let invalid = run(temp.path(), &["validate", "-p", "DEM", "-o", "json"]);
    assert_eq!(invalid.status.code(), Some(5));
    let result: Value = serde_json::from_slice(&invalid.stderr).unwrap();
    assert!(
        result["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|problem| problem["code"] == "user_filename_mismatch")
    );
}

#[test]
fn concurrent_project_creation_preserves_prefix_uniqueness() {
    let temp = TempDir::new().unwrap();
    let spawn = |name: &'static str| {
        Command::new(bin())
            .args(["create", "project", name, "--prefix", "DUP", "-o", "json"])
            .env("TASKER_ROOT", temp.path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    };
    let first = spawn("First").wait_with_output().unwrap();
    let second = spawn("Second").wait_with_output().unwrap();
    assert_eq!(
        usize::from(first.status.success()) + usize::from(second.status.success()),
        1
    );
}

#[test]
fn acceptance_criteria_and_context_are_first_class_task_data() {
    let temp = TempDir::new().unwrap();
    create_project(temp.path());
    ok(
        temp.path(),
        &[
            "create",
            "task",
            "Measured work",
            "-p",
            "DEM",
            "--acceptance-criterion",
            "Result is deterministic",
            "-o",
            "json",
        ],
    );
    let created = ok(temp.path(), &["get", "task", "DEM-1", "-o", "json"]);
    assert_eq!(created["acceptance_criteria"][0]["id"], "AC-1");
    assert_eq!(created["acceptance_criteria"][0]["completed"], false);

    ok(
        temp.path(),
        &[
            "context",
            "add",
            "DEM-1",
            "Selected seeded generation",
            "--kind",
            "decision",
            "-o",
            "json",
        ],
    );
    let context = ok(temp.path(), &["context", "list", "DEM-1", "-o", "json"]);
    assert_eq!(context[0]["id"], "CTX-1");
    assert_eq!(context[0]["kind"], "decision");

    ok(
        temp.path(),
        &["claim", "DEM-1", "--as", "agent", "-o", "json"],
    );
    let incomplete = run(temp.path(), &["transition", "DEM-1", "done", "-o", "json"]);
    assert_eq!(incomplete.status.code(), Some(5));
    assert_eq!(
        serde_json::from_slice::<Value>(&incomplete.stderr).unwrap()["error"]["code"],
        "acceptance_criteria_incomplete"
    );
    ok(
        temp.path(),
        &["acceptance", "check", "DEM-1", "AC-1", "-o", "json"],
    );
    let completed = ok(temp.path(), &["transition", "DEM-1", "done", "-o", "json"]);
    assert_eq!(completed["state"], "done");
}

#[test]
fn every_command_level_has_agent_discoverable_help() {
    let commands = [
        "config",
        "config show",
        "config get",
        "config set",
        "create",
        "create project",
        "create task",
        "create user",
        "ensure",
        "ensure user",
        "list",
        "list projects",
        "list tasks",
        "list users",
        "get",
        "get project",
        "get task",
        "get user",
        "update",
        "update task",
        "update user",
        "assign",
        "unassign",
        "transition",
        "status",
        "claim",
        "next",
        "dependency",
        "dependency add",
        "dependency remove",
        "dependency list",
        "dependency check",
        "acceptance",
        "acceptance add",
        "acceptance remove",
        "acceptance check",
        "acceptance uncheck",
        "acceptance list",
        "context",
        "context add",
        "context list",
        "relation",
        "relation add",
        "relation remove",
        "relation list",
        "search",
        "search tasks",
        "search changelog",
        "changelog",
        "validate",
    ];
    for command in commands {
        let output = Command::new(bin())
            .args(command.split_whitespace())
            .arg("--help")
            .output()
            .unwrap();
        assert!(output.status.success(), "help failed for tasker {command}");
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(
            text.contains("Usage:"),
            "missing usage for tasker {command}"
        );
        assert!(
            text.len() > 250,
            "help is too terse for an unfamiliar agent: tasker {command}"
        );
    }

    let checks = [
        ("claim", "Atomically"),
        ("dependency", "depends on"),
        ("acceptance", "AC-N"),
        ("context", "progress"),
        ("relation", "subtask_of"),
        ("search", "case-insensitive"),
        ("transition", "acceptance criteria"),
        ("validate", "cross-file invariants"),
    ];
    for (command, expected) in checks {
        let output = Command::new(bin())
            .arg(command)
            .arg("--help")
            .output()
            .unwrap();
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(
            text.contains(expected),
            "tasker {command} --help is missing {expected}"
        );
    }
}

#[test]
fn help_and_machine_errors_are_agent_discoverable() {
    let help = Command::new(bin()).arg("--help").output().unwrap();
    let text = String::from_utf8(help.stdout).unwrap();
    for expected in [
        "AGENT QUICKSTART",
        "list projects",
        "list tasks",
        "claim next",
        "search tasks",
        "dependency add",
        "acceptance add",
        "context add",
        "relation add",
        "transition",
        "changelog",
    ] {
        assert!(text.contains(expected), "missing {expected}");
    }
    let temp = TempDir::new().unwrap();
    create_project(temp.path());
    let missing = run(temp.path(), &["get", "task", "DEM-999", "-o", "json"]);
    assert_eq!(missing.status.code(), Some(3));
    let error: Value = serde_json::from_slice(&missing.stderr).unwrap();
    assert_eq!(error["error"]["code"], "task_not_found");

    let config = temp.path().join("demo").join("tasker.yaml");
    let mut text = std::fs::read_to_string(&config).unwrap();
    text.push_str("unknown_project_field: true\n");
    std::fs::write(config, text).unwrap();
    let invalid = run(temp.path(), &["validate", "-p", "DEM", "-o", "json"]);
    assert_eq!(invalid.status.code(), Some(5));
    let result: Value = serde_json::from_slice(&invalid.stderr).unwrap();
    assert_eq!(result["valid"], false);
    assert_eq!(result["errors"][0]["code"], "malformed_project_config");
}
