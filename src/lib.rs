mod app;
mod cli;
mod error;
mod knowledge;
mod model;
mod storage;
mod transaction;

pub use app::execute;
pub use cli::{Cli, OutputFormat};
pub use error::AppError;

use serde_json::Value;

pub fn render_success(value: &Value, format: OutputFormat, pretty: bool) -> String {
    match format {
        OutputFormat::Json => if pretty {
            serde_json::to_string_pretty(value)
        } else {
            serde_json::to_string(value)
        }
        .unwrap_or_else(|_| "null".to_string()),
        OutputFormat::Yaml => serde_yaml_ng::to_string(value)
            .unwrap_or_else(|_| "null\n".to_string())
            .trim_end()
            .to_string(),
        OutputFormat::Human | OutputFormat::Markdown => human_value(value),
    }
}

pub fn render_command_success(cli: &Cli, value: &Value) -> String {
    let format = cli.effective_output();
    if matches!(
        (&cli.command, format),
        (
            cli::Command::Project {
                command: cli::ProjectCommand::Brief {
                    command: cli::ProjectBriefCommand::Show,
                },
            },
            OutputFormat::Human | OutputFormat::Markdown
        )
    ) {
        return value
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
    }
    let text = match (&cli.command, format) {
        (cli::Command::Brief(_), OutputFormat::Human | OutputFormat::Markdown) => {
            render_brief_markdown(value)
        }
        (
            cli::Command::Get {
                command: cli::GetCommand::Project { .. },
            },
            OutputFormat::Human | OutputFormat::Markdown,
        ) => format!(
            "{}\n\nUse `tasker brief -p {}` for assembled project knowledge.",
            human_value(value),
            value["prefix"].as_str().unwrap_or("PROJECT")
        ),
        _ => render_success(value, format, cli.pretty),
    };
    format!("{}\n", text.trim_end_matches(['\r', '\n']))
}

pub fn render_error_line(error: &AppError, format: OutputFormat, pretty: bool) -> String {
    format!("{}\n", render_error(error, format, pretty).trim_end())
}

pub fn render_error(error: &AppError, format: OutputFormat, pretty: bool) -> String {
    if error.code == "validation_failed"
        && let Some(result) = error.details.get("result")
        && format != OutputFormat::Human
    {
        return render_success(result, format, pretty);
    }
    match format {
        OutputFormat::Human | OutputFormat::Markdown => {
            format!("error [{}]: {}", error.code, error.message)
        }
        _ => render_success(&error.value(), format, pretty),
    }
}

fn render_brief_markdown(value: &Value) -> String {
    let project = &value["project"];
    let mut output = format!(
        "# Project brief: {} ({})\n\n",
        project["name"].as_str().unwrap_or("Project"),
        project["id"].as_str().unwrap_or("")
    );
    if let Some(task) = value.get("task").filter(|task| !task.is_null()) {
        output.push_str(&format!(
            "## Task {} — {}\n\n**State:** {}  \n**Assignee:** {}\n\n{}\n\n",
            task["id"].as_str().unwrap_or(""),
            task["header"].as_str().unwrap_or(""),
            task["state"].as_str().unwrap_or(""),
            task["assignee"].as_str().unwrap_or("unassigned"),
            task["description"].as_str().unwrap_or("")
        ));
        if let Some(criteria) = value["acceptance_criteria"].as_array()
            && !criteria.is_empty()
        {
            output.push_str("### Acceptance criteria\n\n");
            for criterion in criteria {
                output.push_str(&format!(
                    "- [{}] {}: {}\n",
                    if criterion["completed"].as_bool().unwrap_or(false) {
                        "x"
                    } else {
                        " "
                    },
                    criterion["id"].as_str().unwrap_or(""),
                    criterion["text"].as_str().unwrap_or("")
                ));
            }
            output.push('\n');
        }
    }
    let project_brief = value["project_brief"]["content"].as_str().unwrap_or("");
    if !project_brief.is_empty() {
        output.push_str("## PROJECT.md\n\n");
        output.push_str(project_brief);
        output.push_str("\n\n");
    }
    if let Some(decisions) = value["decisions"].as_object() {
        let mut any = false;
        for section in ["linked", "project", "proposed"] {
            if let Some(items) = decisions.get(section).and_then(Value::as_array) {
                for decision in items {
                    if !any {
                        output.push_str("## Decisions\n\n");
                        any = true;
                    }
                    output.push_str(&format!(
                        "### {} — {}\n\n",
                        decision["id"].as_str().unwrap_or(""),
                        decision["title"].as_str().unwrap_or("")
                    ));
                    if let Some(content) = decision["content"].as_str() {
                        output.push_str(content);
                        output.push_str("\n\n");
                    }
                }
            }
        }
    }
    if let Some(resources) = value["resources"].as_array()
        && !resources.is_empty()
    {
        output.push_str("## Resources\n\n");
        for resource in resources {
            output.push_str(&format!(
                "### {} — {}\n\n",
                resource["id"].as_str().unwrap_or(""),
                resource["title"].as_str().unwrap_or("")
            ));
            if let Some(content) = resource["content"].as_str() {
                output.push_str(content);
                output.push_str("\n\n");
            }
        }
    }
    if let Some(status) = value
        .get("dependency_status")
        .filter(|status| !status.is_null())
    {
        output.push_str("## Dependency status\n\n");
        output.push_str(&format!(
            "**Blocked:** {}\n\n",
            status["blocked"].as_bool().unwrap_or(false)
        ));
        if let Some(dependencies) = value["dependencies"].as_array() {
            for dependency in dependencies {
                output.push_str(&format!(
                    "- {} — {} ({})\n",
                    dependency["id"].as_str().unwrap_or(""),
                    dependency["header"].as_str().unwrap_or(""),
                    dependency["state"].as_str().unwrap_or("")
                ));
            }
            if !dependencies.is_empty() {
                output.push('\n');
            }
        }
    }
    if let Some(relations) = value["relations"].as_array()
        && !relations.is_empty()
    {
        output.push_str("## Relations\n\n");
        for relation in relations {
            output.push_str(&format!(
                "- `{}` → {}\n",
                relation["type"].as_str().unwrap_or("relation"),
                relation["target"].as_str().unwrap_or("")
            ));
        }
        output.push('\n');
    }
    if let Some(context) = value["task_context"].as_array()
        && !context.is_empty()
    {
        output.push_str("## Task context\n\n");
        for entry in context {
            output.push_str(&format!(
                "- **{} ({})** {}\n",
                entry["id"].as_str().unwrap_or(""),
                entry["kind"].as_str().unwrap_or(""),
                entry["message"].as_str().unwrap_or("")
            ));
        }
        output.push('\n');
    }
    if let Some(warnings) = value["warnings"].as_array()
        && !warnings.is_empty()
    {
        output.push_str("## Warnings\n\n");
        for warning in warnings {
            output.push_str(&format!(
                "- `{}` {}\n",
                warning["code"].as_str().unwrap_or("warning"),
                warning["message"].as_str().unwrap_or("")
            ));
        }
        output.push('\n');
    }
    if value["truncation"]["truncated"].as_bool() == Some(true) {
        let omitted = value["truncation"]["omitted"]
            .as_array()
            .map(Vec::len)
            .unwrap_or(0);
        output.push_str("## Truncation\n\n");
        output.push_str(&format!(
            "> Truncated: {omitted} field(s) omitted to fit --max-bytes {}.\n",
            value["truncation"]["max_bytes"].as_u64().unwrap_or(0)
        ));
    }
    output
}

fn human_value(value: &Value) -> String {
    match value {
        Value::Array(items) if items.is_empty() => "No results.".to_string(),
        Value::Array(items) => items.iter().map(human_line).collect::<Vec<_>>().join("\n"),
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        _ => serde_json::to_string_pretty(value).unwrap_or_default(),
    }
}

fn human_line(value: &Value) -> String {
    let Some(object) = value.as_object() else {
        return value.to_string();
    };
    if let (Some(id), Some(state), Some(header)) = (
        object.get("id").and_then(Value::as_str),
        object.get("state").and_then(Value::as_str),
        object.get("header").and_then(Value::as_str),
    ) {
        let assignee = object
            .get("assignee")
            .and_then(Value::as_str)
            .unwrap_or("-");
        return format!("{id:<10} {state:<12} {assignee:<16} {header}");
    }
    if let (Some(name), Some(prefix), Some(path)) = (
        object.get("name").and_then(Value::as_str),
        object.get("prefix").and_then(Value::as_str),
        object.get("path").and_then(Value::as_str),
    ) {
        return format!("{prefix:<10} {name:<24} {path}");
    }
    if let (Some(id), Some(text), Some(completed)) = (
        object.get("id").and_then(Value::as_str),
        object.get("text").and_then(Value::as_str),
        object.get("completed").and_then(Value::as_bool),
    ) {
        return format!("[{}] {id:<8} {text}", if completed { "x" } else { " " });
    }
    if let (Some(id), Some(kind), Some(message)) = (
        object.get("id").and_then(Value::as_str),
        object.get("kind").and_then(Value::as_str),
        object.get("message").and_then(Value::as_str),
    ) {
        let actor = object
            .get("created_by")
            .and_then(Value::as_str)
            .unwrap_or("-");
        return format!("{id:<8} {kind:<10} {actor:<16} {message}");
    }
    if let (Some(id), Some(name)) = (
        object.get("id").and_then(Value::as_str),
        object.get("name").and_then(Value::as_str),
    ) {
        return format!("{id:<20} {name}");
    }
    serde_json::to_string(value).unwrap_or_default()
}
