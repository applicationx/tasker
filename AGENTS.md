# AGENTS.md

Guidance for coding agents changing Tasker itself.

## Product invariants

- Tasker is a synchronous, local-first filesystem primitive. Normal CLI commands remain one-shot, offline, and require no prerequisite process. The sole exception is the explicitly invoked `tasker ui` feature, which may run one transient child process per projects root, bind only to numeric loopback on an OS-assigned port, and serve embedded UI/API assets. It must never auto-start, register as an OS service, bind non-loopback, make outbound network requests, maintain authoritative persistent state or indexes, or become required by CLI workflows. The explicitly invoked `tasker mcp` is a second exception: a synchronous stdio Model Context Protocol server launched by its client, which opens no socket, makes no network request, keeps no state of its own, never consumes stdin as command input, runs every tool through the normal CLI code path, and exits when stdin closes. No other daemon, service, database, async runtime, networking, telemetry, authentication, or authorization may be added.
- Board roots resolve from `TASKER_ROOT`, then the nearest real `.tasker` folder at or above the current directory (committed with the code), then global config, then `~/tasker`. The global `registry.toml` is a non-authoritative list of known roots for `tasker list`; it is not a search index, entries are re-read on use, and missing entries are warnings. Tests must set `TASKER_CONFIG_DIR` whenever they run without `TASKER_ROOT` or touch the registry.
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

When running tests from an agent shell, close stdin (`mise run check < /dev/null`, or `$null | mise run check` in PowerShell). Some commands read stdin when no content is given, and an open pipe makes those tests hang.

## Releasing to AppX Nexus

Tasker is published to the private Nexus cargo registry. `.cargo/config.toml` defines `appx-hosted` (`cargo-hosted`, the publish target) and `appx` (`cargo-group`, used for installs), and `Cargo.toml` restricts `publish` to `appx-hosted`. Never publish to crates.io.

When the user asks to release, publish, or ship a version, do all of the following:

1. Start from a clean working tree containing the intended commits. Ask before including unrelated uncommitted changes.
2. Pick the next version with semver. Breaking CLI, JSON/YAML, error-code, or persistence changes get a minor bump while the version is 0.x, and a major bump after that. Everything else gets a patch bump. Ask the user if the bump is unclear. A version that already exists in Nexus can never be published again.
3. Bump `version` in `Cargo.toml`, run `cargo check --locked` so `Cargo.lock` updates, and commit both as `Release tasker vX.Y.Z`.
4. Run `mise run publish < /dev/null`. It runs `mise run check` and then `cargo publish --locked --registry appx-hosted`.
5. After a successful publish, tag the commit `vX.Y.Z`. Push the commit and tag only when the user asks.
6. Verify with `cargo info tasker --registry appx` (Nexus has no search API, so `cargo search` returns 404), or `cargo install tasker --registry appx --locked --force` when the user wants the new binary installed. Then report the published version.

If cargo reports "authenticated registries require a credential-provider", "registry index was not found", a missing token, or 401 for `appx`/`appx-hosted`, the machine setup is incomplete. The `appx-nexus-cargo` skill covers the setup for Windows, macOS, Linux, and CI. Do not write tokens into the repository, `credentials.toml`, or shell history. Instead, give the user the one-time setup from README "Install from AppX Nexus": the `~/.cargo/config.toml` snippet with `cargo:wincred`, and `cargo login` fed from `op read 'op://programming/nexus - appx/...'`. You may run it yourself only if the user explicitly approves it in the current session. Never print the password or token.

`mise run package` builds the exact crate without uploading it. Use it to check packaging changes. When a new top-level file is needed at build time, add it to `include` in `Cargo.toml`.

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
