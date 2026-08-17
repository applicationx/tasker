import { badge, el, emptyState, pageHeader, sectionHeading } from "../dom.js";
import { projectFrom, stateEntries, stateLabel, workflowFrom } from "../model.js";

export function renderWorkflow(ctx) {
  const project = projectFrom(ctx.snapshot);
  const workflow = workflowFrom(ctx.snapshot);
  const states = stateEntries(ctx.snapshot);
  const relations = Object.entries(project.relations || workflow.relations || {});
  const root = el("div", {}, pageHeader(
    ctx.prefix,
    "Configured workflow",
    "States, transition paths, completion semantics, and relation types come from tasker.yaml and are read-only here.",
  ));

  root.append(sectionHeading("States", `${states.length} configured`));
  if (!states.length) root.append(emptyState("No workflow states", "The project configuration did not return any states."));
  else root.append(el("div", { className: "grid grid-3" }, states.map(([key, config], index) => el("section", { className: "card workflow-state" },
    el("div", { className: "workflow-state-top" },
      el("h2", { text: config.label || key }),
      badge(config.terminal ? "Terminal" : "Active", config.terminal ? "success" : index),
    ),
    el("code", { text: key }),
    el("div", { className: "tag-list" },
      key === workflow.initial_state ? el("span", { className: "tag", text: "Initial state" }) : null,
      key === workflow.claim_state ? el("span", { className: "tag", text: "Claim state" }) : null,
      el("span", { className: "tag", text: config.dependency_satisfied ? "Satisfies dependents" : "Does not satisfy dependents" }),
    ),
    el("div", { className: "transition-list" },
      el("strong", { text: "Transitions" }),
      (workflow.transitions?.[key] || []).length
        ? (workflow.transitions[key]).flatMap((target, targetIndex) => [targetIndex ? el("span", { text: "," }) : null, el("span", {}, el("span", { className: "transition-arrow", text: "→ " }), stateLabel(ctx.snapshot, target))])
        : el("span", { text: "None" }),
    ),
  ))));

  root.append(sectionHeading("Relation types", `${relations.length} configured`));
  if (!relations.length) root.append(emptyState("No relation types", "Dependencies remain available even when no non-blocking relation types are configured."));
  else root.append(el("section", { className: "card card-body" },
    el("ul", { className: "compact-list" }, relations.map(([key, relation]) => el("li", {},
      el("div", { className: "list-main" },
        el("span", { className: "mono", text: key }),
        el("div", { className: "list-meta" }, `Inverse label: ${relation.inverse_label || "—"}`),
      ),
      el("div", { className: "tag-list" },
        el("span", { className: "tag", text: relation.symmetric ? "Symmetric" : "Directional" }),
        el("span", { className: "tag", text: relation.acyclic ? "Acyclic" : "Cycles allowed" }),
      ),
    ))),
  ));
  return root;
}
