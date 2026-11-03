//! The agent skill embedded in the binary, so installed skills always match
//! the CLI that serves them.

use crate::cli::{SkillAgent, SkillCommand, SkillInstallArgs};
use crate::error::{AppError, ErrorCategory, Result};
use crate::storage;
use directories::BaseDirs;
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

pub const SKILL_MD: &str = include_str!("../skills/tasker/SKILL.md");

pub fn command(command: &SkillCommand) -> Result<Value> {
    match command {
        SkillCommand::Show => Ok(Value::String(SKILL_MD.to_string())),
        SkillCommand::Install(args) => install(args),
    }
}

/// Directory that holds skill folders for an agent: user-level or repository-level.
fn skills_dir(agent: SkillAgent, repo: bool) -> Result<PathBuf> {
    let base = if repo {
        let cwd = std::env::current_dir()
            .map_err(|e| AppError::io(e, "cannot determine current directory"))?;
        cwd.ancestors()
            .find(|dir| dir.join(".git").exists())
            .map(PathBuf::from)
            .unwrap_or(cwd)
    } else {
        BaseDirs::new()
            .map(|dirs| dirs.home_dir().to_path_buf())
            .ok_or_else(|| AppError::input("cannot determine the home directory; use --path"))?
    };
    let relative = match agent {
        SkillAgent::Claude => ".claude/skills",
        SkillAgent::Codex => ".agents/skills",
        SkillAgent::Cursor => ".cursor/skills",
    };
    Ok(base.join(relative))
}

fn install(args: &SkillInstallArgs) -> Result<Value> {
    let dir = match &args.path {
        Some(path) => storage::expand_path(path),
        None => skills_dir(args.agent, args.repo)?,
    };
    let path = dir.join("tasker").join("SKILL.md");
    let existing = fs::read(&path).ok();
    let unchanged = existing.as_deref() == Some(SKILL_MD.as_bytes());
    if existing.is_some() && !unchanged && !args.force {
        return Err(AppError::new(
            "skill_exists",
            format!(
                "{} already exists with different content; pass --force to replace it",
                path.display()
            ),
            ErrorCategory::Conflict,
        )
        .detail("path", path.to_string_lossy().to_string()));
    }
    if !unchanged {
        fs::create_dir_all(path.parent().unwrap_or(&dir))
            .map_err(|e| AppError::io(e, format!("cannot create {}", dir.display())))?;
        storage::atomic_write(&path, SKILL_MD.as_bytes())?;
    }
    Ok(json!({
        "agent": format!("{:?}", args.agent).to_lowercase(),
        "path": path.to_string_lossy(),
        "written": !unchanged,
        "replaced": existing.is_some() && !unchanged,
    }))
}
