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
    assert_eq!(value["workflow"]["initial_state"], "backlog");
    assert_eq!(
        value["relations"]["subtask_of"]["inverse_label"],
        "parent_of"
    );
    assert!(
        value["config_file"]
            .as_str()
            .unwrap()
            .ends_with("tasker.yaml")
    );
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

    let added = ok(
        temp.path(),
        &[
            "acceptance",
            "add",
            "DEM-1",
            "Temporary checkpoint",
            "-o",
            "json",
        ],
    );
    assert_eq!(added["acceptance_criteria"][1]["id"], "AC-2");
    ok(
        temp.path(),
        &["acceptance", "remove", "DEM-1", "AC-2", "-o", "json"],
    );
    let added_again = ok(
        temp.path(),
        &[
            "acceptance",
            "add",
            "DEM-1",
            "Replacement checkpoint",
            "-o",
            "json",
        ],
    );
    assert_eq!(added_again["acceptance_criteria"][1]["id"], "AC-3");
    ok(
        temp.path(),
        &["acceptance", "remove", "DEM-1", "AC-3", "-o", "json"],
    );

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
        "ui",
        "ui start",
        "ui stop",
        "ui restart",
        "ui status",
        "config",
        "config show",
        "config get",
        "config set",
        "create",
        "create project",
        "create task",
        "create user",
        "create resource",
        "create decision",
        "project",
        "project brief",
        "project brief show",
        "project brief path",
        "project brief init",
        "project brief set",
        "ensure",
        "ensure user",
        "list",
        "list projects",
        "list tasks",
        "list users",
        "list resources",
        "list decisions",
        "get",
        "get project",
        "get task",
        "get user",
        "get resource",
        "get decision",
        "update",
        "update task",
        "update user",
        "update resource",
        "update decision",
        "archive",
        "archive resource",
        "decision",
        "decision accept",
        "decision reject",
        "decision supersede",
        "context-ref",
        "context-ref add",
        "context-ref list",
        "context-ref remove",
        "context-ref reverse",
        "brief",
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
        "search resources",
        "search decisions",
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
        ("ui", "127.0.0.1"),
        ("ui status", "nonce-authenticated"),
        ("config", "TASKER_ROOT"),
        ("create project", "^[A-Z][A-Z0-9]{1,9}$"),
        ("create task", "acceptance_criteria"),
        ("create resource", "project-prefixed ID"),
        ("create decision", "Options considered"),
        ("project brief show", "raw Markdown"),
        ("decision supersede", "project lock"),
        ("context-ref add", "idempotent"),
        ("brief", "byte-bounded"),
        ("get project", "config_file"),
        ("claim", "exclusive project lock"),
        ("next", "not an exact preview"),
        ("dependency", "same project"),
        ("acceptance", "never reused"),
        ("context", "progress"),
        ("relation remove", "derived views"),
        ("search tasks", "QUERY is optional"),
        ("changelog", "task.transitioned"),
        ("transition", "Unresolved dependencies do not prohibit"),
        ("validate", "never creates"),
    ];
    for (command, expected) in checks {
        let output = Command::new(bin())
            .args(command.split_whitespace())
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

/// Runs tasker in `dir` with an isolated configuration directory and no
/// TASKER_ROOT unless one is given, so workspace discovery and the registry
/// are exercised without touching real user configuration.
fn run_workspace(dir: &Path, config: &Path, root: Option<&Path>, args: &[&str]) -> Output {
    let mut command = Command::new(bin());
    command
        .args(args)
        .current_dir(dir)
        .env("TASKER_ACTOR", "tester")
        .env("TASKER_CONFIG_DIR", config)
        .env_remove("TASKER_ROOT")
        .env_remove("TASKER_PROJECT");
    if let Some(root) = root {
        command.env("TASKER_ROOT", root);
    }
    command.output().unwrap()
}

fn ok_workspace(dir: &Path, config: &Path, root: Option<&Path>, args: &[&str]) -> Value {
    let output = run_workspace(dir, config, root, args);
    assert!(
        output.status.success(),
        "command {:?} in {} failed: {}",
        args,
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn same_path(left: &str, right: &Path) -> bool {
    Path::new(left).canonicalize().unwrap() == right.canonicalize().unwrap()
}

#[test]
fn workspace_board_is_discovered_initialized_and_listed() {
    let temp = TempDir::new().unwrap();
    let config = temp.path().join("config");
    let repo = temp.path().join("repo");
    let nested = repo.join("src").join("deep");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::create_dir_all(repo.join("tasker")).unwrap();
    std::fs::create_dir_all(&nested).unwrap();
    let board = repo.join(".tasker");

    // A Git repository or an undotted tasker folder never selects a root.
    let shown = ok_workspace(&nested, &config, None, &["config", "show", "-o", "json"]);
    assert_ne!(shown["root_source"], "workspace");

    // init creates .tasker at the Git top level, ignores transient files,
    // registers the board, and creates the first project.
    let init = ok_workspace(
        &nested,
        &config,
        None,
        &["init", "--name", "Demo", "--prefix", "DEM", "-o", "json"],
    );
    assert_eq!(init["created"], true);
    assert_eq!(init["gitignore"], true);
    assert_eq!(init["registered"], true);
    assert_eq!(init["project"]["prefix"], "DEM");
    assert!(same_path(init["root"].as_str().unwrap(), &board));
    let gitignore = std::fs::read_to_string(board.join(".gitignore")).unwrap();
    assert!(gitignore.contains(".tasker-root.lock"));
    assert!(gitignore.contains(".tasker-transactions/"));
    let again = ok_workspace(
        &repo,
        &config,
        None,
        &["init", "--name", "Demo", "--prefix", "DEM", "-o", "json"],
    );
    assert_eq!(again["created"], false);
    assert_eq!(again["gitignore"], false);
    assert_eq!(again["registered"], false);
    assert_eq!(again["project"]["prefix"], "DEM");

    // Every directory inside the repository resolves the same board.
    for dir in [repo.clone(), nested.clone(), board.join("demo")] {
        let shown = ok_workspace(&dir, &config, None, &["config", "show", "-o", "json"]);
        assert_eq!(shown["root_source"], "workspace", "from {}", dir.display());
        assert!(same_path(shown["projects_root"].as_str().unwrap(), &board));
    }
    let task = ok_workspace(
        &nested,
        &config,
        None,
        &["create", "task", "Local task", "-p", "DEM", "-o", "json"],
    );
    assert_eq!(task["id"], "DEM-1");
    assert!(board.join("demo/tasks/DEM-1.json").is_file());

    // TASKER_ROOT still takes precedence over the workspace board.
    let other = temp.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    let shown = ok_workspace(
        &nested,
        &config,
        Some(&other),
        &["config", "show", "-o", "json"],
    );
    assert_eq!(shown["root_source"], "env");
    assert!(same_path(shown["projects_root"].as_str().unwrap(), &other));

    // Elsewhere, bare `tasker list` shows the current root plus the board.
    let listed = ok_workspace(&other, &config, Some(&other), &["list", "-o", "json"]);
    let boards = listed["boards"].as_array().unwrap();
    assert_eq!(boards.len(), 2);
    assert_eq!(boards[0]["current"], true);
    assert_eq!(boards[0]["source"], "env");
    assert_eq!(boards[1]["source"], "registry");
    assert_eq!(boards[1]["projects"][0]["prefix"], "DEM");
    let stats = ok_workspace(
        &other,
        &config,
        Some(&other),
        &["list", "--stats", "-o", "json"],
    );
    assert_eq!(stats["boards"][1]["projects"][0]["stats"]["backlog"], 1);

    // Inside the repository the board is current and not duplicated.
    let listed = ok_workspace(&nested, &config, None, &["list", "-o", "json"]);
    let boards = listed["boards"].as_array().unwrap();
    assert_eq!(boards.len(), 1);
    assert_eq!(boards[0]["source"], "workspace");
    assert_eq!(boards[0]["registered"], true);
}

#[test]
fn board_registry_round_trips_and_tolerates_missing_roots() {
    let temp = TempDir::new().unwrap();
    let config = temp.path().join("config");
    let current = temp.path().join("current");
    let gone = temp.path().join("gone");
    std::fs::create_dir_all(&current).unwrap();
    std::fs::create_dir_all(&gone).unwrap();
    let gone_text = gone.to_string_lossy().to_string();
    let root = Some(current.as_path());

    let added = ok_workspace(
        &current,
        &config,
        root,
        &["root", "add", &gone_text, "-o", "json"],
    );
    assert_eq!(added["added"], true);
    let repeated = ok_workspace(
        &current,
        &config,
        root,
        &["root", "add", &gone_text, "-o", "json"],
    );
    assert_eq!(repeated["added"], false);
    let missing = run_workspace(
        &current,
        &config,
        root,
        &["root", "add", "absent", "-o", "json"],
    );
    assert_eq!(missing.status.code(), Some(2));

    let registered = added["root"].as_str().unwrap().to_string();
    std::fs::remove_dir(&gone).unwrap();
    let roots = ok_workspace(&current, &config, root, &["root", "list", "-o", "json"]);
    assert_eq!(roots[0]["root"], registered.as_str());
    assert_eq!(roots[0]["exists"], false);

    let listed = ok_workspace(&current, &config, root, &["list", "-o", "json"]);
    assert_eq!(listed["boards"].as_array().unwrap().len(), 1);
    assert_eq!(listed["warnings"][0]["code"], "registered_root_missing");

    let removed = ok_workspace(
        &current,
        &config,
        root,
        &["root", "remove", &registered, "-o", "json"],
    );
    assert_eq!(removed["removed"], true);
    let roots = ok_workspace(&current, &config, root, &["root", "list", "-o", "json"]);
    assert!(roots.as_array().unwrap().is_empty());
}

#[test]
fn competing_registry_writers_keep_every_root() {
    let temp = TempDir::new().unwrap();
    let config = temp.path().join("config");
    let dirs: Vec<_> = (0..8)
        .map(|n| {
            let dir = temp.path().join(format!("board-{n}"));
            std::fs::create_dir_all(&dir).unwrap();
            dir
        })
        .collect();
    let children: Vec<_> = dirs
        .iter()
        .map(|dir| {
            Command::new(bin())
                .args(["root", "add", &dir.to_string_lossy(), "-o", "json"])
                .env("TASKER_CONFIG_DIR", &config)
                .env("TASKER_ROOT", temp.path())
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    let root = Some(temp.path());
    let roots = ok_workspace(temp.path(), &config, root, &["root", "list", "-o", "json"]);
    assert_eq!(roots.as_array().unwrap().len(), dirs.len());
}

#[cfg(unix)]
#[test]
fn symlinked_workspace_folder_is_ignored() {
    let temp = TempDir::new().unwrap();
    let config = temp.path().join("config");
    let real = temp.path().join("real");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    std::os::unix::fs::symlink(&real, repo.join(".tasker")).unwrap();
    let shown = ok_workspace(&repo, &config, None, &["config", "show", "-o", "json"]);
    assert_ne!(shown["root_source"], "workspace");
}
