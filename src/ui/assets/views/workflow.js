import { dot, el, emptyState, pageHeader, sectionHeading } from "../dom.js";
import { projectFrom, stateColor, stateEntries, stateLabel, workflowFrom } from "../model.js";

export function renderWorkflow(ctx) {
  const project = projectFrom(ctx.snapshot);
  const workflow = workflowFrom(ctx.snapshot);
  const states = stateEntries(ctx.snapshot);
  const relations = Object.entries(project.relations || workflow.relations || {});
  const root = el("div", {}, pageHeader(
    null,
    "Workflow",
    "States, transitions and terminal behaviour for this project, read from tasker.yaml. Every other view reads this.",
  ));

  if (!states.length) root.append(emptyState("No workflow states", "The project configuration did not return any states."));
  else root.append(el("div", { className: "workflow-grid" }, states.map(([key, config]) => {
    const transitions = workflow.transitions?.[key] || [];
    const flags = [
      config.terminal ? ["terminal", "accent"] : ["non-terminal", ""],
      config.dependency_satisfied ? ["satisfies dependents", "ok"] : ["does not satisfy", ""],
      key === workflow.initial_state ? ["initial", "cool"] : null,
      key === workflow.claim_state ? ["claim state", "cool"] : null,
    ].filter(Boolean);
    return el("section", { className: "workflow-state", "aria-labelledby": `workflow-${key}` },
      el("div", { className: "workflow-state-top" },
        dot(stateColor(ctx.snapshot, key)),
        el("h2", { id: `workflow-${key}`, text: config.label || key }),
        el("code", { text: key }),
      ),
      el("div", { className: "workflow-flags" }, flags.map(([label, tone]) => el("span", { className: `pill ${tone}`.trim(), text: label }))),
      el("div", { className: "transition-heading", text: config.terminal ? "Can reopen to" : "Transitions to" }),
      transitions.length
        ? el("ul", { className: "transition-list", "aria-label": `Transitions from ${config.label || key}` }, transitions.map((target) => el("li", { title: stateLabel(ctx.snapshot, target) },
          dot(stateColor(ctx.snapshot, target)), target,
        )))
        : el("p", { className: "field-hint", text: "None" }),
    );
  })));

  root.append(sectionHeading("Relation types", `${relations.length} configured`));
  if (!relations.length) root.append(emptyState("No relation types", "Dependencies remain available even when no non-blocking relation types are configured."));
  else root.append(el("section", { className: "panel" },
    el("ul", { className: "relation-list" }, relations.map(([key, relation]) => el("li", {},
      el("div", {},
        el("span", { className: "mono", text: key }),
        el("small", { text: `Inverse label: ${relation.inverse_label || "—"}` }),
      ),
      el("div", { className: "tag-list" },
        el("span", { className: "pill", text: relation.symmetric ? "symmetric" : "directional" }),
        el("span", { className: `pill ${relation.acyclic ? "accent" : ""}`.trim(), text: relation.acyclic ? "acyclic" : "cycles allowed" }),
      ),
    ))),
  ));
  return root;
}
