mod api;
mod assets;
mod lifecycle;
mod server;
mod state;
mod watch;

use crate::cli::{Cli, UiCommand};
use crate::error::{AppError, Result};
use serde_json::{Value, json};

pub fn command(cli: &Cli, command: &UiCommand) -> Result<Value> {
    if cli.project.is_some() {
        return Err(AppError::input(
            "Tasker UI lifecycle commands are projects-root scoped; do not use --project",
        ));
    }
    match command {
        UiCommand::Start { open } => value(lifecycle::start(*open)?),
        UiCommand::Stop => value(lifecycle::stop()?),
        UiCommand::Restart { open } => value(lifecycle::restart(*open)?),
        UiCommand::Status => value(lifecycle::status()?),
        UiCommand::Serve {
            projects_root,
            launch_id,
        } => match server::serve(projects_root, launch_id) {
            Ok(()) => Ok(Value::Null),
            Err(error) => {
                lifecycle::write_launch_failure(projects_root, launch_id, &error);
                Err(error)
            }
        },
    }
}

fn value(value: state::LifecycleResult) -> Result<Value> {
    serde_json::to_value(value)
        .map_err(|error| AppError::input(format!("cannot serialize UI lifecycle result: {error}")))
}

pub fn human_lifecycle(value: &Value) -> Option<String> {
    let operation = value.get("operation")?.as_str()?;
    let running = value.get("running")?.as_bool()?;
    let changed = value.get("changed")?.as_bool()?;
    let text = match (operation, running, changed) {
        ("start", true, true) => format!("Tasker UI started: {}", value["url"].as_str()?),
        ("start", true, false) => {
            format!("Tasker UI already running: {}", value["url"].as_str()?)
        }
        ("restart", true, _) => format!("Tasker UI restarted: {}", value["url"].as_str()?),
        ("status", true, _) => format!(
            "Tasker UI running: {} (PID {})",
            value["url"].as_str()?,
            value["pid"].as_u64()?
        ),
        ("stop", false, true) => "Tasker UI stopped.".to_string(),
        ("stop" | "status", false, false) => "Tasker UI is not running.".to_string(),
        _ => return Some(json!(value).to_string()),
    };
    let warnings = value["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|warning| format!("warning: {warning}"))
        .collect::<Vec<_>>();
    Some(if warnings.is_empty() {
        text
    } else {
        format!("{text}\n{}", warnings.join("\n"))
    })
}
