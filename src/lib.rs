mod app;
mod cli;
mod error;
mod model;
mod storage;

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
        OutputFormat::Human => human_value(value),
    }
}

pub fn render_error(error: &AppError, format: OutputFormat, pretty: bool) -> String {
    if error.code == "validation_failed"
        && let Some(result) = error.details.get("result")
        && format != OutputFormat::Human
    {
        return render_success(result, format, pretty);
    }
    match format {
        OutputFormat::Human => format!("error [{}]: {}", error.code, error.message),
        _ => render_success(&error.value(), format, pretty),
    }
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
