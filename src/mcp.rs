//! Synchronous Model Context Protocol server over stdin/stdout.
//!
//! Each newline-delimited JSON-RPC request is answered in order. Tool calls
//! are translated into CLI argument vectors and run through `execute`, so
//! validation, locking, workflow, and claim rules are exactly the CLI's. The
//! server opens no socket, keeps no state, and returns when stdin closes.

use crate::cli::{Cli, OutputFormat};
use crate::error::AppError;
use crate::storage;
use clap::Parser;
use serde_json::{Map, Value, json};
use std::io::{BufRead, Write};

const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const INSTRUCTIONS: &str = "Tasker is a local-first task board. The projects root defaults to the nearest .tasker folder in the workspace. Typical loop: tasker_list_tasks or tasker_next, tasker_claim, tasker_get_task and tasker_status, tasker_context_add while working, tasker_acceptance check, then tasker_transition to done. Pass if_revision on mutations to avoid overwriting concurrent edits. tasker_cli runs any other tasker command.";

/// Serves until stdin closes. `root` pins the projects root for every call.
pub fn serve(root: Option<&str>) -> std::io::Result<()> {
    if let Some(root) = root {
        storage::override_projects_root(storage::expand_path(root));
    }
    storage::reserve_stdin();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = handle_line(&line) {
            writeln!(stdout, "{response}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

fn handle_line(line: &str) -> Option<Value> {
    let request: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(e) => return Some(rpc_error(Value::Null, -32700, &format!("Parse error: {e}"))),
    };
    let Some(object) = request.as_object() else {
        return Some(rpc_error(Value::Null, -32600, "Invalid request"));
    };
    let method = object.get("method").and_then(Value::as_str).unwrap_or("");
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    // Requests carry an id; notifications never receive a response.
    let id = object.get("id").cloned()?;
    let result = match method {
        "initialize" => Ok(initialize(&params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tool_definitions()})),
        "tools/call" => call_tool(&params),
        _ => Err((-32601, format!("Method not found: {method}"))),
    };
    Some(match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err((code, message)) => rpc_error(id, code, &message),
    })
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn initialize(params: &Value) -> Value {
    let requested = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let version = PROTOCOL_VERSIONS
        .iter()
        .find(|v| **v == requested)
        .unwrap_or(&PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": "tasker", "version": env!("CARGO_PKG_VERSION")},
        "instructions": INSTRUCTIONS,
    })
}

fn call_tool(params: &Value) -> std::result::Result<Value, (i64, String)> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or((-32602, "tools/call requires a tool name".to_string()))?;
    let empty = Map::new();
    let arguments = params
        .get("arguments")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    if !tool_definitions().iter().any(|tool| tool["name"] == name) {
        return Err((-32602, format!("Unknown tool: {name}")));
    }
    let outcome = build_args(name, &Args(arguments))
        .map_err(AppError::input)
        .and_then(run);
    Ok(match outcome {
        Ok(value) => {
            let mut result = json!({
                "content": [{"type": "text", "text": value.to_string()}],
                "isError": false,
            });
            if value.is_object() {
                result["structuredContent"] = value;
            }
            result
        }
        Err(error) => json!({
            "content": [{"type": "text", "text": crate::render_error(&error, OutputFormat::Json, false)}],
            "isError": true,
        }),
    })
}

fn run(args: Vec<String>) -> crate::error::Result<Value> {
    let mut argv = vec!["tasker".to_string(), "-o".to_string(), "json".to_string()];
    argv.extend(args);
    let cli = Cli::try_parse_from(&argv).map_err(|e| AppError::input(e.to_string()))?;
    crate::execute(&cli)
}

/// Typed accessors over tool arguments.
struct Args<'a>(&'a Map<String, Value>);
impl Args<'_> {
    fn text(&self, key: &str) -> Option<String> {
        match self.0.get(key)? {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }
    fn required(&self, key: &str) -> Result<String, String> {
        self.text(key)
            .ok_or_else(|| format!("Missing required argument {key}"))
    }
    fn list(&self, key: &str) -> Vec<String> {
        match self.0.get(key) {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
            Some(Value::String(s)) => vec![s.clone()],
            _ => vec![],
        }
    }
    fn flag(&self, key: &str) -> bool {
        self.0.get(key).and_then(Value::as_bool).unwrap_or(false)
    }
}

/// Command words, then `--flag=value` options, then `--` and positionals, so
/// free text beginning with `-` is never mistaken for an option.
struct Argv {
    words: Vec<String>,
    options: Vec<String>,
    positionals: Vec<String>,
}
impl Argv {
    fn new(words: &[&str]) -> Self {
        Self {
            words: words.iter().map(|w| w.to_string()).collect(),
            options: vec![],
            positionals: vec![],
        }
    }
    fn opt(mut self, flag: &str, value: Option<String>) -> Self {
        if let Some(value) = value {
            self.options.push(format!("--{flag}={value}"));
        }
        self
    }
    fn opts(mut self, flag: &str, values: Vec<String>) -> Self {
        for value in values {
            self.options.push(format!("--{flag}={value}"));
        }
        self
    }
    fn switch(mut self, flag: &str, on: bool) -> Self {
        if on {
            self.options.push(format!("--{flag}"));
        }
        self
    }
    fn pos(mut self, value: Option<String>) -> Self {
        self.positionals.extend(value);
        self
    }
    /// Adds the shared `project`, `actor`, and (when supported) `if_revision`.
    fn common(self, args: &Args, revision: bool) -> Self {
        let this = self
            .opt("project", args.text("project"))
            .opt("actor", args.text("actor"));
        if revision {
            this.opt("if-revision", args.text("if_revision"))
        } else {
            this
        }
    }
    fn finish(self) -> Vec<String> {
        let mut out = self.words;
        out.extend(self.options);
        if !self.positionals.is_empty() {
            out.push("--".into());
            out.extend(self.positionals);
        }
        out
    }
}

fn build_args(name: &str, a: &Args) -> Result<Vec<String>, String> {
    let argv = match name {
        "tasker_list_boards" => Argv::new(&["list"]).switch("stats", a.flag("stats")),
        "tasker_list_tasks" => task_filters(Argv::new(&["list", "tasks"]), a).common(a, false),
        "tasker_get_task" => Argv::new(&["get", "task"])
            .common(a, false)
            .pos(Some(a.required("id")?)),
        "tasker_status" => Argv::new(&["status"])
            .common(a, false)
            .pos(Some(a.required("id")?)),
        "tasker_next" => Argv::new(&["next"])
            .opt("as", a.text("as"))
            .opt("limit", a.text("limit"))
            .common(a, false),
        "tasker_claim" => Argv::new(&["claim"])
            .opt("as", Some(a.required("as")?))
            .common(a, true)
            .pos(Some(a.text("id").unwrap_or_else(|| "next".into()))),
        "tasker_create_task" => Argv::new(&["create", "task"])
            .opt("description", a.text("description"))
            .opts("tag", a.list("tags"))
            .opts("acceptance-criterion", a.list("acceptance_criteria"))
            .opts("context-ref", a.list("context_refs"))
            .common(a, false)
            .pos(Some(a.required("header")?)),
        "tasker_update_task" => Argv::new(&["update", "task"])
            .opt("header", a.text("header"))
            .opt("description", a.text("description"))
            .opts("tag", a.list("tags"))
            .switch("clear-tags", a.flag("clear_tags"))
            .common(a, true)
            .pos(Some(a.required("id")?)),
        "tasker_transition" => Argv::new(&["transition"])
            .common(a, true)
            .pos(Some(a.required("id")?))
            .pos(Some(a.required("state")?)),
        "tasker_acceptance" => {
            let action = a.required("action")?;
            let argv = Argv::new(&["acceptance", action.as_str()]);
            match action.as_str() {
                "list" => argv.common(a, false).pos(Some(a.required("id")?)),
                "add" => argv
                    .common(a, true)
                    .pos(Some(a.required("id")?))
                    .pos(Some(a.required("text")?)),
                "check" | "uncheck" | "remove" => argv
                    .common(a, true)
                    .pos(Some(a.required("id")?))
                    .pos(Some(a.required("criterion")?)),
                other => return Err(format!("Unknown acceptance action {other}")),
            }
        }
        "tasker_context_add" => Argv::new(&["context", "add"])
            .opt("kind", a.text("kind"))
            .common(a, true)
            .pos(Some(a.required("id")?))
            .pos(Some(a.required("text")?)),
        "tasker_dependency" => {
            let action = a.required("action")?;
            let argv = Argv::new(&["dependency", action.as_str()]);
            match action.as_str() {
                "add" | "remove" => argv
                    .common(a, true)
                    .pos(Some(a.required("id")?))
                    .pos(Some(a.required("depends_on")?)),
                "list" => argv
                    .switch("recursive", a.flag("recursive"))
                    .switch("reverse", a.flag("reverse"))
                    .common(a, false)
                    .pos(Some(a.required("id")?)),
                "check" => argv.common(a, false).pos(Some(a.required("id")?)),
                other => return Err(format!("Unknown dependency action {other}")),
            }
        }
        "tasker_search" => task_filters(Argv::new(&["search", "tasks"]), a)
            .common(a, false)
            .pos(a.text("query")),
        "tasker_brief" => Argv::new(&["brief"])
            .opt("max-bytes", a.text("max_bytes"))
            .common(a, false)
            .pos(a.text("id")),
        "tasker_validate" => Argv::new(&["validate"]).common(a, false),
        "tasker_changelog" => Argv::new(&["changelog"])
            .opt("task", a.text("task"))
            .opt("limit", a.text("limit"))
            .opt("project", a.text("project")),
        "tasker_init" => Argv::new(&["init"])
            .opt("path", a.text("path"))
            .opt("name", a.text("name"))
            .opt("prefix", a.text("prefix"))
            .opt("actor", a.text("actor")),
        "tasker_cli" => return passthrough(a.list("args")),
        other => return Err(format!("Unknown tool {other}")),
    };
    Ok(argv.finish())
}

fn task_filters(argv: Argv, a: &Args) -> Argv {
    argv.opts("state", a.list("states"))
        .opts("tag", a.list("tags"))
        .opt("assignee", a.text("assignee"))
        .switch("unassigned", a.flag("unassigned"))
        .switch("ready", a.flag("ready"))
        .opt("limit", a.text("limit"))
        .switch("full", a.flag("full"))
}

/// Raw CLI arguments, minus commands that would fight the protocol for the
/// process (ui, mcp), persist global settings, or choose an output format.
fn passthrough(args: Vec<String>) -> Result<Vec<String>, String> {
    let Some(first) = args.first() else {
        return Err("tasker_cli requires a non-empty args array".into());
    };
    if matches!(first.as_str(), "ui" | "mcp")
        || (first == "config" && args.get(1).is_some_and(|a| a == "set"))
    {
        return Err(format!("tasker {first} is not available through MCP"));
    }
    if args
        .iter()
        .any(|a| a == "-o" || a == "--output" || a.starts_with("--output=") || a.starts_with("-o="))
    {
        return Err("tasker_cli always returns JSON; omit -o/--output".into());
    }
    Ok(args)
}

fn schema(properties: Value, required: &[&str], common: bool, revision: bool) -> Value {
    let mut properties = properties.as_object().cloned().unwrap_or_default();
    if common {
        properties.insert(
            "project".into(),
            json!({"type": "string", "description": "Project prefix, folder, name, or path; inferred from task IDs when omitted"}),
        );
        properties.insert(
            "actor".into(),
            json!({"type": "string", "description": "Attribution actor; defaults to TASKER_ACTOR or the OS user"}),
        );
    }
    if revision {
        properties.insert(
            "if_revision".into(),
            json!({"type": "integer", "description": "Reject the change unless the task is still at this revision"}),
        );
    }
    json!({"type": "object", "properties": properties, "required": required})
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({"name": name, "description": description, "inputSchema": input_schema})
}

fn filter_properties() -> Value {
    json!({
        "states": {"type": "array", "items": {"type": "string"}, "description": "Match any of these workflow states"},
        "tags": {"type": "array", "items": {"type": "string"}, "description": "Require all of these tags"},
        "assignee": {"type": "string"},
        "unassigned": {"type": "boolean"},
        "ready": {"type": "boolean", "description": "Only tasks whose dependencies are satisfied"},
        "limit": {"type": "integer"},
        "full": {"type": "boolean", "description": "Return complete task objects"}
    })
}

fn tool_definitions() -> Vec<Value> {
    let id = json!({"type": "string", "description": "Task ID such as APP-12"});
    let text = |d: &str| json!({"type": "string", "description": d});
    let strings = json!({"type": "array", "items": {"type": "string"}});
    let mut filters = filter_properties();
    let mut search = filters.as_object().cloned().unwrap_or_default();
    search.insert(
        "query".into(),
        text("Case-insensitive terms matched with AND"),
    );
    vec![
        tool(
            "tasker_list_boards",
            "List every known board: the workspace or effective projects root plus registered roots, each with its projects.",
            schema(json!({"stats": {"type": "boolean"}}), &[], false, false),
        ),
        tool(
            "tasker_list_tasks",
            "List compact task summaries in a project with optional filters.",
            schema(filters.take(), &[], true, false),
        ),
        tool(
            "tasker_get_task",
            "Read a complete task including description, acceptance criteria, context, dependencies, relations, and revision.",
            schema(json!({"id": id}), &["id"], true, false),
        ),
        tool(
            "tasker_status",
            "Show unresolved dependencies, acceptance completion, allowed transitions, and claimability for a task.",
            schema(json!({"id": id}), &["id"], true, false),
        ),
        tool(
            "tasker_next",
            "Preview actionable work without mutating anything.",
            schema(
                json!({"as": text("Existing user to evaluate work for"), "limit": {"type": "integer"}}),
                &[],
                true,
                false,
            ),
        ),
        tool(
            "tasker_claim",
            "Atomically claim a task (or the next actionable task when id is omitted), assigning it and moving it to the claim state.",
            schema(
                json!({"id": text("Task ID; omit to claim the next actionable task"), "as": text("Claiming user; created when unknown")}),
                &["as"],
                true,
                true,
            ),
        ),
        tool(
            "tasker_create_task",
            "Create a task with an explicit contract: header, description, tags, and measurable acceptance criteria.",
            schema(
                json!({
                    "header": text("Short task title"),
                    "description": text("What must be done"),
                    "tags": strings,
                    "acceptance_criteria": strings,
                    "context_refs": {"type": "array", "items": {"type": "string"}, "description": "ID[:role] links to resources or decisions"}
                }),
                &["header"],
                true,
                false,
            ),
        ),
        tool(
            "tasker_update_task",
            "Update a task's header, description, or tags.",
            schema(
                json!({"id": id, "header": text("New header"), "description": text("New description"), "tags": strings, "clear_tags": {"type": "boolean"}}),
                &["id"],
                true,
                true,
            ),
        ),
        tool(
            "tasker_transition",
            "Move a task to another workflow state allowed by the project's configured transitions.",
            schema(
                json!({"id": id, "state": text("Target state")}),
                &["id", "state"],
                true,
                true,
            ),
        ),
        tool(
            "tasker_acceptance",
            "List, add, check, uncheck, or remove acceptance criteria. Check a criterion only after evidence verifies it.",
            schema(
                json!({
                    "action": {"type": "string", "enum": ["list", "add", "check", "uncheck", "remove"]},
                    "id": id,
                    "text": text("Criterion text for add"),
                    "criterion": text("Criterion ID such as AC-1")
                }),
                &["action", "id"],
                true,
                true,
            ),
        ),
        tool(
            "tasker_context_add",
            "Append a curated progress, decision, or context entry to a task for future agents.",
            schema(
                json!({"id": id, "text": text("Entry text"), "kind": {"type": "string", "enum": ["context", "progress", "decision"]}}),
                &["id", "text"],
                true,
                true,
            ),
        ),
        tool(
            "tasker_dependency",
            "Add, remove, list, or check blocking dependencies. add means id depends on depends_on.",
            schema(
                json!({
                    "action": {"type": "string", "enum": ["add", "remove", "list", "check"]},
                    "id": id,
                    "depends_on": text("Prerequisite task ID for add/remove"),
                    "recursive": {"type": "boolean"},
                    "reverse": {"type": "boolean"}
                }),
                &["action", "id"],
                true,
                true,
            ),
        ),
        tool(
            "tasker_search",
            "Search task content, acceptance criteria, and context with optional filters.",
            schema(Value::Object(search), &[], true, false),
        ),
        tool(
            "tasker_brief",
            "Assemble a bounded agent brief for a task or, without id, for the project.",
            schema(
                json!({"id": text("Task ID; omit for a project brief"), "max_bytes": {"type": "integer"}}),
                &[],
                true,
                false,
            ),
        ),
        tool(
            "tasker_validate",
            "Validate project files, workflow, and graph invariants.",
            schema(json!({}), &[], true, false),
        ),
        tool(
            "tasker_changelog",
            "Read the immutable project audit trail, optionally for one task.",
            schema(
                json!({"project": text("Project selector"), "task": text("Task ID"), "limit": {"type": "integer"}}),
                &[],
                false,
                false,
            ),
        ),
        tool(
            "tasker_init",
            "Create a .tasker board at the Git top level (or path), register it, and optionally create its first project.",
            schema(
                json!({"path": text("Directory receiving .tasker"), "name": text("First project name"), "prefix": text("First project prefix"), "actor": text("Attribution actor")}),
                &[],
                false,
                false,
            ),
        ),
        tool(
            "tasker_cli",
            "Run any other tasker command, for example [\"relation\", \"add\", \"APP-3\", \"subtask_of\", \"APP-1\"]. Output is always JSON; ui, mcp, and config set are unavailable, and stdin input is not supported (use --file or --content).",
            schema(json!({"args": strings}), &["args"], false, false),
        ),
    ]
}
