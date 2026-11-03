---
name: tasker
description: Plan, claim, track, and complete work with the local-first Tasker CLI or its MCP server. Use whenever work must be created, decomposed, blocked, claimed, evidenced, verified, or completed in Tasker, or when the user mentions tasker, a .tasker board, task IDs like APP-12, or Tasker projects.
---

# Tasker

Tasker keeps tasks as plain JSON files that the filesystem owns. Always change them through `tasker` commands (or the `tasker_*` MCP tools), never by hand-editing task files: commands take the project lock, validate, and write the audit log.

`tasker --help` and `tasker <command> --help` are the full contract. Add `-o json` for machine output; errors go to stderr as `{"error":{"code",...}}`.

## Where boards live

A *board* (projects root) holds one folder per project. The root resolves in this order:

1. `--root PATH` (one command) or `tasker mcp --root PATH`
2. `TASKER_ROOT`
3. the nearest `.tasker/` folder at or above the current directory (a board committed with the code; create it with `tasker init`)
4. global config `projects_root` (`tasker config set projects-root ~/tasker`)
5. `~/tasker`

Project selectors (`-p APP`, or the prefix of an ID such as `APP-12`) are searched in the effective root first, then the home root (4/5), then roots registered with `tasker root add`. So from inside a repo you can still reach home projects by prefix.

```sh
tasker config show -o json          # effective root and root_source
tasker list -o json                 # every known board and its projects
tasker init --name "App" --prefix APP          # board inside this repo
tasker --root ~/tasker create project "Notes" --prefix NOTE   # board outside the repo
```

## Core loop

```sh
tasker next -p APP -o json
tasker claim next -p APP --as my-agent -o json      # or: tasker claim APP-12 --as my-agent
tasker brief APP-12 -o markdown                     # contract, blockers, decisions, context
tasker status APP-12 -o json                        # blocked_by, blocks, allowed transitions
tasker context add APP-12 "EVIDENCE: tests pass (cargo test)" --kind progress
tasker acceptance check APP-12 AC-1 --actor my-agent
tasker transition APP-12 done --actor my-agent
tasker validate -p APP -o json
```

Only check an acceptance criterion after you have evidence. Read allowed states from `status`; workflows are configurable, so don't assume `done` or `in_progress` exist.

## Creating work

```sh
tasker create task "Implement parser" -p APP --description "..." \
  --acceptance-criterion "Parser tests pass" --tag backend -o json
tasker type list -p APP -o json                     # configured task types
tasker create task "Which storage format?" -p APP --type question --blocks APP-12
tasker create task "Login happy path" -p APP --type test_case --parent APP-3
```

Keep `description` to what must be done. Put measurable checkpoints in acceptance criteria and progress or decisions in `context add`.

## Blocking: blocked_by / blocks

`APP-12 blocked_by APP-4` means APP-12 waits until APP-4 reaches a state with `dependency_satisfied: true` (usually done). A blocked task cannot be claimed, cannot be transitioned into the claim state, and cannot be completed. The error code is `dependencies_unsatisfied`.

```sh
tasker dependency add APP-12 APP-4                 # APP-12 blocked_by APP-4
tasker relation add APP-12 blocked_by APP-4        # same edge
tasker relation add APP-4 blocks APP-12            # same edge, written from the blocker
tasker dependency list APP-4 --reverse             # what APP-4 blocks
tasker list tasks -p APP --dependency-blocked -o json
```

Cycles are rejected with `dependency_cycle`.

## Task types and relations

Types such as `task`, `story`, `bug`, `question` and `test_case` are configured in the project's `tasker.yaml` (`task_types`, `default_task_type`). Add one with `tasker type add spike -p APP --label "Spike"`, change a task's type with `tasker update task APP-7 --type story`, and filter with `--type`.

Relation types (`subtask_of`, `relates_to`, ...) can restrict the task types at each end with `source_types` / `target_types`, for example only `test_case` or `task` as `subtask_of` a `story`. A rule violation returns `relation_type_mismatch`. After editing `tasker.yaml`, run `tasker validate`.

## MCP

`tasker mcp` serves the same commands over stdio. Tools: `tasker_list_boards`, `tasker_list_tasks`, `tasker_get_task`, `tasker_status`, `tasker_next`, `tasker_claim`, `tasker_create_task`, `tasker_update_task`, `tasker_transition`, `tasker_acceptance`, `tasker_context_add`, `tasker_dependency`, `tasker_relation`, `tasker_task_types`, `tasker_search`, `tasker_brief`, `tasker_validate`, `tasker_changelog`, `tasker_init`, and `tasker_cli` for anything else. Every tool accepts an optional `root` (and `project`). Use them when the MCP server is configured; otherwise run the CLI.

## Report

When you finish, report the changed IDs, the validation result, and any blocker with its error code.
