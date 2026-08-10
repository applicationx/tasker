use crate::knowledge::model::{DecisionScope, DecisionStatus, ResourceKind, ResourceStatus};
use crate::model::{ContextKind, ContextReferenceRole, UserKind};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::str::FromStr;

pub const LONG_ABOUT: &str = r#"Tasker is a local-first, daemonless task manager for coding agents and humans.

Each project is a self-contained filesystem directory containing tasker.yaml,
PROJECT.md, tasks, resources, decisions, project-local users, and immutable
changelog events. No server, login, database, network, or background process is required.

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

  # Project knowledge, decisions, task references, and bounded agent briefs
  tasker project brief show -p PEV -o markdown
  tasker create resource "CLI design" -p PEV --kind design --file design.md
  tasker create decision "Filesystem authority" -p PEV --file decision.md
  tasker decision accept PEV-D1
  tasker context-ref add PEV-12 PEV-D1 --role constrained_by
  tasker brief PEV-12 -o markdown

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

PLANNING-TO-EXECUTION WORKFLOW

  tasker project brief show -p PEV -o markdown
  tasker create resource "Requirements" -p PEV --kind requirement --status active --file requirements.md
  tasker create decision "Keep files authoritative" -p PEV -o json
  tasker decision accept PEV-D1
  tasker create task "Implement requirements" -p PEV --from PEV-R1 --context-ref PEV-D1:constrained_by
  tasker brief PEV-1 -o markdown
  tasker claim PEV-1 --as pi-agent -o json
  tasker context add PEV-1 "Implemented the planned slice" --kind progress
  tasker acceptance list PEV-1
  tasker transition PEV-1 done --actor pi-agent

KNOWLEDGE CONCEPTS

  PROJECT.md is the editable project purpose and direction. Resources catalog
  managed Markdown or safely reference live project files. Decisions are durable
  proposed/settled choices. Task.context_refs explicitly connect tasks to
  resources and decisions. Task context is the append-only curated execution
  narrative. The changelog is the immutable machine audit trail. `tasker brief`
  assembles these authoritative sources without creating another stored source.

PROJECT RESOLUTION AND CONFIGURATION

  Task and knowledge commands infer a project from --project, then an ID prefix,
  TASKER_PROJECT, or the nearest ancestor containing tasker.yaml. Project
  selectors accept a prefix, folder name, exact project name, or path.
  Inspect workflow states, transitions, and relation types with:
    tasker get project PEV -o json --pretty
  Edit the returned config_file (tasker.yaml) to customize them, then run:
    tasker validate -p PEV -o json

MACHINE USE

  Use -o json for compact JSON, -o yaml for YAML, or -o markdown for briefs.
  Add --pretty for readable JSON. After parsing, output defaults through TASKER_OUTPUT, global config,
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
  Useful starting points: tasker claim --help, tasker project brief --help,
  tasker create resource --help, tasker decision --help, tasker context-ref --help,
  tasker brief --help, tasker dependency --help, and tasker search --help.
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
    Markdown,
    Json,
    Yaml,
}

impl FromStr for OutputFormat {
    type Err = ();
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "human" => Ok(Self::Human),
            "markdown" | "md" => Ok(Self::Markdown),
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
    /// Create a project, task, user, resource, or decision
    #[command(
        long_about = "Create project-local records.\n\nProjects are self-contained directories. Tasks, resources, and decisions receive sequential project-prefixed IDs. Users provide attribution, not authentication.\n\nExamples:\n  tasker create project \"Example App\" --prefix APP\n  tasker create task \"Implement parser\" -p APP --description \"Parse input\" --tag cli\n  tasker create resource \"CLI design\" -p APP --kind design --file design.md\n  tasker create decision \"Filesystem authority\" -p APP --file decision.md\n  tasker create user \"Pi Backend\" -p APP --kind agent"
    )]
    Create {
        #[command(subcommand)]
        command: CreateCommand,
    },
    /// Manage the root PROJECT.md project brief
    #[command(
        long_about = "Manage the authoritative root PROJECT.md project brief. New projects receive a detailed default template. Show or locate it, initialize legacy projects, or atomically replace it.\n\nExamples:\n  tasker project brief show -p APP\n  tasker project brief path -p APP -o json\n  tasker project brief init -p APP\n  tasker project brief set -p APP --file PROJECT.new.md"
    )]
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    /// Idempotently ensure a project-local user exists
    #[command(
        long_about = "Idempotently ensure a project-local user exists.\n\nUse ensure when an agent may run setup repeatedly. Existing users are returned\nwithout modification (including kind); missing users use the requested kind.\n\nExample:\n  tasker ensure user \"Pi Backend\" -p APP --kind agent -o json"
    )]
    Ensure {
        #[command(subcommand)]
        command: EnsureCommand,
    },
    /// List projects, tasks, users, resources, or decisions
    #[command(
        long_about = "List project records deterministically from current files.\n\nProject listing is cheap and does not scan tasks unless --stats is used. Task, resource, and decision listing expose entity-specific filters and pagination.\n\nExamples:\n  tasker list projects -o json\n  tasker list tasks -p APP --ready --unassigned -o json\n  tasker list resources -p APP --status active -o json\n  tasker list decisions -p APP --status accepted -o json\n  tasker list users -p APP"
    )]
    List {
        #[command(subcommand)]
        command: ListCommand,
    },
    /// Get a project, task, user, resource, or decision
    #[command(
        long_about = "Get one complete project record.\n\nTask and knowledge IDs infer their project when --project is omitted. Project output exposes workflow configuration plus editable knowledge paths and counts. Resource get reopens current referenced content; decision get returns its body.\n\nExamples:\n  tasker get project APP -o json --pretty\n  tasker get task APP-12 -o json --pretty\n  tasker get resource APP-R1 -o json\n  tasker get decision APP-D1 -o json"
    )]
    Get {
        #[command(subcommand)]
        command: GetCommand,
    },
    /// Update mutable task, user, resource, or decision fields
    #[command(
        long_about = "Update mutable record fields under the project lock.\n\nTask protected fields retain dedicated commands. Managed and referenced resource source types cannot change. Settled decisions permit tags only; lifecycle status uses tasker decision. JSON/YAML input is a patch and direct flags override it.\n\nExamples:\n  tasker update task APP-12 --description-file work.md --if-revision 4\n  tasker update resource APP-R1 --status active --if-revision 1\n  tasker update decision APP-D1 --file revised.md\n  tasker update user pi-backend -p APP --name \"Pi Backend Agent\""
    )]
    Update {
        #[command(subcommand)]
        command: UpdateCommand,
    },
    /// Archive a resource without deleting its catalog record
    #[command(
        long_about = "Archive a project resource in place. The Markdown catalog file and monotonic ID remain forever; referenced source content is never modified.\n\nExample:\n  tasker archive resource APP-R2 --if-revision 3 -o json"
    )]
    Archive {
        #[command(subcommand)]
        command: ArchiveCommand,
    },
    /// Change decision lifecycle status, including atomic supersession
    #[command(
        long_about = "Accept or reject a proposed decision, or atomically supersede one accepted decision with another accepted decision. Lifecycle commands set decision attribution and preserve both immutable IDs.\n\nExamples:\n  tasker decision accept APP-D1\n  tasker decision reject APP-D2\n  tasker decision supersede APP-D1 --with APP-D3"
    )]
    Decision {
        #[command(subcommand)]
        command: DecisionCommand,
    },
    /// Manage task links to project resources and decisions
    #[command(
        name = "context-ref",
        long_about = "Manage authoritative knowledge links stored on Task.context_refs. Targets must exist in the same project. Roles are implements, informed_by, constrained_by, and verifies.\n\nExamples:\n  tasker context-ref add APP-12 APP-R2 --role implements\n  tasker context-ref list APP-12 -o json\n  tasker context-ref reverse APP-D1 -o json"
    )]
    ContextRef {
        #[command(subcommand)]
        command: ContextRefCommand,
    },
    /// Assemble a deterministic byte-bounded project or task brief
    #[command(
        long_about = "Assemble a deterministic byte-bounded view of current project knowledge directly from authoritative files. With a task ID, include its contract, checkpoints, narrative, graph state, and explicit knowledge references. Human and markdown output render Markdown; JSON and YAML use a stable structured shape.\n\nInclusion priority is identity; task description and acceptance; linked accepted decisions; linked active resources; dependency state and relations; task context; accepted project decisions; PROJECT.md; linked historical decisions; requested archived resources; then proposed decisions and summaries. Complete lower-priority bodies are omitted before metadata, retained linked decision/resource metadata outranks mandatory text shortening, every omission is recorded, and human/Markdown output shows a truncation notice.\n\nExamples:\n  tasker brief -p APP -o markdown\n  tasker brief APP-12 --max-bytes 50000 -o json --pretty"
    )]
    Brief(BriefArgs),
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
    /// Search tasks, resources, decisions, or changelog events
    #[command(
        long_about = "Search current tasks, resources, decisions, or immutable audit events without an index.\n\nCase-insensitive whitespace terms use AND semantics. Knowledge search reopens current managed or safely referenced Markdown and includes historical statuses by default.\n\nExamples:\n  tasker search tasks \"oauth token\" -p APP --state backlog -o json\n  tasker search resources \"CLI design\" -p APP --full -o json\n  tasker search decisions filesystem -p APP --status accepted\n  tasker search changelog \"APP-12 task.transitioned\" -p APP"
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
        long_about = "Validate manually editable project files and invariants: typed schemas; task/user/resource/decision filenames and IDs; workflow and graphs; acceptance/context/context_refs; safe referenced sources; decision sections/lifecycle/supersession; allocators; transactions; and changelog schemas. Legacy missing knowledge paths produce warnings and remain valid. Validation never creates, repairs, or recovers files. Success returns valid=true; errors use exit code 5.\n\nExamples:\n  tasker validate -p APP\n  tasker validate -p APP -o json"
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
        long_about = "Create a task in workflow.initial_state and allocate the next prefixed ID under the project lock. Description says what to implement; acceptance criteria are measurable checkpoints. JSON/YAML accepts header (required), description, tags, acceptance_criteria (string array), and context_refs (typed objects). Direct flags override corresponding structured values; unknown fields are rejected. --context-ref uses ID:ROLE; --from uses informed_by. --file is used only with -i json|yaml.\n\nExamples:\n  tasker create task \"Implement parser\" -p APP --description \"Parse config files\" --tag cli\n  tasker create task \"Implement parser\" -p APP --acceptance-criterion \"Tests pass\"\n  tasker create task \"Implement plan\" -p APP --from APP-R1 --context-ref APP-D1:constrained_by\n  tasker create task -p APP -i json --file task.json -o json\n  tasker create task -p APP -i yaml < task.yaml -o yaml"
    )]
    Task(CreateTaskArgs),
    /// Create a project-local user
    #[command(
        long_about = "Create a project-local attribution identity. Names are case-insensitively unique; Tasker generates a stable slug ID. Users provide no permissions.\n\nExample:\n  tasker create user \"Pi Backend\" -p APP --kind agent -o json"
    )]
    User(CreateUserArgs),
    /// Create a managed or referenced project resource
    #[command(
        long_about = "Create a resource catalog record with a monotonic project-prefixed ID such as APP-R1. Managed resources own their Markdown body through --content/--file (or receive a generated heading); --path instead creates a safe project-relative reference whose current source is reopened on every read. Referenced --path and managed content are mutually exclusive.\n\nExamples:\n  tasker create resource \"Product brief\" -p APP --kind brief --status active --content \"# Product\"\n  tasker create resource \"Architecture\" -p APP --kind design --path docs/architecture.md"
    )]
    Resource(CreateResourceArgs),
    /// Create a proposed architectural or product decision
    #[command(
        long_about = "Create a proposed decision such as APP-D1. Supplied content must contain Context, Options considered, Decision, Rationale, and Consequences ATX headings in that order. Without body input Tasker creates a valid editable five-section decision template.\n\nExamples:\n  tasker create decision \"Filesystem authority\" -p APP\n  tasker create decision \"Filesystem authority\" -p APP --scope project --file decision.md"
    )]
    Decision(CreateDecisionArgs),
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
    /// Knowledge reference ID and role, for example APP-R2:implements; repeat as needed
    #[arg(long = "context-ref")]
    pub context_refs: Vec<String>,
    /// Resource or decision ID to link with the informed_by role; repeat as needed
    #[arg(long = "from")]
    pub from: Vec<String>,
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
    /// List resources in numeric ID order
    #[command(
        long_about = "List resource summaries from the live resources directory. Archived resources are omitted unless --include-archived or an explicit archived status is requested. Filters combine deterministically.\n\nExample:\n  tasker list resources -p APP --kind design --status active -o json"
    )]
    Resources(ResourceListArgs),
    /// List decisions in numeric ID order
    #[command(
        long_about = "List decision summaries with status, scope, and tag filters. Decision files are scanned on every invocation, so manual edits are immediately visible.\n\nExample:\n  tasker list decisions -p APP --status accepted --scope project -o json"
    )]
    Decisions(DecisionListArgs),
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
    /// Get a resource and its current managed or referenced content
    #[command(
        long_about = "Get resource metadata and live content. Referenced resources reopen their safe project-relative source path; Tasker never copies or modifies that source. IDs are case-insensitive.\n\nExample:\n  tasker get resource app-r2 -o json"
    )]
    Resource { resource: String },
    /// Get a decision and its complete Markdown content
    #[command(
        long_about = "Get complete decision metadata and body, including lifecycle attribution and supersession backlinks.\n\nExample:\n  tasker get decision APP-D1 -o json --pretty"
    )]
    Decision { decision: String },
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
    /// Update resource metadata or source-appropriate content
    #[command(
        long_about = "Update a resource under the project lock. Managed resources accept --content/--file; referenced resources accept --path. Source type cannot change. Byte-identical updates preserve revision and create no event.\n\nExample:\n  tasker update resource APP-R2 --status active --tag architecture --if-revision 1"
    )]
    Resource(UpdateResourceArgs),
    /// Update a proposed decision; settled decisions permit tags only
    #[command(
        long_about = "Update decision title, scope, content, or tags while proposed. Accepted, rejected, and superseded decisions permit only non-material tag changes. Status changes use tasker decision.\n\nExample:\n  tasker update decision APP-D1 --file revised.md --if-revision 1"
    )]
    Decision(UpdateDecisionArgs),
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
    /// Search current resource metadata and Markdown with AND-term matching
    #[command(
        long_about = "Search resource IDs, titles, kinds, statuses, tags, paths, catalog bodies, and current referenced source text. Archived resources are included by default. Repeated tags use AND and statuses use OR.\n\nExample:\n  tasker search resources \"oauth flow\" -p APP --kind design --full -o json"
    )]
    Resources(SearchResourcesArgs),
    /// Search current decision metadata and Markdown with AND-term matching
    #[command(
        long_about = "Search decision IDs, titles, statuses, scopes, tags, supersession metadata, and Markdown bodies. Rejected and superseded decisions are included by default.\n\nExample:\n  tasker search decisions filesystem -p APP --status accepted --full -o json"
    )]
    Decisions(SearchDecisionsArgs),
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

#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// Manage PROJECT.md
    #[command(
        long_about = "Manage the root PROJECT.md file without frontmatter. Reads reflect manual edits immediately; mutations lock the project, atomically replace content, and record hashes and byte counts.\n\nExample:\n  tasker project brief show -p APP"
    )]
    Brief {
        #[command(subcommand)]
        command: ProjectBriefCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum ProjectBriefCommand {
    /// Print the current project brief
    #[command(
        long_about = "Show current PROJECT.md. Human and markdown output is raw Markdown; JSON and YAML return {path,content}. This read-only command never initializes a missing file.\n\nExample:\n  tasker project brief show -p APP -o markdown"
    )]
    Show,
    /// Return the editable relative and absolute PROJECT.md paths
    #[command(
        long_about = "Return project_relative_path=PROJECT.md and its absolute path. The file need not exist, making this useful to agents preparing legacy projects.\n\nExample:\n  tasker project brief path -p APP -o json"
    )]
    Path,
    /// Initialize the exact default template
    #[command(
        long_about = "Create the detailed default PROJECT.md template for a legacy project. Existing content is protected unless --force; byte-identical forced initialization is a no-op.\n\nExample:\n  tasker project brief init -p APP --force"
    )]
    Init {
        /// Replace an existing brief with the default template
        #[arg(long)]
        force: bool,
    },
    /// Atomically replace PROJECT.md
    #[command(
        long_about = "Set PROJECT.md from --content, a UTF-8 --file, or stdin. Direct content takes precedence over no input; identical bytes preserve history.\n\nExamples:\n  tasker project brief set -p APP --content \"# App\"\n  tasker project brief set -p APP --file brief.md"
    )]
    Set(BodyInputArgs),
}

#[derive(Debug, Args)]
pub struct BodyInputArgs {
    /// Inline Markdown body
    #[arg(long, conflicts_with = "file")]
    pub content: Option<String>,
    /// UTF-8 Markdown body file
    #[arg(long, conflicts_with = "content")]
    pub file: Option<String>,
}

#[derive(Debug, Args)]
pub struct KnowledgeInputArgs {
    /// Structured input format; direct flags override supplied fields
    #[arg(short = 'i', long, value_enum, default_value = "human")]
    pub input: InputFormat,
    /// JSON/YAML structured input file; otherwise structured input reads stdin
    #[arg(long)]
    pub input_file: Option<String>,
}

#[derive(Debug, Args)]
pub struct CreateResourceArgs {
    /// Resource title; may be supplied by structured input
    pub title: Option<String>,
    /// Resource kind: brief, requirement, design, research, plan, or reference
    #[arg(long, value_enum)]
    pub kind: Option<ResourceKind>,
    /// Initial status; defaults to draft
    #[arg(long, value_enum)]
    pub status: Option<ResourceStatus>,
    /// Inline managed catalog Markdown
    #[arg(long, conflicts_with_all = ["file", "path"])]
    pub content: Option<String>,
    /// Managed catalog Markdown file
    #[arg(long, conflicts_with_all = ["content", "path"])]
    pub file: Option<String>,
    /// Safe project-relative current source path; mutually exclusive with managed content
    #[arg(long, conflicts_with_all = ["content", "file"])]
    pub path: Option<String>,
    /// Lowercase tag; repeat as needed
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    /// Include active content in project briefs
    #[arg(long, conflicts_with = "exclude_from_project_brief")]
    pub include_in_project_brief: bool,
    /// Exclude content from project briefs
    #[arg(long, conflicts_with = "include_in_project_brief")]
    pub exclude_from_project_brief: bool,
    #[command(flatten)]
    pub input_args: KnowledgeInputArgs,
}

#[derive(Debug, Args)]
pub struct UpdateResourceArgs {
    /// Project-prefixed resource ID, for example APP-R2
    pub resource: String,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long, value_enum)]
    pub kind: Option<ResourceKind>,
    #[arg(long, value_enum)]
    pub status: Option<ResourceStatus>,
    #[arg(long, conflicts_with_all = ["file", "path"])]
    pub content: Option<String>,
    #[arg(long, conflicts_with_all = ["content", "path"])]
    pub file: Option<String>,
    /// Replacement safe path; valid only for a referenced resource
    #[arg(long, conflicts_with_all = ["content", "file"])]
    pub path: Option<String>,
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    #[arg(long)]
    pub clear_tags: bool,
    #[arg(long, conflicts_with = "exclude_from_project_brief")]
    pub include_in_project_brief: bool,
    #[arg(long, conflicts_with = "include_in_project_brief")]
    pub exclude_from_project_brief: bool,
    #[arg(long)]
    pub if_revision: Option<u64>,
    #[command(flatten)]
    pub input_args: KnowledgeInputArgs,
}

#[derive(Debug, Args, Default)]
pub struct ResourceListArgs {
    #[arg(long = "kind", value_enum)]
    pub kinds: Vec<ResourceKind>,
    #[arg(long = "status", value_enum)]
    pub statuses: Vec<ResourceStatus>,
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    #[arg(long)]
    pub include_archived: bool,
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
    #[arg(long, default_value_t = 0)]
    pub offset: usize,
    #[arg(long)]
    pub full: bool,
}

#[derive(Debug, Args)]
pub struct CreateDecisionArgs {
    /// Decision title; may be supplied by structured input
    pub title: Option<String>,
    /// Project-wide or linked-only applicability; defaults to project
    #[arg(long, value_enum)]
    pub scope: Option<DecisionScope>,
    #[arg(long, conflicts_with = "file")]
    pub content: Option<String>,
    #[arg(long, conflicts_with = "content")]
    pub file: Option<String>,
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    #[command(flatten)]
    pub input_args: KnowledgeInputArgs,
}

#[derive(Debug, Args)]
pub struct UpdateDecisionArgs {
    pub decision: String,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long, value_enum)]
    pub scope: Option<DecisionScope>,
    #[arg(long, conflicts_with = "file")]
    pub content: Option<String>,
    #[arg(long, conflicts_with = "content")]
    pub file: Option<String>,
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    #[arg(long)]
    pub clear_tags: bool,
    #[arg(long)]
    pub if_revision: Option<u64>,
    #[command(flatten)]
    pub input_args: KnowledgeInputArgs,
}

#[derive(Debug, Args, Default)]
pub struct DecisionListArgs {
    #[arg(long = "status", value_enum)]
    pub statuses: Vec<DecisionStatus>,
    #[arg(long = "scope", value_enum)]
    pub scopes: Vec<DecisionScope>,
    #[arg(long = "tag")]
    pub tags: Vec<String>,
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
    #[arg(long, default_value_t = 0)]
    pub offset: usize,
    #[arg(long)]
    pub full: bool,
}

#[derive(Debug, Subcommand)]
pub enum ArchiveCommand {
    /// Set a resource status to archived without deleting it
    #[command(
        long_about = "Archive a resource under the project lock. Repeating archive on an archived record is a no-op. Referenced source files are never touched.\n\nExample:\n  tasker archive resource APP-R2 --if-revision 2"
    )]
    Resource(KnowledgeRevisionArgs),
}

#[derive(Debug, Args)]
pub struct KnowledgeRevisionArgs {
    pub id: String,
    #[arg(long)]
    pub if_revision: Option<u64>,
}

#[derive(Debug, Subcommand)]
pub enum DecisionCommand {
    /// Transition proposed to accepted and set decision attribution
    #[command(
        long_about = "Accept a proposed decision, setting decided_at and decided_by and incrementing revision exactly once.\n\nExample:\n  tasker decision accept APP-D1 --if-revision 1"
    )]
    Accept(KnowledgeRevisionArgs),
    /// Transition proposed to rejected and set decision attribution
    #[command(
        long_about = "Reject a proposed decision, setting decided_at and decided_by and retaining its historical record.\n\nExample:\n  tasker decision reject APP-D2 --if-revision 1"
    )]
    Reject(KnowledgeRevisionArgs),
    /// Atomically supersede an accepted decision with another accepted decision
    #[command(
        long_about = "Set the old accepted decision to superseded and link both records atomically while holding the project lock. One accepted replacement may supersede multiple predecessors, but each predecessor may have only one successor. Self-links, duplicate edges, stale revisions, and cycles are rejected.\n\nExample:\n  tasker decision supersede APP-D1 --with APP-D3 --if-revision 2 --with-if-revision 2"
    )]
    Supersede(SupersedeArgs),
}

#[derive(Debug, Args)]
pub struct SupersedeArgs {
    pub decision: String,
    #[arg(long = "with")]
    pub replacement: String,
    #[arg(long)]
    pub if_revision: Option<u64>,
    #[arg(long)]
    pub with_if_revision: Option<u64>,
}

#[derive(Debug, Subcommand)]
pub enum ContextRefCommand {
    /// Add an existing same-project knowledge target
    #[command(
        long_about = "Add one Task.context_refs edge. An exact duplicate is an idempotent no-op; the same target with another role is a conflict.\n\nExample:\n  tasker context-ref add APP-12 APP-R2 --role informed_by --if-revision 3"
    )]
    Add(ContextRefAddArgs),
    /// List references in canonical kind/numeric/role order
    #[command(
        long_about = "List authoritative knowledge links stored on a task in deterministic canonical order.\n\nExample:\n  tasker context-ref list APP-12 -o json"
    )]
    List(TaskIdArgs),
    /// Remove one target link
    #[command(
        long_about = "Remove a task knowledge link by target ID and increment the task revision once.\n\nExample:\n  tasker context-ref remove APP-12 APP-R2 --if-revision 4"
    )]
    Remove(ContextRefRemoveArgs),
    /// Find all tasks linking a resource or decision
    #[command(
        long_about = "Scan current task files and return references to a resource or decision in numeric task order.\n\nExample:\n  tasker context-ref reverse APP-D1 -o json"
    )]
    Reverse { knowledge_id: String },
}

#[derive(Debug, Args)]
pub struct ContextRefAddArgs {
    pub task: String,
    pub knowledge_id: String,
    #[arg(long, value_enum)]
    pub role: ContextReferenceRole,
    #[arg(long)]
    pub if_revision: Option<u64>,
}

#[derive(Debug, Args)]
pub struct ContextRefRemoveArgs {
    pub task: String,
    pub knowledge_id: String,
    #[arg(long)]
    pub if_revision: Option<u64>,
}

#[derive(Debug, Args)]
pub struct SearchResourcesArgs {
    pub query: Option<String>,
    #[command(flatten)]
    pub filters: ResourceListArgs,
}

#[derive(Debug, Args)]
pub struct SearchDecisionsArgs {
    pub query: Option<String>,
    #[command(flatten)]
    pub filters: DecisionListArgs,
}

#[derive(Debug, Args)]
pub struct BriefArgs {
    /// Optional task ID; omit for a project-level brief
    pub task: Option<String>,
    /// Exact final output byte limit, including the trailing newline
    #[arg(long, default_value_t = 50000)]
    pub max_bytes: usize,
    /// Permit archived explicitly linked resources in a task brief
    #[arg(long)]
    pub include_archived: bool,
}
