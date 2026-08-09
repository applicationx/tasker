# AGENTS.md

Guidance for coding agents changing Tasker itself.

## Product invariants

- Tasker is a synchronous, local-first filesystem primitive. Do not add a daemon, service, database, async runtime, networking, telemetry, authentication, or authorization.
- The filesystem is authoritative. Manual edits and Git checkouts must be immediately visible; do not add hidden required state or a persistent search index.
- Keep one JSON file per task/user and one immutable JSON file per changelog event. Preserve the documented schema, pretty formatting, trailing newline, deterministic ordering, and focused Git diffs.
- All project mutations must take the stable project-level `.tasker.lock`, reload relevant data after locking, validate, and use atomic replacement for mutable files. Never replace the lock file itself.
- Dependency and configured acyclic-relation cycle correctness, workflow enforcement, and concurrent claim correctness are higher priority than presentation.
- CLI syntax, JSON/YAML shapes, machine error codes, exit categories, stdout/stderr separation, and help examples are public APIs. Commands remain deterministic and non-interactive.
- Keep agent workflows discoverable from `tasker --help`. Do not require README knowledge for routine claim/search/create/update/acceptance/context/dependency/relation/transition/history operations.
- Keep `description` focused on what must be done. Store measurable completion checkpoints in `acceptance_criteria`; store curated agent progress, context, and decisions in the append-only task `context` list. The project changelog remains the machine audit trail.

## Toolchain and checks

Rust 2024 is managed by `mise.toml`.

```sh
mise install
mise run fmt
mise run clippy
mise run test
mise run check
mise run build
```

Before completing any change, `mise run check` and `mise run build` must pass. Do not suppress Clippy warnings or skip tests to obtain a green run.

## Change discipline

- Prefer concrete synchronous functions and typed Serde models over framework abstractions.
- Reject unknown persistent fields when this catches manual-edit typos.
- Avoid rewriting an unchanged managed file.
- Do not hard-code project state names beyond interpreting configured `initial_state`, `claim_state`, `terminal`, `dependency_satisfied`, and transitions.
- Do not implement hard task/project deletion; cancellation is a workflow operation.
- Do not add cross-project dependencies or relations.
- When a CLI contract or persistence format changes, update unit tests, CLI integration tests, root/subcommand help, and README examples together.
- Include a competing-process test for changes affecting locks, claims, ID allocation, or write ordering.
- Tests must use temporary `TASKER_ROOT` values and must not write global user configuration.

The primary acceptance path lives in `tests/cli.rs` and covers project/task lifecycle, machine errors, structured input, workflow, acceptance criteria, task context, dependencies, relations, users, project inference, validation, history, and concurrent claims.
