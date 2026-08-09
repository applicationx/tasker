use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: u32 = 1;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    pub schema_version: u32,
    pub name: String,
    pub prefix: String,
    pub workflow: Workflow,
    pub relations: BTreeMap<String, RelationType>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workflow {
    pub initial_state: String,
    pub claim_state: String,
    pub states: BTreeMap<String, StateConfig>,
    pub transitions: BTreeMap<String, Vec<String>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateConfig {
    pub label: String,
    pub terminal: bool,
    pub dependency_satisfied: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelationType {
    pub inverse_label: String,
    pub symmetric: bool,
    pub acyclic: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meta {
    pub schema_version: u32,
    pub next_task_number: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct Relation {
    #[serde(rename = "type")]
    pub kind: String,
    pub target: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceCriterion {
    pub id: String,
    pub text: String,
    pub completed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_by: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, clap::ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum ContextKind {
    Context,
    Progress,
    Decision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskContextEntry {
    pub id: String,
    pub kind: ContextKind,
    pub message: String,
    pub created_at: String,
    pub created_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub schema_version: u32,
    pub id: String,
    pub header: String,
    pub description: String,
    #[serde(default)]
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    /// Monotonic allocator; prevents AC-N identifiers from being reused after removal.
    #[serde(default = "default_acceptance_number")]
    pub next_acceptance_number: u64,
    #[serde(default)]
    pub context: Vec<TaskContextEntry>,
    pub state: String,
    pub assignee: Option<String>,
    pub tags: Vec<String>,
    pub dependencies: Vec<String>,
    pub relations: Vec<Relation>,
    pub created_at: String,
    pub created_by: String,
    pub updated_at: String,
    pub updated_by: String,
    pub revision: u64,
}
fn default_acceptance_number() -> u64 {
    1
}

impl Task {
    pub fn normalize(&mut self) {
        self.tags = self
            .tags
            .drain(..)
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        self.tags.sort();
        self.tags.dedup();
        self.dependencies.sort_by(|a, b| task_id_cmp(a, b));
        self.dependencies.dedup();
        let max_acceptance_number = self
            .acceptance_criteria
            .iter()
            .filter_map(|criterion| criterion.id.strip_prefix("AC-")?.parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        self.next_acceptance_number = self.next_acceptance_number.max(max_acceptance_number + 1);
        self.relations.sort();
        self.relations.dedup();
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, clap::ValueEnum, Default)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum UserKind {
    Human,
    Agent,
    #[default]
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct User {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub kind: UserKind,
    pub created_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActorSnapshot {
    pub id: String,
    pub name: String,
}
impl From<&User> for ActorSnapshot {
    fn from(u: &User) -> Self {
        Self {
            id: u.id.clone(),
            name: u.name.clone(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub schema_version: u32,
    pub timestamp: String,
    pub actor: ActorSnapshot,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_revision: Option<u64>,
    pub changes: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct GlobalConfig {
    pub projects_root: Option<String>,
    pub default_output: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct ProjectSummary {
    pub name: String,
    pub prefix: String,
    pub path: String,
}
#[derive(Debug, Serialize)]
pub struct TaskSummary {
    pub id: String,
    pub header: String,
    pub state: String,
    pub assignee: Option<String>,
    pub tags: Vec<String>,
    pub dependency_count: usize,
    pub dependency_blocked: bool,
}
#[derive(Debug, Serialize)]
pub struct StatusView {
    pub id: String,
    pub state: String,
    pub assignee: Option<String>,
    pub acceptance_criteria_total: usize,
    pub acceptance_criteria_completed: usize,
    pub acceptance_criteria_remaining: Vec<String>,
    pub dependency_blocked: bool,
    pub unresolved_dependencies: Vec<String>,
    pub allowed_transitions: Vec<String>,
    pub claimable: bool,
}
#[derive(Debug, Serialize)]
pub struct RelationView {
    pub source: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub target: String,
    pub direction: String,
}

pub fn default_project(name: String, prefix: String) -> ProjectConfig {
    let mut states = BTreeMap::new();
    for (n, l, t, d) in [
        ("backlog", "Backlog", false, false),
        ("ready", "Ready", false, false),
        ("in_progress", "In Progress", false, false),
        ("blocked", "Blocked", false, false),
        ("done", "Done", true, true),
        ("cancelled", "Cancelled", true, false),
    ] {
        states.insert(
            n.into(),
            StateConfig {
                label: l.into(),
                terminal: t,
                dependency_satisfied: d,
            },
        );
    }
    let mut transitions = BTreeMap::new();
    let strings = |v: &[&str]| v.iter().map(|s| s.to_string()).collect();
    transitions.insert(
        "backlog".into(),
        strings(&["ready", "in_progress", "blocked", "cancelled"]),
    );
    transitions.insert(
        "ready".into(),
        strings(&["backlog", "in_progress", "blocked", "cancelled"]),
    );
    transitions.insert(
        "in_progress".into(),
        strings(&["backlog", "ready", "blocked", "done", "cancelled"]),
    );
    transitions.insert(
        "blocked".into(),
        strings(&["backlog", "ready", "in_progress", "cancelled"]),
    );
    transitions.insert("done".into(), strings(&["in_progress"]));
    transitions.insert("cancelled".into(), strings(&["backlog"]));
    let mut relations = BTreeMap::new();
    relations.insert(
        "subtask_of".into(),
        RelationType {
            inverse_label: "parent_of".into(),
            symmetric: false,
            acyclic: true,
        },
    );
    relations.insert(
        "relates_to".into(),
        RelationType {
            inverse_label: "relates_to".into(),
            symmetric: true,
            acyclic: false,
        },
    );
    ProjectConfig {
        schema_version: SCHEMA_VERSION,
        name,
        prefix,
        workflow: Workflow {
            initial_state: "backlog".into(),
            claim_state: "in_progress".into(),
            states,
            transitions,
        },
        relations,
    }
}
pub fn task_id_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let p = |id: &str| id.rsplit_once('-').and_then(|(_, n)| n.parse::<u64>().ok());
    p(a).cmp(&p(b)).then_with(|| a.cmp(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workflow_defaults() {
        let p = default_project("Demo".into(), "DEM".into());
        assert_eq!(p.workflow.initial_state, "backlog");
        assert!(p.workflow.states["done"].dependency_satisfied);
    }
    #[test]
    fn deterministic_task() {
        let mut t = Task {
            schema_version: 1,
            id: "DEM-3".into(),
            header: "x".into(),
            description: "".into(),
            acceptance_criteria: vec![],
            next_acceptance_number: 1,
            context: vec![],
            state: "backlog".into(),
            assignee: None,
            tags: vec!["Z".into(), "a".into(), "A".into()],
            dependencies: vec!["DEM-10".into(), "DEM-2".into()],
            relations: vec![],
            created_at: "x".into(),
            created_by: "u".into(),
            updated_at: "x".into(),
            updated_by: "u".into(),
            revision: 1,
        };
        t.normalize();
        assert_eq!(t.tags, vec!["a", "z"]);
        assert_eq!(t.dependencies, vec!["DEM-2", "DEM-10"]);
        let _: Task = serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
    }
    #[test]
    fn yaml_roundtrip() {
        let p = default_project("Demo".into(), "DEM".into());
        let d: ProjectConfig =
            serde_yaml_ng::from_str(&serde_yaml_ng::to_string(&p).unwrap()).unwrap();
        assert_eq!(d.prefix, "DEM");
    }
}
