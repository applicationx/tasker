use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_tasker")
}

struct Fixture {
    _temp: TempDir,
    root: PathBuf,
    runtime: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("projects");
        let runtime = temp.path().join("runtime");
        fs::create_dir_all(&root).unwrap();
        let fixture = Self {
            _temp: temp,
            root,
            runtime,
        };
        fixture.ok(&["create", "project", "Demo", "--prefix", "DEM", "-o", "json"]);
        fixture
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(bin());
        command
            .args(args)
            .env("TASKER_ROOT", &self.root)
            .env("TASKER_UI_RUNTIME", &self.runtime)
            .env("TASKER_ACTOR", "tester")
            .env("TASKER_OUTPUT", "human")
            .env_remove("TASKER_PROJECT");
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    fn ok(&self, args: &[&str]) -> Value {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn start(&self) -> RunningUi<'_> {
        let state = self.ok(&["ui", "start", "-o", "json"]);
        RunningUi {
            fixture: self,
            port: state["port"].as_u64().unwrap() as u16,
            token: None,
        }
    }

    fn state_path(&self) -> PathBuf {
        let mut states = Vec::new();
        visit(&self.runtime, &mut |path| {
            if path.file_name().and_then(|name| name.to_str()) == Some("state.json") {
                states.push(path.to_path_buf());
            }
        });
        assert_eq!(states.len(), 1);
        states.pop().unwrap()
    }
}

struct RunningUi<'a> {
    fixture: &'a Fixture,
    port: u16,
    token: Option<String>,
}

impl RunningUi<'_> {
    fn request(&self, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Http {
        raw_request(self.port, method, path, headers, body)
    }

    fn session(&mut self) -> Value {
        let response = self.request("GET", "/api/v1/session", &[], "");
        assert_eq!(
            response.status,
            200,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        let value: Value = serde_json::from_slice(&response.body).unwrap();
        self.token = Some(value["data"]["token"].as_str().unwrap().to_string());
        value["data"].clone()
    }

    fn mutation(&self, value: Value) -> Http {
        let body = serde_json::to_string(&value).unwrap();
        let origin = format!("http://127.0.0.1:{}", self.port);
        self.request(
            "POST",
            "/api/v1/projects/DEM/mutations",
            &[
                ("Origin", &origin),
                ("Content-Type", "application/json"),
                ("X-Tasker-Token", self.token.as_deref().unwrap()),
                ("Sec-Fetch-Site", "same-origin"),
            ],
            &body,
        )
    }
}

impl Drop for RunningUi<'_> {
    fn drop(&mut self) {
        let _ = self.fixture.run(&["ui", "stop", "-o", "json"]);
    }
}

struct Http {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Http {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

fn raw_request(port: u16, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Http {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(8)))
        .unwrap();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n"
    )
    .unwrap();
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n").unwrap();
    }
    write!(
        stream,
        "Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).unwrap();
    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    let head = String::from_utf8(bytes[..split].to_vec()).unwrap();
    let mut lines = head.lines();
    let status = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|line| {
            line.split_once(':')
                .map(|(name, value)| (name.to_string(), value.trim().to_string()))
        })
        .collect();
    Http {
        status,
        headers,
        body: bytes[split + 4..].to_vec(),
    }
}

#[cfg(windows)]
fn process_thread_count(pid: u32) -> Option<usize> {
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!("(Get-Process -Id {pid}).Threads.Count"),
        ])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().parse().ok())
        .flatten()
}

#[cfg(target_os = "linux")]
fn process_thread_count(pid: u32) -> Option<usize> {
    fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("Threads:")?.trim().parse().ok())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn process_thread_count(_pid: u32) -> Option<usize> {
    None
}

fn visit(path: &Path, operation: &mut impl FnMut(&Path)) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        operation(&path);
        if path.is_dir() {
            visit(&path, operation);
        }
    }
}

#[test]
fn ui_help_lifecycle_concurrency_and_transient_state_contract() {
    let fixture = Fixture::new();
    let root_help = fixture.run(&["--help"]);
    let ui_help = fixture.run(&["ui", "--help"]);
    let root_text = String::from_utf8(root_help.stdout).unwrap();
    let ui_text = String::from_utf8(ui_help.stdout).unwrap();
    assert!(root_text.contains("ui"));
    assert!(ui_text.contains("start"));
    assert!(ui_text.contains("restart"));
    assert!(!root_text.contains("ui serve"));
    assert!(
        !ui_text
            .lines()
            .any(|line| line.trim_start().starts_with("serve"))
    );

    let invalid = fixture.run(&["-p", "DEM", "ui", "status", "-o", "json"]);
    assert_eq!(invalid.status.code(), Some(2));
    assert_eq!(
        serde_json::from_slice::<Value>(&invalid.stderr).unwrap()["error"]["code"],
        "invalid_input"
    );

    let mut first_command = fixture.command(&["ui", "start", "-o", "json"]);
    first_command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let first = first_command.spawn().unwrap();
    let mut second_command = fixture.command(&["ui", "start", "-o", "json"]);
    second_command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let second = second_command.spawn().unwrap();
    let first = first.wait_with_output().unwrap();
    let second = second.wait_with_output().unwrap();
    assert!(first.status.success());
    assert!(second.status.success());
    let first: Value = serde_json::from_slice(&first.stdout).unwrap();
    let second: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_ne!(first["changed"], second["changed"]);
    assert_eq!(first["instance_id"], second["instance_id"]);
    assert_eq!(first["port"], second["port"]);
    assert_eq!(first["address"], "127.0.0.1");
    assert!(first["port"].as_u64().unwrap() > 0);

    let human = fixture.run(&["ui", "status"]);
    assert!(human.status.success());
    assert!(
        String::from_utf8(human.stdout)
            .unwrap()
            .contains("Tasker UI running:")
    );

    let state_path = fixture.state_path();
    let original = fs::read(&state_path).unwrap();
    fs::write(&state_path, b"not-json\n").unwrap();
    let corrupt = fixture.run(&["ui", "status", "-o", "json"]);
    assert_eq!(corrupt.status.code(), Some(4));
    assert!(!corrupt.stderr.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&corrupt.stderr).unwrap()["error"]["code"],
        "ui_state_corrupt"
    );
    fs::write(&state_path, &original).unwrap();
    let mut mismatched: Value = serde_json::from_slice(&original).unwrap();
    mismatched["server_version"] = json!("0.0.9");
    fs::write(
        &state_path,
        format!("{}\n", serde_json::to_string_pretty(&mismatched).unwrap()),
    )
    .unwrap();
    let mismatch = fixture.ok(&["ui", "status", "-o", "json"]);
    assert_eq!(mismatch["running"], true);
    assert_eq!(mismatch["server_version"], "0.0.9");
    assert!(
        mismatch["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning.as_str().unwrap().contains("0.0.9"))
    );
    let old_instance = mismatch["instance_id"].clone();
    let replaced = fixture.ok(&["ui", "start", "-o", "json"]);
    assert_eq!(replaced["changed"], true);
    assert_ne!(replaced["instance_id"], old_instance);
    assert_eq!(replaced["server_version"], env!("CARGO_PKG_VERSION"));

    let replacement_state = fs::read(&state_path).unwrap();
    let mut wrong_nonce: Value = serde_json::from_slice(&replacement_state).unwrap();
    wrong_nonce["token"] = json!("0".repeat(64));
    fs::write(
        &state_path,
        format!("{}\n", serde_json::to_string_pretty(&wrong_nonce).unwrap()),
    )
    .unwrap();
    let unreachable = fixture.run(&["ui", "status", "-o", "json"]);
    assert_eq!(unreachable.status.code(), Some(4));
    assert_eq!(
        serde_json::from_slice::<Value>(&unreachable.stderr).unwrap()["error"]["code"],
        "ui_instance_unreachable"
    );
    fs::write(&state_path, &replacement_state).unwrap();

    let mut old_for_restart: Value = serde_json::from_slice(&replacement_state).unwrap();
    old_for_restart["server_version"] = json!("0.0.8");
    fs::write(
        &state_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&old_for_restart).unwrap()
        ),
    )
    .unwrap();
    let restarted = fixture.ok(&["ui", "restart", "-o", "json"]);
    assert_eq!(restarted["operation"], "restart");
    assert_eq!(restarted["changed"], true);
    assert_eq!(restarted["running"], true);
    let mut old_for_stop: Value = serde_json::from_slice(&fs::read(&state_path).unwrap()).unwrap();
    old_for_stop["server_version"] = json!("0.0.7");
    fs::write(
        &state_path,
        format!("{}\n", serde_json::to_string_pretty(&old_for_stop).unwrap()),
    )
    .unwrap();
    let stopped = fixture.ok(&["ui", "stop", "-o", "json"]);
    assert_eq!(stopped["changed"], true);
    assert_eq!(stopped["running"], false);
    let stopped_again = fixture.ok(&["ui", "stop", "-o", "json"]);
    assert_eq!(stopped_again["changed"], false);

    let lock_parent = fixture
        .runtime
        .join("tasker")
        .join("ui")
        .read_dir()
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(lock_parent.join("state.json"), b"stale-corrupt\n").unwrap();
    let stale = fixture.ok(&["ui", "status", "-o", "json"]);
    assert_eq!(stale["running"], false);
    assert!(!lock_parent.join("state.json").exists());
}

#[test]
fn ui_instances_are_isolated_by_canonical_projects_root() {
    let first = Fixture::new();
    let mut second = Fixture::new();
    second.runtime = first.runtime.clone();
    let first_ui = first.start();
    let second_ui = second.start();
    assert_ne!(first_ui.port, second_ui.port);
    let first_status = first.ok(&["ui", "status", "-o", "json"]);
    let second_status = second.ok(&["ui", "status", "-o", "json"]);
    assert_ne!(first_status["instance_id"], second_status["instance_id"]);
    assert_ne!(
        first_status["projects_root"],
        second_status["projects_root"]
    );
    drop(first_ui);
    assert_eq!(second.ok(&["ui", "status", "-o", "json"])["running"], true);
    drop(second_ui);

    let missing = TempDir::new().unwrap();
    let output = Command::new(bin())
        .args(["ui", "start", "-o", "json"])
        .env("TASKER_ROOT", missing.path().join("does-not-exist"))
        .env("TASKER_UI_RUNTIME", missing.path().join("runtime"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
        "ui_root_unavailable"
    );
    let root_file = missing.path().join("root-file");
    fs::write(&root_file, "not a directory").unwrap();
    let output = Command::new(bin())
        .args(["ui", "status", "-o", "json"])
        .env("TASKER_ROOT", &root_file)
        .env("TASKER_UI_RUNTIME", missing.path().join("runtime-2"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
        "ui_root_unavailable"
    );
}

#[cfg(unix)]
#[test]
fn ui_rejects_non_utf8_canonical_root_display_paths() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let temp = TempDir::new().unwrap();
    let root = temp.path().join(OsStr::from_bytes(b"projects-\xff"));
    fs::create_dir(&root).unwrap();
    let output = Command::new(bin())
        .args(["ui", "status", "-o", "json"])
        .env("TASKER_ROOT", &root)
        .env("TASKER_UI_RUNTIME", temp.path().join("runtime"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
        "ui_root_unavailable"
    );
}

#[test]
fn ui_http_security_snapshots_manual_edits_and_offline_assets() {
    let fixture = Fixture::new();
    fixture.ok(&[
        "create",
        "task",
        "Initial header",
        "-p",
        "DEM",
        "-o",
        "json",
    ]);
    fs::write(fixture.root.join("ignore.txt"), "ignored").unwrap();
    fs::create_dir(fixture.root.join("empty")).unwrap();
    fs::create_dir(fixture.root.join("broken")).unwrap();
    fs::write(
        fixture.root.join("broken").join("tasker.yaml"),
        "not: valid: yaml",
    )
    .unwrap();
    let mut ui = fixture.start();

    let index = ui.request("GET", "/", &[], "");
    assert_eq!(index.status, 200);
    assert!(String::from_utf8_lossy(&index.body).contains("Tasker"));
    let csp = index.header("Content-Security-Policy").unwrap();
    assert!(csp.contains("default-src 'none'"));
    assert!(csp.contains("connect-src 'self'"));
    assert!(index.header("Access-Control-Allow-Origin").is_none());
    assert_eq!(index.header("Cache-Control"), Some("no-store"));
    assert_eq!(ui.request("GET", "/favicon.ico", &[], "").status, 200);
    assert_eq!(ui.request("GET", "/favicon.svg", &[], "").status, 200);
    let app = ui.request("GET", "/assets/app.js", &[], "");
    assert_eq!(app.status, 200);
    assert_eq!(app.header("Cache-Control"), Some("no-store"));
    let app_text = String::from_utf8_lossy(&app.body);
    assert!(!app_text.contains("https://"));
    assert!(!app_text.contains("eval("));
    assert!(app_text.contains("this.sidebar.inert = closedMobile"));
    assert!(app_text.contains("if (this.dirty)"));
    let shared_asset = ui.request("GET", "/assets/views/shared.js", &[], "");
    let shared_text = String::from_utf8_lossy(&shared_asset.body);
    assert!(shared_text.contains("const draftsRemain = ctx.markSaved(form)"));
    assert!(!shared_text.contains("ctx.setDirty(false);\n    ctx.toast"));
    for path in [
        "/index.html",
        "/assets/app.css",
        "/assets/api.js",
        "/assets/dom.js",
        "/assets/model.js",
        "/assets/router.js",
        "/assets/views/board.js",
        "/assets/views/brief.js",
        "/assets/views/dependencies.js",
        "/assets/views/history.js",
        "/assets/views/knowledge.js",
        "/assets/views/overview.js",
        "/assets/views/projects.js",
        "/assets/views/shared.js",
        "/assets/views/task.js",
        "/assets/views/tasks.js",
        "/assets/views/workflow.js",
    ] {
        let asset = ui.request("GET", path, &[], "");
        assert_eq!(asset.status, 200, "{path}");
        assert_eq!(asset.header("Cache-Control"), Some("no-store"), "{path}");
    }
    assert_eq!(ui.request("GET", "/assets/missing.js", &[], "").status, 404);
    assert_eq!(ui.request("POST", "/assets/app.js", &[], "").status, 405);

    let session = ui.session();
    assert_eq!(session["server_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(session["token"].as_str().unwrap().len(), 64);

    let projects = ui.request("GET", "/api/v1/projects", &[], "").json();
    assert_eq!(projects["data"][0]["prefix"], "DEM");
    assert_eq!(projects["data"][0]["stats"]["backlog"], 1);
    let snapshot = ui
        .request("GET", "/api/v1/projects/DEM/snapshot", &[], "")
        .json();
    assert_eq!(
        snapshot["data"]["project"]["workflow"]["initial_state"],
        "backlog"
    );
    assert_eq!(
        snapshot["data"]["tasks"][0]["task"]["header"],
        "Initial header"
    );
    assert_eq!(snapshot["data"]["validation"]["valid"], true);

    let task_path = fixture.root.join("demo").join("tasks").join("DEM-1.json");
    let mut task: Value = serde_json::from_slice(&fs::read(&task_path).unwrap()).unwrap();
    task["header"] = json!("Manual edit");
    fs::write(
        &task_path,
        format!("{}\n", serde_json::to_string_pretty(&task).unwrap()),
    )
    .unwrap();
    let task = ui
        .request("GET", "/api/v1/projects/DEM/tasks/DEM-1", &[], "")
        .json();
    assert_eq!(task["data"]["task"]["header"], "Manual edit");
    assert_eq!(task["data"]["status"]["state"], "backlog");

    let wrong_host =
        raw_request_with_host(ui.port, "GET", "/api/v1/session", "evil.invalid", &[], "");
    assert_eq!(wrong_host.status, 400);
    let traversal = ui.request("GET", "/assets/%2e%2e/tasker.yaml", &[], "");
    assert_eq!(traversal.status, 400);
    let no_token = ui.request("GET", "/api/v1/changes?since=0", &[], "");
    assert_eq!(no_token.status, 403);
    let no_origin = ui.request(
        "POST",
        "/api/v1/projects/DEM/mutations",
        &[
            ("Content-Type", "application/json"),
            ("X-Tasker-Token", ui.token.as_deref().unwrap()),
        ],
        "{}",
    );
    assert_eq!(no_origin.status, 403);
    let origin = format!("http://127.0.0.1:{}", ui.port);
    let invalid_json = ui.request(
        "POST",
        "/api/v1/projects/DEM/mutations",
        &[
            ("Origin", &origin),
            ("Content-Type", "application/json"),
            ("X-Tasker-Token", ui.token.as_deref().unwrap()),
        ],
        "{}",
    );
    assert_eq!(invalid_json.status, 400);
    let cross_site = ui.request(
        "POST",
        "/api/v1/projects/DEM/mutations",
        &[
            ("Origin", &origin),
            ("Content-Type", "application/json"),
            ("X-Tasker-Token", ui.token.as_deref().unwrap()),
            ("Sec-Fetch-Site", "cross-site"),
        ],
        "{}",
    );
    assert_eq!(cross_site.status, 403);
    assert_eq!(
        ui.request("GET", "/__health", &[("X-Tasker-Token", "wrong")], "")
            .status,
        403
    );
    assert_eq!(ui.request("POST", "/api/v1/session", &[], "").status, 405);
    assert_eq!(ui.request("POST", "/api/v1/projects", &[], "").status, 405);
    assert_eq!(ui.request("GET", "/api/v1/unknown", &[], "").status, 404);

    let wrong_type = ui.request(
        "POST",
        "/api/v1/projects/DEM/mutations",
        &[
            ("Origin", &origin),
            ("Content-Type", "text/plain"),
            ("X-Tasker-Token", ui.token.as_deref().unwrap()),
        ],
        "{}",
    );
    assert_eq!(wrong_type.status, 415);
    let get_mutation = ui.request("GET", "/api/v1/projects/DEM/mutations", &[], "");
    assert_eq!(get_mutation.status, 405);
    let missing = ui.request("GET", "/api/v1/projects/DEM/tasks/DEM-99", &[], "");
    assert_eq!(missing.status, 404);
    assert_eq!(missing.json()["error"]["code"], "task_not_found");
}

fn raw_request_with_host(
    port: u16,
    method: &str,
    path: &str,
    host: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Http {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(stream, "{method} {path} HTTP/1.1\r\nHost: {host}\r\n").unwrap();
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n").unwrap();
    }
    write!(
        stream,
        "Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).unwrap();
    parse_http(bytes)
}

fn parse_http(bytes: Vec<u8>) -> Http {
    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    let head = String::from_utf8(bytes[..split].to_vec()).unwrap();
    let mut lines = head.lines();
    let status = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|line| {
            line.split_once(':')
                .map(|(name, value)| (name.to_string(), value.trim().to_string()))
        })
        .collect();
    Http {
        status,
        headers,
        body: bytes[split + 4..].to_vec(),
    }
}

#[test]
fn ui_mutations_use_existing_actors_revisions_events_and_research_boundaries() {
    let fixture = Fixture::new();
    fixture.ok(&[
        "create",
        "task",
        "Editable",
        "-p",
        "DEM",
        "--acceptance-criterion",
        "Verified",
        "-o",
        "json",
    ]);
    let research = fixture.ok(&[
        "create",
        "resource",
        "Research",
        "-p",
        "DEM",
        "--kind",
        "research",
        "--status",
        "active",
        "--content",
        "# Original",
        "-o",
        "json",
    ]);
    fixture.ok(&[
        "create",
        "resource",
        "Design",
        "-p",
        "DEM",
        "--kind",
        "design",
        "--content",
        "# Design",
        "-o",
        "json",
    ]);
    fs::write(fixture.root.join("demo").join("notes.md"), "live\n").unwrap();
    fixture.ok(&[
        "create",
        "resource",
        "Referenced research",
        "-p",
        "DEM",
        "--kind",
        "research",
        "--path",
        "notes.md",
        "-o",
        "json",
    ]);
    fixture.ok(&[
        "create",
        "decision",
        "Read-only decision",
        "-p",
        "DEM",
        "-o",
        "json",
    ]);
    let task = fixture.ok(&[
        "context-ref",
        "add",
        "DEM-1",
        "DEM-R1",
        "--role",
        "informed_by",
        "-o",
        "json",
    ]);

    let mut ui = fixture.start();
    ui.session();
    let snapshot = ui
        .request("GET", "/api/v1/projects/DEM/snapshot", &[], "")
        .json();
    assert_eq!(snapshot["data"]["resources"].as_array().unwrap().len(), 3);
    assert_eq!(snapshot["data"]["decisions"].as_array().unwrap().len(), 1);
    assert_eq!(
        snapshot["data"]["reverse_context_refs"]["DEM-R1"][0]["task_id"],
        "DEM-1"
    );
    let revision = task["revision"].as_u64().unwrap();
    let updated = ui.mutation(json!({
        "operation":"task.update","actor":"tester","task_id":"DEM-1","if_revision":revision,
        "patch":{"header":"Updated","description":"Through the UI","tags":["UI","backend"]}
    }));
    assert_eq!(
        updated.status,
        200,
        "{}",
        String::from_utf8_lossy(&updated.body)
    );
    let task = updated.json()["data"]["task"].clone();
    assert_eq!(task["revision"], revision + 1);
    assert_eq!(task["tags"], json!(["backend", "ui"]));

    let no_op = ui.mutation(json!({
        "operation":"task.update","actor":"tester","task_id":"DEM-1","if_revision":revision + 1,
        "patch":{"header":"Updated","description":"Through the UI","tags":["backend","ui"]}
    }));
    assert_eq!(no_op.status, 200);
    assert_eq!(no_op.json()["data"]["task"]["revision"], revision + 1);
    let stale = ui.mutation(json!({
        "operation":"task.unassign","actor":"tester","task_id":"DEM-1","if_revision":revision
    }));
    assert_eq!(stale.status, 409);
    assert_eq!(stale.json()["error"]["code"], "revision_conflict");
    assert_eq!(stale.json()["error"]["expected_revision"], revision);

    let unknown_actor = ui.mutation(json!({
        "operation":"task.context.append","actor":"missing-user","task_id":"DEM-1",
        "if_revision":revision + 1,"kind":"progress","message":"not written"
    }));
    assert_eq!(unknown_actor.status, 404);
    assert_eq!(unknown_actor.json()["error"]["code"], "user_not_found");
    let current = revision + 1;
    for (payload, status, code) in [
        (
            json!({"operation":"task.update","actor":"tester","task_id":"DEM-1","if_revision":current,"patch":{}}),
            400,
            "invalid_input",
        ),
        (
            json!({"operation":"task.transition","actor":"tester","task_id":"DEM-1","if_revision":current,"target_state":"imaginary"}),
            422,
            "unknown_state",
        ),
        (
            json!({"operation":"task.assign","actor":"tester","task_id":"DEM-1","if_revision":current,"assignee":"missing-user"}),
            404,
            "user_not_found",
        ),
        (
            json!({"operation":"task.acceptance.add","actor":"tester","task_id":"DEM-1","if_revision":current,"text":"Verified"}),
            409,
            "acceptance_criterion_exists",
        ),
        (
            json!({"operation":"task.acceptance.remove","actor":"tester","task_id":"DEM-1","if_revision":current,"criterion_id":"AC-99"}),
            404,
            "acceptance_criterion_not_found",
        ),
        (
            json!({"operation":"task.context.append","actor":"tester","task_id":"DEM-1","if_revision":current,"kind":"context","message":"  "}),
            400,
            "invalid_input",
        ),
    ] {
        let response = ui.mutation(payload);
        assert_eq!(
            response.status,
            status,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        assert_eq!(response.json()["error"]["code"], code);
    }
    let acceptance_noop = ui.mutation(json!({
        "operation":"task.acceptance.set","actor":"tester","task_id":"DEM-1",
        "if_revision":current,"criterion_id":"AC-1","completed":false
    }));
    assert_eq!(acceptance_noop.status, 200);
    assert_eq!(acceptance_noop.json()["data"]["task"]["revision"], current);

    let added = ui.mutation(json!({
        "operation":"task.acceptance.add","actor":"tester","task_id":"DEM-1",
        "if_revision":revision + 1,"text":"Second checkpoint"
    }));
    let mut revision = added.json()["data"]["task"]["revision"].as_u64().unwrap();
    let checked = ui.mutation(json!({
        "operation":"task.acceptance.set","actor":"tester","task_id":"DEM-1",
        "if_revision":revision,"criterion_id":"AC-1","completed":true
    }));
    revision = checked.json()["data"]["task"]["revision"].as_u64().unwrap();
    let removed = ui.mutation(json!({
        "operation":"task.acceptance.remove","actor":"tester","task_id":"DEM-1",
        "if_revision":revision,"criterion_id":"AC-2"
    }));
    revision = removed.json()["data"]["task"]["revision"].as_u64().unwrap();
    let context = ui.mutation(json!({
        "operation":"task.context.append","actor":"tester","task_id":"DEM-1",
        "if_revision":revision,"kind":"progress","message":"UI mutation verified"
    }));
    revision = context.json()["data"]["task"]["revision"].as_u64().unwrap();
    let transitioned = ui.mutation(json!({
        "operation":"task.transition","actor":"tester","task_id":"DEM-1",
        "if_revision":revision,"target_state":"in_progress"
    }));
    revision = transitioned.json()["data"]["task"]["revision"]
        .as_u64()
        .unwrap();
    let assigned = ui.mutation(json!({
        "operation":"task.assign","actor":"tester","task_id":"DEM-1",
        "if_revision":revision,"assignee":"tester"
    }));
    revision = assigned.json()["data"]["task"]["revision"]
        .as_u64()
        .unwrap();
    let unassigned = ui.mutation(json!({
        "operation":"task.unassign","actor":"tester","task_id":"DEM-1","if_revision":revision
    }));
    assert_eq!(unassigned.status, 200);
    assert!(unassigned.json()["data"]["task"]["assignee"].is_null());

    let updated_research = ui.mutation(json!({
        "operation":"research.update","actor":"tester","resource_id":research["id"],
        "if_revision":research["revision"],
        "patch":{"title":"New research","status":"draft","tags":["UI"],
                 "include_in_project_brief":true,"content":"# Changed\n\nEvidence.\n"}
    }));
    assert_eq!(
        updated_research.status,
        200,
        "{}",
        String::from_utf8_lossy(&updated_research.body)
    );
    assert_eq!(
        updated_research.json()["data"]["resource"]["title"],
        "New research"
    );
    assert_eq!(
        updated_research.json()["data"]["resource"]["content"],
        "# Changed\n\nEvidence.\n"
    );
    let research_revision = updated_research.json()["data"]["resource"]["revision"]
        .as_u64()
        .unwrap();
    let research_noop = ui.mutation(json!({
        "operation":"research.update","actor":"tester","resource_id":"DEM-R1",
        "if_revision":research_revision,
        "patch":{"title":"New research","status":"draft","tags":["ui"],
                 "include_in_project_brief":true,"content":"# Changed\n\nEvidence.\n"}
    }));
    assert_eq!(research_noop.status, 200);
    assert_eq!(
        research_noop.json()["data"]["resource"]["revision"],
        research_revision
    );
    for (payload, status, code) in [
        (
            json!({"operation":"research.update","actor":"tester","resource_id":"DEM-R1","if_revision":research_revision,"patch":{}}),
            400,
            "invalid_input",
        ),
        (
            json!({"operation":"research.update","actor":"tester","resource_id":"DEM-R1","if_revision":research_revision,"patch":{"status":"archived"}}),
            422,
            "research_archive_read_only",
        ),
        (
            json!({"operation":"research.update","actor":"tester","resource_id":"DEM-R1","if_revision":1,"patch":{"title":"stale"}}),
            409,
            "revision_conflict",
        ),
    ] {
        let response = ui.mutation(payload);
        assert_eq!(response.status, status);
        assert_eq!(response.json()["error"]["code"], code);
    }
    let oversized_research = ui.mutation(json!({
        "operation":"research.update","actor":"tester","resource_id":"DEM-R1",
        "if_revision":research_revision,"patch":{"content":"x".repeat(1024 * 1024 + 1)}
    }));
    assert_eq!(oversized_research.status, 400);

    let wrong_kind = ui.mutation(json!({
        "operation":"research.update","actor":"tester","resource_id":"DEM-R2","if_revision":1,
        "patch":{"title":"No"}
    }));
    assert_eq!(wrong_kind.status, 422);
    assert_eq!(wrong_kind.json()["error"]["code"], "research_not_editable");
    let referenced = ui.mutation(json!({
        "operation":"research.update","actor":"tester","resource_id":"DEM-R3","if_revision":1,
        "patch":{"content":"outside"}
    }));
    assert_eq!(referenced.status, 422);
    assert_eq!(
        fs::read_to_string(fixture.root.join("demo").join("notes.md")).unwrap(),
        "live\n"
    );

    assert_eq!(
        ui.request("GET", "/api/v1/projects/dem/snapshot", &[], "")
            .status,
        400
    );
    assert_eq!(
        ui.request("GET", "/api/v1/projects/NOPE/snapshot", &[], "")
            .status,
        404
    );
    assert_eq!(
        ui.request("GET", "/api/v1/projects/DEM/tasks/OTHER-1", &[], "")
            .status,
        422
    );
    assert_eq!(
        ui.request("GET", "/api/v1/projects/DEM/resources/DEM-R99", &[], "")
            .status,
        404
    );
    assert_eq!(
        ui.request("GET", "/api/v1/projects/DEM/decisions/DEM-D99", &[], "")
            .status,
        404
    );
    let resource = ui.request("GET", "/api/v1/projects/DEM/resources/DEM-R1", &[], "");
    assert_eq!(resource.status, 200);
    assert_eq!(resource.json()["data"]["source_type"], "managed");
    let referenced_read = ui.request("GET", "/api/v1/projects/DEM/resources/DEM-R3", &[], "");
    assert_eq!(referenced_read.json()["data"]["content"], "live\n");
    let decision = ui.request("GET", "/api/v1/projects/DEM/decisions/DEM-D1", &[], "");
    assert_eq!(decision.status, 200);
    assert!(
        decision.json()["data"]["content"]
            .as_str()
            .unwrap()
            .contains("# Context")
    );
    let project_brief = ui.request("GET", "/api/v1/projects/DEM/project-brief", &[], "");
    assert_eq!(project_brief.status, 200);
    assert_eq!(project_brief.json()["data"]["path"], "PROJECT.md");

    let brief = ui.request(
        "GET",
        "/api/v1/projects/DEM/brief?task_id=DEM-1&max_bytes=65536&format=markdown",
        &[],
        "",
    );
    assert_eq!(
        brief.status,
        200,
        "{}",
        String::from_utf8_lossy(&brief.body)
    );
    assert!(
        brief.json()["data"]["content"]
            .as_str()
            .unwrap()
            .contains("# Project brief:")
    );
    let json_brief = ui.request(
        "GET",
        "/api/v1/projects/DEM/brief?max_bytes=65536&format=json",
        &[],
        "",
    );
    assert_eq!(json_brief.status, 200);
    assert_eq!(json_brief.json()["data"]["project"]["id"], "DEM");
    let yaml_brief = ui.request(
        "GET",
        "/api/v1/projects/DEM/brief?max_bytes=65536&format=yaml",
        &[],
        "",
    );
    assert!(
        yaml_brief.json()["data"]["yaml"]
            .as_str()
            .unwrap()
            .contains("project:")
    );
    assert_eq!(
        ui.request("GET", "/api/v1/projects/DEM/brief?format=xml", &[], "")
            .status,
        400
    );
    assert_eq!(
        ui.request("GET", "/api/v1/projects/DEM/brief?max_bytes=1", &[], "")
            .status,
        400
    );
    assert_eq!(
        ui.request("GET", "/api/v1/projects/DEM/brief?task_id=OTHER-1", &[], "")
            .status,
        422
    );
    assert_eq!(
        ui.request("GET", "/api/v1/projects/DEM/events?limit=501", &[], "")
            .status,
        400
    );
    let events = ui
        .request(
            "GET",
            "/api/v1/projects/DEM/events?offset=0&limit=100",
            &[],
            "",
        )
        .json();
    let actions: Vec<&str> = events["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|event| event["action"].as_str())
        .collect();
    assert!(actions.contains(&"task.updated"));
    assert!(actions.contains(&"task.context_added"));
    assert!(actions.contains(&"resource.updated"));

    let task_path = fixture.root.join("demo").join("tasks").join("DEM-1.json");
    let original_task = fs::read(&task_path).unwrap();
    let mut invalid_task: Value = serde_json::from_slice(&original_task).unwrap();
    invalid_task["state"] = json!("not_configured");
    fs::write(
        &task_path,
        format!("{}\n", serde_json::to_string_pretty(&invalid_task).unwrap()),
    )
    .unwrap();
    let invalid_snapshot = ui
        .request("GET", "/api/v1/projects/DEM/snapshot", &[], "")
        .json();
    assert_eq!(invalid_snapshot["data"]["validation"]["valid"], false);
    fs::write(&task_path, original_task).unwrap();

    let brief_path = fixture.root.join("demo").join("PROJECT.md");
    let original_brief = fs::read(&brief_path).unwrap();
    fs::remove_file(&brief_path).unwrap();
    assert_eq!(
        ui.request("GET", "/api/v1/projects/DEM/project-brief", &[], "")
            .status,
        404
    );
    fs::write(brief_path, original_brief).unwrap();
}

#[test]
fn ui_bounded_ingress_and_change_streams_preserve_lifecycle_capacity() {
    let fixture = Fixture::new();
    let mut ui = fixture.start();
    ui.session();
    let lifecycle = fixture.ok(&["ui", "status", "-o", "json"]);
    let pid = lifecycle["pid"].as_u64().unwrap() as u32;
    let baseline_threads = process_thread_count(pid);

    for _ in 0..2 {
        let mut incomplete = Vec::new();
        for _ in 0..30 {
            let mut stream = TcpStream::connect(("127.0.0.1", ui.port)).unwrap();
            let _ = write!(
                stream,
                "GET /api/v1/session HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nX-Slow: ",
                ui.port
            );
            incomplete.push(stream);
        }
        let status = fixture.run(&["ui", "status", "-o", "json"]);
        assert!(
            status.status.success(),
            "slow headers blocked status: {}",
            String::from_utf8_lossy(&status.stderr)
        );
        drop(incomplete);
    }

    assert_eq!(
        raw_http_bytes(
            ui.port,
            format!("GET / HTTP/1.0\r\nHost: 127.0.0.1:{}\r\n\r\n", ui.port).as_bytes(),
        )
        .status,
        400
    );
    assert_eq!(
        raw_http_bytes(
            ui.port,
            format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n", ui.port).as_bytes(),
        )
        .status,
        400
    );
    assert_eq!(
        raw_http_bytes(
            ui.port,
            format!(
                "POST / HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nTransfer-Encoding: chunked\r\n\r\n",
                ui.port
            )
            .as_bytes(),
        )
        .status,
        400
    );
    assert_eq!(
        raw_http_bytes(
            ui.port,
            format!(
                "POST / HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n",
                ui.port
            )
            .as_bytes(),
        )
        .status,
        400
    );
    assert_eq!(
        raw_http_bytes(
            ui.port,
            format!(
                "POST / HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: invalid\r\n\r\n",
                ui.port
            )
            .as_bytes(),
        )
        .status,
        400
    );
    assert_eq!(
        raw_http_bytes(
            ui.port,
            format!(
                "POST / HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: 5\r\n\r\nx",
                ui.port
            )
            .as_bytes(),
        )
        .status,
        400
    );

    let long_target = format!("/{}", "x".repeat(2200));
    assert_eq!(
        raw_http_bytes(
            ui.port,
            format!(
                "GET {long_target} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n\r\n",
                ui.port
            )
            .as_bytes(),
        )
        .status,
        414
    );
    let mut many_headers = format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n", ui.port);
    for index in 0..70 {
        many_headers.push_str(&format!("X-{index}: value\r\n"));
    }
    many_headers.push_str("\r\n");
    assert_eq!(raw_http_bytes(ui.port, many_headers.as_bytes()).status, 431);
    let oversized_prefix = format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nX-Large: ", ui.port);
    let oversized_header = format!(
        "{oversized_prefix}{}",
        "x".repeat(16 * 1024 - oversized_prefix.len())
    );
    assert_eq!(
        raw_http_bytes(ui.port, oversized_header.as_bytes()).status,
        431
    );
    let oversized_body = format!(
        "POST /api/v1/projects/DEM/mutations HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: {}\r\n\r\n",
        ui.port,
        2 * 1024 * 1024 + 1
    );
    assert_eq!(
        raw_http_bytes(ui.port, oversized_body.as_bytes()).status,
        413
    );
    assert!(
        fixture
            .run(&["ui", "status", "-o", "json"])
            .status
            .success(),
        "oversized parser inputs blocked status"
    );
    thread::sleep(Duration::from_millis(250));
    if let (Some(baseline), Some(after)) = (baseline_threads, process_thread_count(pid)) {
        assert!(
            after <= baseline + 2,
            "adversarial ingress grew persistent threads: {baseline} -> {after}"
        );
    }

    let mut streams = Vec::new();
    for _ in 0..4 {
        streams.push(open_sse(ui.port, ui.token.as_deref().unwrap()));
    }
    let limited = ui.request(
        "GET",
        "/api/v1/changes?since=0",
        &[("X-Tasker-Token", ui.token.as_deref().unwrap())],
        "",
    );
    assert_eq!(limited.status, 429);
    assert_eq!(limited.json()["error"]["code"], "ui_change_stream_limit");
    assert_eq!(
        ui.request("GET", "/api/v1/session", &[], "").status,
        200,
        "ordinary HTTP was stranded by valid change streams"
    );
    let status = fixture.run(&["ui", "status", "-o", "json"]);
    assert!(status.status.success(), "change streams blocked status");
    let stopped = fixture.ok(&["ui", "stop", "-o", "json"]);
    assert_eq!(stopped["changed"], true);
    drop(streams);
}

fn raw_http_bytes(port: u16, request: &[u8]) -> Http {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(8)))
        .unwrap();
    let _ = stream.write_all(request);
    let _ = stream.shutdown(Shutdown::Write);
    let mut response = Vec::new();
    match stream.read_to_end(&mut response) {
        Ok(_) => {}
        Err(error) if !response.is_empty() => {
            eprintln!("connection closed after bounded parser response: {error}");
        }
        Err(error) => panic!("cannot read bounded parser response: {error}"),
    }
    parse_http(response)
}

fn open_sse(port: u16, token: &str) -> TcpStream {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(8)))
        .unwrap();
    write!(
        stream,
        "GET /api/v1/changes?since=0 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Tasker-Token: {token}\r\nConnection: keep-alive\r\n\r\n"
    )
    .unwrap();
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    assert!(
        String::from_utf8(head)
            .unwrap()
            .starts_with("HTTP/1.1 200 ")
    );
    stream
}

#[test]
fn ui_native_watcher_debounces_and_signals_cli_changes_without_polling() {
    let fixture = Fixture::new();
    fixture.ok(&["create", "task", "Watched", "-p", "DEM", "-o", "json"]);
    let mut ui = fixture.start();
    let session = ui.session();
    let generation = session["change_generation"].as_u64().unwrap();
    let port = ui.port;
    let token = ui.token.clone().unwrap();
    let waiter = thread::spawn(move || {
        raw_sse_event(port, &format!("/api/v1/changes?since={generation}"), &token)
    });
    thread::sleep(Duration::from_millis(100));
    fixture.ok(&[
        "update",
        "task",
        "DEM-1",
        "--header",
        "Changed by CLI",
        "-p",
        "DEM",
        "-o",
        "json",
    ]);
    thread::sleep(Duration::from_secs(1));
    let observed =
        ui.request("GET", "/api/v1/session", &[], "").json()["data"]["change_generation"]
            .as_u64()
            .unwrap();
    assert!(observed > generation, "watcher did not advance generation");
    let change = waiter.join().unwrap();
    assert_eq!(
        change.status,
        200,
        "{}",
        String::from_utf8_lossy(&change.body)
    );
    assert_eq!(
        change.header("Content-Type"),
        Some("text/event-stream; charset=utf-8")
    );
    let text = String::from_utf8(change.body).unwrap();
    assert!(text.contains("event: change"));
    assert!(text.contains("filesystem"));
    assert!(text.contains("\"project_prefix\":\"DEM\""));

    fixture.ok(&["create", "project", "Late", "--prefix", "LAT", "-o", "json"]);
    thread::sleep(Duration::from_secs(1));
    let generation =
        ui.request("GET", "/api/v1/session", &[], "").json()["data"]["change_generation"]
            .as_u64()
            .unwrap();
    let port = ui.port;
    let token = ui.token.clone().unwrap();
    let waiter = thread::spawn(move || {
        raw_sse_event(port, &format!("/api/v1/changes?since={generation}"), &token)
    });
    thread::sleep(Duration::from_millis(100));
    fixture.ok(&["create", "task", "Late task", "-p", "LAT", "-o", "json"]);
    let late_change = waiter.join().unwrap();
    let late_text = String::from_utf8(late_change.body).unwrap();
    assert!(late_text.contains("\"project_prefix\":\"LAT\""));

    let reference = fixture.root.join("late").join("live.md");
    fs::write(&reference, "first\n").unwrap();
    fixture.ok(&[
        "create", "resource", "Live", "-p", "LAT", "--kind", "research", "--path", "live.md", "-o",
        "json",
    ]);
    thread::sleep(Duration::from_secs(1));
    let generation =
        ui.request("GET", "/api/v1/session", &[], "").json()["data"]["change_generation"]
            .as_u64()
            .unwrap();
    let port = ui.port;
    let token = ui.token.clone().unwrap();
    let waiter = thread::spawn(move || {
        raw_sse_event(port, &format!("/api/v1/changes?since={generation}"), &token)
    });
    thread::sleep(Duration::from_millis(100));
    fs::write(reference, "second\n").unwrap();
    let reference_change = waiter.join().unwrap();
    assert!(
        String::from_utf8(reference_change.body)
            .unwrap()
            .contains("\"project_prefix\":\"LAT\"")
    );
}

fn raw_sse_event(port: u16, path: &str, token: &str) -> Http {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(8)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Tasker-Token: {token}\r\nConnection: keep-alive\r\n\r\n"
    )
    .unwrap();
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    while !String::from_utf8_lossy(&bytes).contains("event: change") {
        let read = stream.read(&mut buffer).unwrap();
        assert!(read > 0, "SSE stream ended before an invalidation event");
        bytes.extend_from_slice(&buffer[..read]);
    }
    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    let head = String::from_utf8(bytes[..split].to_vec()).unwrap();
    let mut lines = head.lines();
    let status = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|line| {
            line.split_once(':')
                .map(|(name, value)| (name.to_string(), value.trim().to_string()))
        })
        .collect();
    Http {
        status,
        headers,
        body: bytes[split + 4..].to_vec(),
    }
}
