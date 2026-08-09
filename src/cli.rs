use crate::model::{ContextKind, UserKind};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::str::FromStr;

pub const LONG_ABOUT: &str = r#"Tasker is a local-first, daemonless task manager for coding agents and humans.

Each project is a self-contained filesystem directory containing tasker.yaml,
tasks, project-local users, and immutable changelog events. No server, login,
database, network, or background process is required.

AGENT QUICKSTART

  # Discover projects
  tasker list projects -o json

  # Find and atomically claim available work
  tasker next -p PEV -o json
  tasker claim next -p PEV --as pi-backend -o json
  tasker claim PEV-12 --as pi-backend -o json

  # Inspect, create, update, and search tasks
  tasker list tasks -p PEV -o json
  tasker get task PEV-12 -o json
  tasker create task "Implement search" -p PEV --tag cli -o json
  tasker update task PEV-12 --description "New details"
  tasker search tasks authentication -p PEV -o json

  # Define and complete measurable acceptance criteria
  tasker acceptance add PEV-12 "All tests pass"
  tasker acceptance check PEV-12 AC-1 --actor pi-backend

  # Report progress, context, and decisions back to the task
  tasker context add PEV-12 "Selected an append-only event design" --kind decision

  # Users, dependencies, relations, workflow, and audit history
  tasker create user "Pi Backend" -p PEV --kind agent
  tasker dependency add PEV-12 PEV-7
  tasker relation add PEV-12 subtask_of PEV-2
  tasker transition PEV-12 done --actor pi-backend
  tasker changelog --task PEV-12 -o json

CORE FLOW

  1. Discover: tasker list projects -o json
  2. Find work: tasker next -p PEV -o json
  3. Claim atomically: tasker claim next -p PEV --as pi-agent -o json
  4. Read contract: tasker get task PEV-12 -o json
  5. Inspect blockers: tasker status PEV-12 -o json
  6. Report work: tasker context add PEV-12 "Implemented parser" --kind progress
  7. Verify checkpoints: tasker acceptance list PEV-12
  8. Check verified criteria: tasker acceptance check PEV-12 AC-1 --actor pi-agent
  9. Complete: tasker transition PEV-12 done --actor pi-agent

PROJECT RESOLUTION AND CONFIGURATION

  Task commands infer a project from --project, then the task-ID prefix,
  TASKER_PROJECT, or the nearest ancestor containing tasker.yaml. Project
  selectors accept a prefix, folder name, exact project name, or path.
  Inspect workflow states, transitions, and relation types with:
    tasker get project PEV -o json --pretty
  Edit the returned config_file (tasker.yaml) to customize them, then run:
    tasker validate -p PEV -o json

MACHINE USE

  Use -o json for compact JSON or -o yaml for YAML. Add --pretty for readable
  JSON. After parsing, output defaults through TASKER_OUTPUT, global config,
  then human. Parse errors use explicit --output/TASKER_OUTPUT, otherwise human.
  Machine errors are written to stderr as {"error":{"code","message",...}}.
  Exit categories: 2 input, 3 not-found/no-work, 4 conflict, 5 validation or
  workflow, 6 graph cycle, 7 project/config, 8 filesystem I/O.

ACTORS AND CONCURRENCY

  Mutations run under the exclusive project lock, reload affected files, and
  atomically replace mutable files. Mutations accept --actor (or TASKER_ACTOR);
  claim --as is also the actor fallback. Use --if-revision N to reject stale
  writes. Resolving an unknown actor/claimant can create a user.created event
  before a later domain check fails; task state itself is never partly claimed.

LEARN MORE

  Run tasker <command> --help and tasker <command> <subcommand> --help.
  Useful starting points: tasker claim --help, tasker acceptance --help,
  tasker context --help, tasker dependency --help, and tasker search --help.
"#;

#[derive(Debug, Parser)]
#[command(name = "tasker", version, about = "Local-first task management for coding agents", long_about = LONG_ABOUT)]
pub struct Cli {
    /// Project selector for commands that need one: prefix, folder, exact name, or path
    #[arg(short, long, global = true)]
    pub project: Option<String>,
    /// Output format; JSON/YAML errors use the same format on stderr after parsing
    #[arg(short, long, global = true, value_enum)]
    pub output: Option<OutputFormat>,
    /// Actor used for attribution; falls back to TASKER_ACTOR, --as, then OS username
    #[arg(long, global = true)]
    pub actor: Option<String>,
    /// Pretty-print JSON machine output (YAML and human output are already readable)
    #[arg(long, global = true)]
    pub pretty: bool,
    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    pub fn effective_output(&self) -> OutputFormat {
        self.output
            .or_else(|| {
                std::env::var("TASKER_OUTPUT")
                    .ok()
                    .and_then(|v| v.parse().ok())
            })
            .or_else(crate::storage::configured_output)
            .unwrap_or(OutputFormat::Human)
    }
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
#[clap(rename_all = "lower")]
pub enum OutputFormat {
    Human,
    Json,
    Yaml,
}

impl FromStr for OutputFormat {
    type Err = ();
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "human" => Ok(Self::Human),
            "json" => Ok(Self::Json),
            "yaml" => Ok(Self::Yaml),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Read or update global configuration
    #[command(
        long_about = "Read or update global configuration.\n\nThe project root resolves from TASKER_ROOT, then global config, then ~/tasker.\nThe default output resolves from --output, TASKER_OUTPUT, global config, then human.\n\nExamples:\n  tasker config show\n  tasker config get projects-root\n  tasker config set projects-root ~/Development/projects\n  tasker config set default-output json"
    )]
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Create a project, task, or user
    #[command(
        long_about = "Create project-local resources.\n\nProjects are self-contained directories. Tasks receive sequential prefixed IDs.\nUsers provide assignment and actor attribution, not authentication.\n\nExamples:\n  tasker create project \"Example App\" --prefix APP\n  tasker create task \"Implement parser\" -p APP --description \"Parse input\" --tag cli\n  tasker create task -p APP -i json < task.json\n  tasker create user \"Pi Backend\" -p APP --kind agent"
    )]
    Create {
        #[command(subcommand)]
        command: CreateCommand,
    },
    /// Idempotently ensure a project-local user exists
    #[command(
        long_about = "Idempotently ensure a project-local user exists.\n\nUse ensure when an agent may run setup repeatedly. Existing users are returned\nwithout modification (including kind); missing users use the requested kind.\n\nExample:\n  tasker ensure user \"Pi Backend\" -p APP --kind agent -o json"
    )]
    Ensure {
        #[command(subcommand)]
        command: EnsureCommand,
    },
    /// List projects, tasks, or users
    #[command(
        long_about = "List resources deterministically.\n\nProject listing is cheap and does not scan tasks unless --stats is used.\nTask listing supports workflow, tag, assignment, readiness, and pagination filters.\n\nExamples:\n  tasker list projects -o json\n  tasker list tasks -p APP --ready --unassigned -o json\n  tasker list tasks -p APP --state backlog --tag backend --full -o json\n  tasker list users -p APP"
    )]
    List {
        #[command(subcommand)]
        command: ListCommand,
    },
    /// Get a project, task, or user
    #[command(
        long_about = "Get one complete resource.\n\nTask IDs infer their project from the prefix when --project is omitted. User\nselectors accept a stable ID or case-insensitive exact name. Project output\nexposes workflow states/transitions, relation types, and editable config_file.\n\nExamples:\n  tasker get project APP -o json --pretty\n  tasker get task APP-12 -o json --pretty\n  tasker get user \"Pi Backend\" -p APP -o json"
    )]
    Get {
        #[command(subcommand)]
        command: GetCommand,
    },
    /// Update mutable task content or a user name
    #[command(
        long_about = "Update mutable resource fields.\n\nTask update can change only header, description, and tags. Use acceptance,\ncontext, assign, transition, dependency, and relation for protected fields.\nJSON/YAML task input is a patch, not replacement.\n\nExamples:\n  tasker update task APP-12 --description-file work.md --if-revision 4\n  echo '{\"tags\":[\"backend\"]}' | tasker update task APP-12 -i json\n  tasker update user pi-backend -p APP --name \"Pi Backend Agent\""
    )]
    Update {
        #[command(subcommand)]
        command: UpdateCommand,
    },
    /// Show derived workflow, acceptance, dependency, and claim status
    #[command(
        long_about = "Show information an agent needs before acting: current state and assignee, acceptance completion, unresolved dependencies, allowed transitions, and claimability. claimable means the task is currently unassigned, non-terminal, dependency-ready, and can transition to workflow.claim_state; it does not evaluate a prospective --as user.\n\nExample:\n  tasker status APP-12 -o json"
    )]
    Status(TaskIdArgs),
    /// Assign a task, automatically creating an unknown user
    #[command(
        long_about = "Assign a task without changing its workflow state. The user selector accepts ID or name; an unknown name is created automatically.\n\nExamples:\n  tasker assign APP-12 \"Pi Backend\" --actor coordinator\n  tasker assign APP-12 pi-backend --if-revision 3 -o json"
    )]
    Assign(AssignArgs),
    /// Remove a task assignment
    #[command(
        long_about = "Remove the assignee without changing workflow state.\n\nExample:\n  tasker unassign APP-12 --actor coordinator --if-revision 4"
    )]
    Unassign(MutateTaskArgs),
    /// Transition a task according to its configured workflow
    #[command(
        long_about = "Change task state using transitions configured in tasker.yaml. Invalid transitions are rejected. Entering a dependency-satisfying state (default: done) requires all acceptance criteria to be checked. Unresolved dependencies do not prohibit manual transitions; inspect status/dependency check before completing work. Discover allowed transitions with status and all configured states with get project.\n\nExamples:\n  tasker status APP-12 -o json\n  tasker transition APP-12 in_progress --actor pi-agent\n  tasker acceptance list APP-12\n  tasker transition APP-12 done --actor pi-agent --if-revision 8"
    )]
    Transition(TransitionArgs),
    /// Atomically claim a task, or use `claim next` to claim available work
    #[command(
        long_about = "Run under the exclusive project lock: create/resolve the claimant, verify ownership and dependencies, assign the task, and transition it to workflow.claim_state. Use target 'next' to choose the lowest-ID non-terminal actionable task that can transition to claim_state under the same lock. Tasks with unresolved dependencies cannot be claimed. A known terminal task follows configured transitions and may reopen if permitted. Unknown claimant/actor users may be created before a later eligibility check fails; task state itself is never partly claimed.\n\nExamples:\n  tasker claim APP-12 --as pi-backend -o json\n  tasker claim next -p APP --as pi-backend -o json"
    )]
    Claim(ClaimArgs),
    /// List actionable work without claiming it
    #[command(
        long_about = "List non-terminal, dependency-ready work to start or continue. Without --as, only unassigned tasks are returned. With --as, the user must already exist and tasks unassigned or assigned to that user are returned. This is non-mutating but not an exact preview of claim next: claim selection additionally requires a configured transition to workflow.claim_state.\n\nExamples:\n  tasker next -p APP -o json\n  tasker ensure user \"Pi Backend\" -p APP --kind agent\n  tasker next -p APP --as pi-backend --limit 10 -o json"
    )]
    Next(NextArgs),
    /// Manage first-class task dependencies
    #[command(
        long_about = "Manage first-class blocking dependencies.\n\n'A depends on B' is stored only on A. A is dependency-blocked until B reaches\na workflow state with dependency_satisfied: true. Both IDs must belong to the\nsame project. Cross-project dependencies and cycles are rejected.\n\nExamples:\n  tasker dependency add APP-12 APP-4\n  tasker dependency check APP-12 -o json\n  tasker dependency list APP-12 --recursive\n  tasker dependency list APP-4 --reverse"
    )]
    Dependency {
        #[command(subcommand)]
        command: DependencyCommand,
    },
    /// Manage measurable task acceptance criteria
    #[command(
        long_about = "Manage measurable completion checkpoints stored separately from description.\n\nCriteria receive monotonic AC-N IDs that are never reused. Checking records\nactor and timestamp. A task with criteria cannot enter a dependency-satisfying\nstate until all are checked.\n\nTypical flow:\n  tasker acceptance list APP-12\n  tasker acceptance add APP-12 \"All parser tests pass\"\n  tasker acceptance check APP-12 AC-1 --actor pi-agent\n  tasker status APP-12 -o json\n  tasker transition APP-12 done --actor pi-agent"
    )]
    Acceptance {
        #[command(subcommand)]
        command: AcceptanceCommand,
    },
    /// Append agent-readable context, progress, and decisions to tasks
    #[command(
        long_about = "Append a curated development narrative to a task.\n\nEntries are append-only and receive CTX-N IDs, actor, and timestamp. Kinds are\ncontext (background), progress (work performed), and decision (choice/rationale).\nThe project changelog remains the lower-level machine audit trail.\n\nExamples:\n  tasker context add APP-12 \"Implemented parser\" --kind progress --actor pi-agent\n  tasker context add APP-12 \"Use recursive descent\" --kind decision\n  tasker context list APP-12 --kind decision -o json"
    )]
    Context {
        #[command(subcommand)]
        command: ContextCommand,
    },
    /// Manage configured generic task relationships
    #[command(
        long_about = "Manage non-blocking relationships configured in tasker.yaml.\n\nDefaults are subtask_of (inverse parent_of, acyclic) and relates_to (symmetric).\nOnly the outgoing relation is stored; incoming/inverse views are query-relative\nderived rows. Remove uses the original stored source, configured type, and target.\n\nExamples:\n  tasker relation add APP-12 subtask_of APP-3\n  tasker relation list APP-3 --direction incoming\n  tasker relation remove APP-12 subtask_of APP-3"
    )]
    Relation {
        #[command(subcommand)]
        command: RelationCommand,
    },
    /// Search tasks or changelog events
    #[command(
        long_about = "Search current tasks or immutable audit events without an index.\n\nTask text search uses case-insensitive AND terms across ID, header, description,\nacceptance criteria, context, tags, and assignee. Filters can be combined.\n\nExamples:\n  tasker search tasks \"oauth token\" -p APP --state backlog -o json\n  tasker search tasks -p APP --ready --unassigned --tag backend\n  tasker search changelog \"APP-12 task.transitioned\" -p APP"
    )]
    Search {
        #[command(subcommand)]
        command: SearchCommand,
    },
    /// Query newest-first immutable project audit history
    #[command(
        long_about = "Query immutable project changelog events, newest first. Filter by task, actor, or case-sensitive exact action. Discover action names from an unfiltered query; common values include project.created, user.created, task.created, task.updated, task.claimed, task.transitioned, task.acceptance_completed, and task.context_added. For curated task-level progress and decisions use 'tasker context'.\n\nExamples:\n  tasker changelog -p APP --limit 20\n  tasker changelog --task APP-12 --action task.transitioned -o json\n  tasker changelog -p APP --actor pi-backend -o yaml"
    )]
    Changelog(ChangelogArgs),
    /// Validate all project files and graph invariants
    #[command(
        long_about = "Validate manually editable project files and selected invariants: typed schemas; task/user filenames and IDs; workflow states; acceptance/context structure; assignees; dependency/relation targets and cycles; task-number metadata; and changelog schema versions. Changelog references and non-assignee attribution fields are not cross-validated. Success returns valid=true; validation problems use exit code 5.\n\nExamples:\n  tasker validate -p APP\n  tasker validate -p APP -o json"
    )]
    Validate,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Show effective projects root, default output, and config file path
    #[command(
        long_about = "Show effective configuration after environment overrides.\n\nExample:\n  tasker config show -o json"
    )]
    Show,
    /// Get projects-root, default-output, or config-file
    #[command(
        long_about = "Get one effective setting. Readable keys: projects-root, default-output, and config-file. Hyphenated or underscored spelling is accepted.\n\nExamples:\n  tasker config get projects-root\n  tasker config get default-output\n  tasker config get config-file"
    )]
    Get {
        /// Readable key: projects-root, default-output, or config-file
        key: String,
    },
    /// Persist projects-root or default-output in the OS config directory
    #[command(
        long_about = "Persist projects-root or default-output. Hyphenated/underscored keys are accepted. TASKER_ROOT and TASKER_OUTPUT still take precedence. Paths beginning with ~ are expanded.\n\nExamples:\n  tasker config set projects-root ~/Development/projects\n  tasker config set default-output json"
    )]
    Set {
        /// Writable key: projects-root or default-output
        key: String,
        /// New path or output format value
        value: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum CreateCommand {
    /// Create a self-contained project directory
    #[command(
        long_about = "Create one self-contained project with tasker.yaml, metadata, lock, tasks, users, and changelog directories. PREFIX must match ^[A-Z][A-Z0-9]{1,9}$ and be unique under projects_root. Relative --path values resolve from the current directory. A project outside a direct child of projects_root is not found by list/prefix/name discovery; select it later by path or run inside it.\n\nExamples:\n  tasker create project \"Example App\" --prefix APP\n  tasker create project \"Example App\" --prefix APP --path ../projects/example-app"
    )]
    Project(CreateProjectArgs),
    /// Create a task using flags or JSON/YAML input
    #[command(
        long_about = "Create a task in workflow.initial_state and allocate the next prefixed ID under the project lock. Description says what to implement; acceptance criteria are measurable checkpoints. JSON/YAML accepts header (required), description, tags, and acceptance_criteria (string array). Direct flags override corresponding structured values; unknown fields are rejected. --file is used only with -i json|yaml.\n\nExamples:\n  tasker create task \"Implement parser\" -p APP --description \"Parse config files\" --tag cli\n  tasker create task \"Implement parser\" -p APP --acceptance-criterion \"Tests pass\"\n  tasker create task -p APP -i json --file task.json -o json\n  tasker create task -p APP -i yaml < task.yaml -o yaml"
    )]
    Task(CreateTaskArgs),
    /// Create a project-local user
    #[command(
        long_about = "Create a project-local attribution identity. Names are case-insensitively unique; Tasker generates a stable slug ID. Users provide no permissions.\n\nExample:\n  tasker create user \"Pi Backend\" -p APP --kind agent -o json"
    )]
    User(CreateUserArgs),
}

#[derive(Debug, Args)]
#[command(
    after_help = "PREFIX: 2-10 uppercase ASCII letters/digits, beginning with a letter.\nRelative --path values resolve from the current directory. Projects outside a direct child of projects_root must later be selected by path or from inside the project.\n\nExamples:\n  tasker create project \"Example App\" --prefix APP\n  tasker create project \"Example App\" --prefix APP --path ../projects/example-app"
)]
pub struct CreateProjectArgs {
    /// Human-readable project name
    pub name: String,
    /// Unique 2-10 character uppercase prefix matching ^[A-Z][A-Z0-9]{1,9}$
    #[arg(long)]
    pub prefix: String,
    /// Directory path; defaults to a direct-child name slug under projects_root
    #[arg(long)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
#[clap(rename_all = "lower")]
pub enum InputFormat {
    Human,
    Json,
    Yaml,
}

#[derive(Debug, Args)]
pub struct InputArgs {
    /// Input format; json/yaml reads stdin or --file, while human uses normal flags
    #[arg(short = 'i', long, value_enum, default_value = "human")]
    pub input: InputFormat,
    /// Read JSON/YAML input from this file; ignored when --input human
    #[arg(long)]
    pub file: Option<String>,
}

#[derive(Debug, Args)]
#[command(
    after_help = "Examples:\n  tasker create task \"Implement parser\" -p APP --description \"Parse configuration\" --tag cli\n  tasker create task \"Implement parser\" -p APP --acceptance-criterion \"All parser tests pass\"\n  echo '{\"header\":\"Implement parser\",\"tags\":[\"cli\"],\"acceptance_criteria\":[\"Tests pass\"]}' | tasker create task -p APP -i json -o json"
)]
pub struct CreateTaskArgs {
    /// Short task title (omit when supplied by JSON/YAML input)
    pub header: Option<String>,
    /// What must be implemented; keep measurable checkpoints in acceptance criteria
    #[arg(long)]
    pub description: Option<String>,
    /// Read the description from a UTF-8 file
    #[arg(long, conflicts_with = "description")]
    pub description_file: Option<String>,
    /// Tag; repeat to add multiple tags
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    /// Measurable acceptance criterion; repeat to add multiple criteria
    #[arg(long = "acceptance-criterion")]
    pub acceptance_criteria: Vec<String>,
    #[command(flatten)]
    pub input_args: InputArgs,
}

#[derive(Debug, Args)]
#[command(
    after_help = "Examples:\n  tasker create user \"Jonas\" -p APP --kind human\n  tasker create user \"Pi Backend\" -p APP --kind agent -o json"
)]
pub struct CreateUserArgs {
    /// Unique case-insensitive display name; a stable slug ID is generated
    pub name: String,
    /// Attribution category only; it grants no permissions
    #[arg(long, value_enum, default_value = "unknown")]
    pub kind: UserKind,
}

#[derive(Debug, Subcommand)]
pub enum EnsureCommand {
    /// Return an existing user by case-insensitive name, or create one
    #[command(
        long_about = "Idempotently resolve an exact case-insensitive name or create it. An existing user's name and kind are returned unchanged; --kind applies only when creating. Useful in repeatable agent setup.\n\nExample:\n  tasker ensure user \"Pi Backend\" -p APP --kind agent -o json"
    )]
    User(CreateUserArgs),
}

#[derive(Debug, Subcommand)]
pub enum ListCommand {
    /// Discover direct child projects without scanning task files
    #[command(
        long_about = "Discover direct children of projects_root having a valid readable tasker.yaml. Malformed project configs are omitted; use validate with a path/name recoverable from YAML to diagnose them. By default no task files are read. --stats scans tasks and reports only states represented by at least one task.\n\nExamples:\n  tasker list projects\n  tasker list projects --stats -o json"
    )]
    Projects {
        /// Include task counts by state (requires scanning tasks)
        #[arg(long)]
        stats: bool,
    },
    /// List compact task summaries with optional filters
    #[command(
        long_about = "List tasks in numeric ID order. Combine state, tag, assignment, readiness, and pagination filters. Use --full for complete task objects.\n\nExamples:\n  tasker list tasks -p APP\n  tasker list tasks -p APP --ready --unassigned -o json\n  tasker list tasks -p APP --state backlog --tag backend --full"
    )]
    Tasks(TaskFilterArgs),
    /// List project-local users
    #[command(
        long_about = "List project-local attribution identities by stable ID.\n\nExample:\n  tasker list users -p APP -o json"
    )]
    Users,
}

#[derive(Debug, Args, Default)]
pub struct TaskFilterArgs {
    /// Match state; repeat for OR semantics
    #[arg(long = "state")]
    pub states: Vec<String>,
    /// Require tag; repeat for AND semantics
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    /// Match assignee by stable ID or case-insensitive name
    #[arg(long)]
    pub assignee: Option<String>,
    /// Return only tasks without an assignee
    #[arg(long)]
    pub unassigned: bool,
    /// Return only non-terminal tasks with all dependencies satisfied
    #[arg(long)]
    pub ready: bool,
    /// Return only tasks having at least one unresolved dependency
    #[arg(long)]
    pub dependency_blocked: bool,
    /// Maximum number of results after filtering
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
    /// Number of filtered results to skip
    #[arg(long, default_value_t = 0)]
    pub offset: usize,
    /// Return complete task objects
    #[arg(long)]
    pub full: bool,
}

#[derive(Debug, Subcommand)]
pub enum GetCommand {
    /// Get project identity, workflow, relation types, and editable config path
    #[command(
        long_about = "Get a project selected by prefix, folder, exact name, or path. Output includes workflow states, transitions, relation definitions, and config_file. To customize, edit tasker.yaml and run validate.\n\nExamples:\n  tasker get project APP -o json --pretty\n  tasker validate -p APP -o json"
    )]
    Project {
        /// Prefix, folder, exact project name, or filesystem path
        project: String,
    },
    /// Get the complete authoritative task object
    #[command(
        long_about = "Get one complete task including description, acceptance criteria, context, state, assignment, dependencies, relations, timestamps, and revision. The ID prefix infers the project.\n\nExample:\n  tasker get task APP-12 -o json --pretty"
    )]
    Task {
        /// Prefixed task ID, for example APP-12
        task: String,
    },
    /// Get a project-local user by ID or exact case-insensitive name
    #[command(
        long_about = "Get a project-local attribution identity.\n\nExamples:\n  tasker get user pi-backend -p APP -o json\n  tasker get user \"Pi Backend\" -p APP"
    )]
    User {
        /// Stable user ID or exact display name
        user: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum UpdateCommand {
    /// Patch only header, description, and tags
    #[command(
        long_about = "Patch mutable task content only. This cannot change state, assignment, criteria, context, dependencies, relations, IDs, timestamps, or revision directly. JSON/YAML is a patch accepting only header, description, and tags; unknown/protected fields are rejected. Direct flags override structured values. --file is used only with -i json|yaml. With both --clear-tags and --tag, supplied tags win.\n\nExamples:\n  tasker update task APP-12 --description-file work.md --if-revision 4\n  echo '{\"header\":\"New header\",\"tags\":[\"backend\"]}' | tasker update task APP-12 -i json\n  tasker update task APP-12 -i yaml --file patch.yaml -o yaml"
    )]
    Task(UpdateTaskArgs),
    /// Rename a user while retaining its stable ID
    #[command(
        long_about = "Change a user's unique display name while keeping its stable ID for task and changelog references.\n\nExample:\n  tasker update user pi-backend -p APP --name \"Pi Backend Agent\""
    )]
    User(UpdateUserArgs),
}

#[derive(Debug, Args)]
#[command(
    after_help = "Examples:\n  tasker update task APP-12 --header \"New header\" --if-revision 3\n  tasker update task APP-12 --description-file implementation.md\n  echo '{\"tags\":[\"backend\",\"auth\"]}' | tasker update task APP-12 -i json"
)]
pub struct UpdateTaskArgs {
    /// Prefixed task ID; its prefix can infer the project
    pub task: String,
    /// Replace the short task title
    #[arg(long)]
    pub header: Option<String>,
    /// Replace what must be implemented
    #[arg(long)]
    pub description: Option<String>,
    /// Read replacement description from a UTF-8 file
    #[arg(long, conflicts_with = "description")]
    pub description_file: Option<String>,
    /// Replace tags with these values; repeat for multiple tags
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    /// Clear tags when no --tag values are supplied; supplied --tag values win
    #[arg(long)]
    pub clear_tags: bool,
    /// Fail with revision_conflict unless the task currently has this revision
    #[arg(long)]
    pub if_revision: Option<u64>,
    #[command(flatten)]
    pub input_args: InputArgs,
}

#[derive(Debug, Args)]
#[command(
    after_help = "Example:\n  tasker update user pi-backend -p APP --name \"Pi Backend Agent\""
)]
pub struct UpdateUserArgs {
    /// Stable user ID or exact case-insensitive display name
    pub user: String,
    /// New unique display name; the stable user ID does not change
    #[arg(long)]
    pub name: String,
}

#[derive(Debug, Args)]
pub struct TaskIdArgs {
    /// Prefixed task ID, for example APP-12
    pub task: String,
}

#[derive(Debug, Args)]
pub struct MutateTaskArgs {
    /// Prefixed task ID, for example APP-12
    pub task: String,
    /// Reject a stale mutation unless the current revision matches
    #[arg(long)]
    pub if_revision: Option<u64>,
}

#[derive(Debug, Args)]
pub struct AssignArgs {
    /// Prefixed task ID, for example APP-12
    pub task: String,
    /// Stable user ID or name; an unknown name is created automatically
    pub user: String,
    /// Reject a stale mutation unless the current revision matches
    #[arg(long)]
    pub if_revision: Option<u64>,
}

#[derive(Debug, Args)]
pub struct TransitionArgs {
    /// Prefixed task ID, for example APP-12
    pub task: String,
    /// Target state configured in the project's tasker.yaml workflow
    pub state: String,
    /// Reject a stale mutation unless the current revision matches
    #[arg(long)]
    pub if_revision: Option<u64>,
}

#[derive(Debug, Args)]
pub struct ClaimArgs {
    /// Task ID, or the literal `next`
    pub target: String,
    /// User claiming the work (created automatically if missing)
    #[arg(long = "as")]
    pub as_user: String,
    /// Reject a stale known-task/selected-task claim unless its revision matches
    #[arg(long)]
    pub if_revision: Option<u64>,
}

#[derive(Debug, Args)]
pub struct NextArgs {
    /// Restrict work to unassigned tasks or tasks assigned to this user
    #[arg(long = "as")]
    pub as_user: Option<String>,
    /// Maximum number of actionable tasks to return
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
}

#[derive(Debug, Subcommand)]
pub enum DependencyCommand {
    /// Add a dependency: the first task depends on the second task
    #[command(
        long_about = "Add a dependency. The first task depends on the second task.\n\nExample:\n  tasker dependency add APP-12 APP-4\n\nThis means APP-12 is not dependency-ready until APP-4 reaches a dependency-satisfying state."
    )]
    Add(PairTaskArgs),
    /// Remove a direct dependency
    #[command(
        long_about = "Remove a direct dependency from the first task.\n\nExample:\n  tasker dependency remove APP-12 APP-4\n\nAfter removal, readiness is recalculated from remaining dependencies."
    )]
    Remove(PairTaskArgs),
    /// List direct, transitive, or reverse dependencies
    #[command(
        long_about = "List task IDs in deterministic numeric order. Default lists direct dependencies; --recursive walks transitive dependencies; --reverse finds tasks that depend on this task.\n\nExamples:\n  tasker dependency list APP-12\n  tasker dependency list APP-12 --recursive\n  tasker dependency list APP-4 --reverse"
    )]
    List(DependencyListArgs),
    /// Report unresolved dependencies and dependency-blocked status
    #[command(
        long_about = "Check whether every dependency is in a state configured with dependency_satisfied: true.\n\nExample:\n  tasker dependency check APP-12 -o json"
    )]
    Check(TaskIdArgs),
}

#[derive(Debug, Args)]
pub struct PairTaskArgs {
    /// Dependent task: this task waits for DEPENDENCY
    pub task: String,
    /// Prerequisite task that must reach a dependency-satisfying state
    pub dependency: String,
    /// Reject a stale mutation unless TASK's current revision matches
    #[arg(long)]
    pub if_revision: Option<u64>,
}

#[derive(Debug, Args)]
pub struct DependencyListArgs {
    /// Task whose dependency graph should be queried
    pub task: String,
    /// Include all transitively required tasks
    #[arg(long, conflicts_with = "reverse")]
    pub recursive: bool,
    /// Instead list tasks that directly depend on TASK
    #[arg(long, conflicts_with = "recursive")]
    pub reverse: bool,
}

#[derive(Debug, Subcommand)]
pub enum AcceptanceCommand {
    /// Add a measurable completion checkpoint and return its stable AC-N ID
    #[command(
        long_about = "Append a measurable checkpoint. Tasker assigns the next monotonic AC-N ID, never reuses removed IDs, and rejects duplicate text.\n\nExample:\n  tasker acceptance add APP-12 \"All parser tests pass\" --actor product-owner"
    )]
    Add(AcceptanceAddArgs),
    /// Remove an acceptance criterion by AC-N ID
    #[command(
        long_about = "Remove a criterion by stable ID. Prefer uncheck when the checkpoint remains part of the contract.\n\nExample:\n  tasker acceptance remove APP-12 AC-3 --if-revision 8"
    )]
    Remove(AcceptanceIdArgs),
    /// Mark an acceptance criterion as completed by the current actor
    #[command(
        long_about = "Mark a verified criterion complete and record actor and timestamp. This is idempotent.\n\nExample:\n  tasker acceptance check APP-12 AC-1 --actor pi-agent"
    )]
    Check(AcceptanceIdArgs),
    /// Reopen a completed acceptance criterion
    #[command(
        long_about = "Mark a criterion incomplete again and clear completion attribution.\n\nExample:\n  tasker acceptance uncheck APP-12 AC-1 --actor reviewer"
    )]
    Uncheck(AcceptanceIdArgs),
    /// List all acceptance criteria and completion state
    #[command(
        long_about = "List stable IDs, text, completion state, and completion attribution.\n\nExamples:\n  tasker acceptance list APP-12\n  tasker acceptance list APP-12 -o json"
    )]
    List(TaskIdArgs),
}

#[derive(Debug, Args)]
pub struct AcceptanceAddArgs {
    /// Task receiving the checkpoint
    pub task: String,
    /// Specific measurable condition that can be verified true or false
    pub criterion: String,
    /// Reject a stale mutation unless the task revision matches
    #[arg(long)]
    pub if_revision: Option<u64>,
}

#[derive(Debug, Args)]
pub struct AcceptanceIdArgs {
    /// Task containing the criterion
    pub task: String,
    /// Stable criterion ID returned by acceptance add/list, for example AC-1
    #[arg(value_name = "ACCEPTANCE_ID")]
    pub acceptance_id: String,
    /// Reject a stale mutation unless the task revision matches
    #[arg(long)]
    pub if_revision: Option<u64>,
}

#[derive(Debug, Subcommand)]
pub enum ContextCommand {
    /// Append immutable context, progress, or a decision to a task
    #[command(
        long_about = "Append an agent-readable report with stable CTX-N ID, actor, and timestamp. Entries cannot be edited or removed.\n\nExamples:\n  tasker context add APP-12 \"Implemented parser\" --kind progress --actor pi-agent\n  tasker context add APP-12 \"Use recursive descent\" --kind decision"
    )]
    Add(ContextAddArgs),
    /// List task context entries in chronological order
    #[command(
        long_about = "List the curated task narrative in chronological order, optionally filtered by kind.\n\nExamples:\n  tasker context list APP-12\n  tasker context list APP-12 --kind decision -o json"
    )]
    List(ContextListArgs),
}

#[derive(Debug, Args)]
pub struct ContextAddArgs {
    /// Task receiving the report
    pub task: String,
    /// Concise useful background, work performed, or decision and rationale
    pub message: String,
    /// Entry purpose: context, progress, or decision
    #[arg(long, value_enum, default_value = "context")]
    pub kind: ContextKind,
    /// Reject a stale append unless the task revision matches
    #[arg(long)]
    pub if_revision: Option<u64>,
}

#[derive(Debug, Args)]
pub struct ContextListArgs {
    /// Task whose curated narrative should be listed
    pub task: String,
    /// Restrict results to context, progress, or decision
    #[arg(long, value_enum)]
    pub kind: Option<ContextKind>,
}

#[derive(Debug, Subcommand)]
pub enum RelationCommand {
    /// Add a configured relationship from one task to another
    #[command(
        long_about = "Add a non-blocking relationship. The relation type must exist in tasker.yaml. Acyclic types reject cycles; symmetric duplicates are rejected.\n\nExample:\n  tasker relation add APP-12 subtask_of APP-3"
    )]
    Add(RelationMutationArgs),
    /// Remove a relationship
    #[command(
        long_about = "Remove the stored outgoing relationship FROM TYPE TO. Incoming list rows are query-relative derived views and may use inverse labels; they cannot be copied directly to remove. Use the original source, configured relation type, and target.\n\nExample:\n  tasker relation remove APP-12 relates_to APP-7"
    )]
    Remove(RelationMutationArgs),
    /// List stored and derived inverse relationships
    #[command(
        long_about = "List outgoing stored relations and/or query-relative incoming derived inverse relations. Incoming rows may use inverse labels; removing them requires the original stored source, configured type, and target.\n\nExamples:\n  tasker relation list APP-12 --direction outgoing\n  tasker relation list APP-3 --direction incoming\n  tasker relation list APP-12 --direction both -o json"
    )]
    List(RelationListArgs),
}

#[derive(Debug, Args)]
pub struct RelationMutationArgs {
    /// Source task that stores the outgoing relation
    pub from: String,
    /// Relation type configured in tasker.yaml, such as subtask_of or relates_to
    #[arg(value_name = "TYPE")]
    pub kind: String,
    /// Target task in the same project
    pub to: String,
    /// Reject a stale mutation unless FROM's revision matches
    #[arg(long)]
    pub if_revision: Option<u64>,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
#[clap(rename_all = "lower")]
pub enum Direction {
    Outgoing,
    Incoming,
    Both,
}

#[derive(Debug, Args)]
pub struct RelationListArgs {
    /// Task whose relationships should be listed
    pub task: String,
    /// outgoing, incoming derived inverses, or both
    #[arg(long, value_enum, default_value = "both")]
    pub direction: Direction,
}

#[derive(Debug, Subcommand)]
pub enum SearchCommand {
    /// Case-insensitive AND-term search over task content with filters
    #[command(
        long_about = "Search task ID, header, description, acceptance criteria, context, tags, assignee ID, and assignee name. Whitespace-separated terms use AND semantics. QUERY is optional; without it this behaves like filtered list tasks. Repeated --state uses OR; repeated --tag uses AND.\n\nExamples:\n  tasker search tasks \"oauth token\" -p APP -o json\n  tasker search tasks -p APP --ready --unassigned --tag backend\n  tasker search tasks websocket -p APP --state backlog --state ready --full"
    )]
    Tasks(SearchTasksArgs),
    /// Search task ID, actor, action, and changed values in audit history
    #[command(
        long_about = "Search serialized immutable changelog events using case-insensitive AND terms and return newest matches first. For curated progress and decisions use context list.\n\nExample:\n  tasker search changelog \"APP-12 transitioned done\" -p APP -o json"
    )]
    Changelog(SearchChangelogArgs),
}

#[derive(Debug, Args)]
pub struct SearchTasksArgs {
    /// Optional whitespace-separated substring terms; all terms must match
    pub query: Option<String>,
    #[command(flatten)]
    pub filters: TaskFilterArgs,
}

#[derive(Debug, Args)]
pub struct SearchChangelogArgs {
    /// Whitespace-separated terms; all must occur in the serialized event
    pub query: String,
    /// Maximum newest matching events to return
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct ChangelogArgs {
    /// Restrict audit events to one task ID
    #[arg(long)]
    pub task: Option<String>,
    /// Restrict to actor ID or case-insensitive actor name
    #[arg(long)]
    pub actor: Option<String>,
    /// Case-sensitive exact action; omit to discover names such as task.transitioned
    #[arg(long)]
    pub action: Option<String>,
    /// Maximum newest events to return
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
}
