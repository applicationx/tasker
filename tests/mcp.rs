use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use tempfile::TempDir;

struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Server {
    fn start(dir: &Path, config: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_tasker"))
            .arg("mcp")
            .current_dir(dir)
            .env("TASKER_ACTOR", "tester")
            .env("TASKER_CONFIG_DIR", config)
            .env_remove("TASKER_ROOT")
            .env_remove("TASKER_PROJECT")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    fn send(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], id);
        response
    }

    fn call(&mut self, name: &str, arguments: Value) -> (bool, Value) {
        let response = self.request("tools/call", json!({"name": name, "arguments": arguments}));
        let result = &response["result"];
        let text = result["content"][0]["text"].as_str().unwrap();
        (
            result["isError"].as_bool().unwrap(),
            serde_json::from_str(text).unwrap(),
        )
    }
}

#[test]
fn mcp_server_runs_cli_tools_against_the_workspace_board() {
    let temp = TempDir::new().unwrap();
    let config = temp.path().join("config");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let mut server = Server::start(&repo, &config);

    let init = server.request("initialize", json!({"protocolVersion": "2025-06-18"}));
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "tasker");
    server.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    assert_eq!(server.request("ping", json!({}))["result"], json!({}));

    let tools = server.request("tools/list", json!({}));
    let names: Vec<_> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect();
    for expected in [
        "tasker_claim",
        "tasker_create_task",
        "tasker_cli",
        "tasker_init",
    ] {
        assert!(names.iter().any(|name| name == expected), "{expected}");
    }

    // A board initialized mid-session is used by later calls.
    let (error, value) = server.call("tasker_init", json!({"name": "Demo", "prefix": "DEM"}));
    assert!(!error, "{value}");
    assert!(repo.join(".tasker/demo/tasker.yaml").is_file());

    let (error, task) = server.call(
        "tasker_create_task",
        json!({"project": "DEM", "header": "-leading dash header", "acceptance_criteria": ["Works"]}),
    );
    assert!(!error, "{task}");
    assert_eq!(task["id"], "DEM-1");
    assert_eq!(task["header"], "-leading dash header");

    let (error, claimed) = server.call("tasker_claim", json!({"project": "DEM", "as": "agent-a"}));
    assert!(!error, "{claimed}");
    assert_eq!(claimed["assignee"], "agent-a");
    assert_eq!(claimed["state"], "in_progress");

    let (error, value) = server.call(
        "tasker_context_add",
        json!({"id": "DEM-1", "text": "Implemented it", "kind": "progress"}),
    );
    assert!(!error, "{value}");
    let (error, stale) = server.call(
        "tasker_transition",
        json!({"id": "DEM-1", "state": "done", "if_revision": 1}),
    );
    assert!(error);
    assert_eq!(stale["error"]["code"], "revision_conflict");

    let (error, missing) = server.call("tasker_get_task", json!({"id": "DEM-99"}));
    assert!(error);
    assert_eq!(missing["error"]["code"], "task_not_found");

    let (error, listed) = server.call(
        "tasker_cli",
        json!({"args": ["list", "tasks", "-p", "DEM"]}),
    );
    assert!(!error, "{listed}");
    assert_eq!(listed[0]["id"], "DEM-1");
    let (error, refused) = server.call("tasker_cli", json!({"args": ["ui", "start"]}));
    assert!(error, "{refused}");

    // Commands that would read stdin must never consume the protocol stream.
    let (error, _) = server.call(
        "tasker_cli",
        json!({"args": ["create", "task", "-i", "json", "-p", "DEM"]}),
    );
    assert!(error);
    assert_eq!(server.request("ping", json!({}))["result"], json!({}));

    let unknown = server.request("does/not/exist", json!({}));
    assert_eq!(unknown["error"]["code"], -32601);

    drop(server.stdin.take());
    assert!(server.child.wait().unwrap().success());
}
