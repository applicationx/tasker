use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ValueEnum, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum ResourceKind {
    Brief,
    Requirement,
    Design,
    Research,
    Plan,
    Reference,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ValueEnum, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum ResourceStatus {
    Draft,
    Active,
    Archived,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ValueEnum, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum DecisionStatus {
    Proposed,
    Accepted,
    Rejected,
    Superseded,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ValueEnum, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum DecisionScope {
    Project,
    Linked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceMeta {
    pub id: String,
    pub title: String,
    pub kind: ResourceKind,
    pub status: ResourceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub include_in_project_brief: bool,
    pub created_at: String,
    pub updated_at: String,
    pub created_by: String,
    pub updated_by: String,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionMeta {
    pub id: String,
    pub title: String,
    pub status: DecisionStatus,
    pub scope: DecisionScope,
    #[serde(default)]
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub created_by: String,
    pub updated_by: String,
    #[serde(default)]
    pub decided_at: Option<String>,
    #[serde(default)]
    pub decided_by: Option<String>,
    #[serde(default)]
    pub supersedes: Vec<String>,
    #[serde(default)]
    pub superseded_by: Option<String>,
    pub revision: u64,
}

#[derive(Debug, Clone)]
pub struct MarkdownDocument<M> {
    pub metadata: M,
    pub body: String,
    pub path: std::path::PathBuf,
}

pub fn normalize_tags(tags: &mut Vec<String>) {
    *tags = tags
        .drain(..)
        .map(|tag| tag.trim().to_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect();
    tags.sort();
    tags.dedup();
}

pub fn numeric_id(id: &str, marker: char) -> Option<u64> {
    let (_, number) = id.rsplit_once(&format!("-{marker}"))?;
    number.parse().ok()
}
