import {
  avatar, button, dot, el, id, labeledField, panelLabel, relativeTime, relativeTimeElement, segmented,
  selectControl, tagList, timeElement, tint,
} from "../dom.js";
import {
  decisionsFrom, normalizeTaskDetail, resourcesFrom, stateColor, stateConfig, stateLabel, taskRecord,
  userFrom, usersFrom, workflowFrom,
} from "../model.js";
import { decisionHref, projectHref, resourceHref, taskHref } from "../router.js";
import {
  dirtyForm, mutationUnavailable, parseTags, statusBadge, submitMutation, tagsInputValue, typePill,
} from "./shared.js";

const CONTEXT_COLORS = { context: "var(--cool)", progress: "var(--accent)", decision: "var(--ok)" };

export async function renderTask(ctx) {
  const value = await ctx.api.task(ctx.prefix, ctx.route.entityId, { signal: ctx.signal });
  const { task, status } = normalizeTaskDetail(value, ctx.snapshot, ctx.route.entityId);
  if (!task) throw { code: "task_not_found", message: `${ctx.route.entityId} was not found.` };

  let mode = ctx.route.query.get("tab") === "working" ? "working" : "document";
  const description = descriptionPanel(ctx, task);
  const criteria = acceptancePanel(ctx, task, status);
  const links = linksPanel(ctx, task);
  const context = contextPanel(ctx, task);
  const main = el("div", { className: "detail-main" });
  const layout = el("div", { className: "detail-layout" }, main, rail(ctx, task, status));

  const arrange = () => {
    layout.classList.toggle("working", mode === "working");
    description.querySelector(".description")?.classList.toggle("compact", mode === "working");
    main.replaceChildren(...(mode === "working" ? [criteria, description, links, context] : [description, criteria, links, context]));
  };
  const toggle = segmented("Task layout", [
    { value: "document", label: "Document" }, { value: "working", label: "Working" },
  ], mode, (next) => {
    mode = next;
    for (const segment of toggle.querySelectorAll("button")) segment.setAttribute("aria-pressed", String(segment.textContent.toLowerCase() === mode));
    arrange();
    window.history.replaceState(null, "", `${taskHref(ctx.prefix, task.id)}${mode === "working" ? "?tab=working" : ""}`);
  });
  arrange();

  return el("div", {},
    el("div", { className: "detail-toolbar" },
      el("a", { className: "button ghost", href: projectHref(ctx.prefix, "tasks"), text: "← Tasks" }),
      el("span", { className: "spacer" }),
      toggle,
    ),
    el("div", { className: "detail-title" },
      el("span", { className: "identifier", text: task.id }),
      typePill(task),
      el("h1", { text: task.header, tabindex: "-1", dataset: { routeHeading: "true" } }),
    ),
    stateRow(ctx, task, status),
    mutationUnavailable(ctx),
    layout,
  );
}

function stateRow(ctx, task, status) {
  const workflow = workflowFrom(ctx.snapshot);
  const current = stateConfig(ctx.snapshot, task.state);
  const allowed = Array.isArray(status.allowed_transitions) ? status.allowed_transitions : workflow.transitions?.[task.state] || [];
  const remaining = status.acceptance_criteria_remaining?.length ?? (task.acceptance_criteria || []).filter((item) => !item.completed).length;
  const container = el("div", {});
  const row = el("div", { className: "state-row" },
    tint(el("span", { className: "state-current", text: stateLabel(ctx.snapshot, task.state) }), stateColor(ctx.snapshot, task.state)),
    allowed.length ? el("span", { className: "state-arrow", "aria-hidden": "true", text: "→" }) : null,
    allowed.map((target) => {
      const gated = remaining > 0 && stateConfig(ctx.snapshot, target).dependency_satisfied;
      const label = `${current.terminal ? "reopen to " : ""}${stateLabel(ctx.snapshot, target)}`;
      return el("button", {
        type: "button", className: "transition-chip", text: gated ? `${label} ✕` : label,
        disabled: !ctx.actor() || gated,
        title: gated ? `${remaining} acceptance ${remaining === 1 ? "criterion is" : "criteria are"} still unchecked` : `Transition to ${stateLabel(ctx.snapshot, target)}`,
        onclick: () => submitMutation(ctx, container, {
          operation: "task.transition", task_id: task.id, if_revision: task.revision, target_state: target,
        }, { target_state: target }, `Task transitioned to ${stateLabel(ctx.snapshot, target)}.`),
      });
    }),
    el("span", { className: "spacer" }),
    el("span", { className: "revision-badge", text: `rev ${task.revision ?? "—"}` }),
    el("a", { className: "button primary", href: `${taskHref(ctx.prefix, task.id)}/brief`, text: "Agent brief" }),
  );
  container.append(row);
  return container;
}

function descriptionPanel(ctx, task) {
  const header = el("input", { type: "text", value: task.header, required: true, maxlength: "500" });
  const description = el("textarea", { text: task.description || "", rows: "8" });
  const tags = el("input", { type: "text", value: tagsInputValue(task.tags), placeholder: "frontend, urgent" });
  const form = el("form", {},
    el("div", { className: "form-grid" },
      labeledField("Header", header, { full: true }),
      labeledField("Description", description, { full: true, hint: "Describe what must be done. Keep measurable checkpoints in acceptance criteria." }),
      labeledField("Tags", tags, { full: true, hint: "Comma-separated; Tasker normalizes and sorts tags." }),
    ),
    el("div", { className: "form-actions" },
      button("Save task content", { kind: "submit", className: "button primary", disabled: !ctx.actor() }),
      el("span", { className: "form-status", dataset: { formStatus: "true" } }),
    ),
  );
  dirtyForm(form, ctx);
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const fields = { header: header.value.trim(), description: description.value, tags: parseTags(tags.value) };
    if (!fields.header) return header.focus();
    await submitMutation(ctx, form, {
      operation: "task.update", task_id: task.id, if_revision: task.revision, patch: fields,
    }, fields, "Task content saved.");
  });
  return el("section", { className: "panel", "aria-label": "Description" },
    panelLabel("Description"),
    el("p", { className: `description ${task.description ? "" : "empty"}`.trim(), text: task.description || "No description yet." }),
    el("details", { className: "editor" }, el("summary", {}, el("span", { className: "sr-only", text: "Edit task content" })), form),
  );
}

function acceptancePanel(ctx, task, status) {
  const criteria = Array.isArray(task.acceptance_criteria) ? task.acceptance_criteria : [];
  const done = criteria.filter((item) => item.completed).length;
  const remaining = criteria.length - done;
  const panel = el("section", { className: "panel", "aria-label": "Acceptance criteria" },
    panelLabel("Acceptance criteria", el("span", { className: "panel-note", text: criteria.length ? `${done} / ${criteria.length} · ${remaining} left` : "none" })),
  );
  if (criteria.length) {
    panel.append(el("ul", { className: "criteria-list" }, criteria.map((criterion) => el("li", {},
      el("button", {
        type: "button", className: "criterion", "aria-pressed": String(Boolean(criterion.completed)), disabled: !ctx.actor(),
        "aria-label": `${criterion.completed ? "Reopen" : "Complete"} ${criterion.id}: ${criterion.text}`,
        onclick: () => submitMutation(ctx, panel, {
          operation: "task.acceptance.set", task_id: task.id, criterion_id: criterion.id,
          completed: !criterion.completed, if_revision: task.revision,
        }, { criterion_id: criterion.id, completed: !criterion.completed }, criterion.completed ? "Criterion reopened." : "Criterion completed."),
      },
      el("span", { className: "check-box", "aria-hidden": "true", text: criterion.completed ? "✓" : "" }),
      el("span", { className: "criterion-copy" },
        el("span", { className: "criterion-text", text: criterion.text }),
        el("span", { className: "criterion-meta", text: criterion.completed
          ? `${criterion.id}  ·  ${criterion.completed_by || "unknown"} · ${relativeTime(criterion.completed_at)}`
          : criterion.id }),
      )),
      button("Remove", {
        className: "text-button", disabled: !ctx.actor(), "aria-label": `Remove ${criterion.id}`,
        onclick: async () => {
          if (!window.confirm(`Remove ${criterion.id}? Criterion text cannot be recovered from the task file.`)) return;
          await submitMutation(ctx, panel, {
            operation: "task.acceptance.remove", task_id: task.id, criterion_id: criterion.id, if_revision: task.revision,
          }, { criterion_id: criterion.id }, "Criterion removed.");
        },
      }),
    ))));
  } else {
    panel.append(el("p", { className: "empty-line", text: "No acceptance criteria yet." }));
  }
  const gate = Object.entries(workflowFrom(ctx.snapshot).states || {}).find(([key, config]) => config.dependency_satisfied && (status.allowed_transitions || []).includes(key));
  if (remaining > 0 && gate) {
    panel.append(el("div", { className: "notice accent inline", text: `Cannot move to ${gate[1].label || gate[0]} — ${remaining} ${remaining === 1 ? "criterion" : "criteria"} still unchecked.` }));
  }

  const text = el("input", { type: "text", required: true, placeholder: "Add a measurable completion checkpoint…", "aria-label": "New acceptance criterion" });
  const addForm = el("form", { className: "inline-form spaced" }, text,
    button("Add", { kind: "submit", className: "button", disabled: !ctx.actor() }),
  );
  dirtyForm(addForm, ctx);
  addForm.addEventListener("submit", async (event) => {
    event.preventDefault();
    await submitMutation(ctx, addForm, {
      operation: "task.acceptance.add", task_id: task.id, text: text.value.trim(), if_revision: task.revision,
    }, { text: text.value }, "Criterion added.");
  });
  panel.append(addForm);
  return panel;
}

function linksPanel(ctx, task) {
  const dependencies = Array.isArray(task.dependencies) ? task.dependencies : [];
  const relations = Array.isArray(task.relations) ? task.relations : [];
  const rows = [
    ...dependencies.map((target) => ({ kind: "depends on", target })),
    ...relations.map((relation) => ({ kind: relation.type || relation.kind || "relates", target: relation.target })),
  ];
  return el("section", { className: "panel", "aria-label": "Dependencies and relations" },
    panelLabel("Dependencies & relations"),
    rows.length ? el("ul", { className: "link-list" }, rows.map(({ kind, target }) => {
      const other = taskRecord(ctx.snapshot, target)?.task;
      return el("li", {}, el("a", { className: "link-row", href: taskHref(ctx.prefix, target) },
        el("span", { className: "link-kind", text: kind }),
        el("span", { className: "identifier", text: target }),
        el("span", { className: "row-title", text: other?.header || "Missing task" }),
        other ? statusBadge(ctx.snapshot, other.state) : null,
      ));
    })) : el("p", { className: "empty-line", text: "No dependencies or relations. Add them with the Tasker CLI." }),
  );
}

function contextPanel(ctx, task) {
  const entries = Array.isArray(task.context) ? task.context : [];
  const kind = selectControl([
    { value: "context", label: "Context" }, { value: "progress", label: "Progress" }, { value: "decision", label: "Decision" },
  ], "progress", { "aria-label": "Entry kind" });
  const message = el("input", { type: "text", required: true, placeholder: "Append context, progress or a decision…", "aria-label": "Context message" });
  const form = el("form", { className: "inline-form" }, kind, message,
    button("Append", { kind: "submit", className: "button", disabled: !ctx.actor() }),
  );
  dirtyForm(form, ctx);
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    await submitMutation(ctx, form, {
      operation: "task.context.append", task_id: task.id, kind: kind.value,
      message: message.value.trim(), if_revision: task.revision,
    }, { kind: kind.value, message: message.value }, "Context appended.");
  });
  return el("section", { className: "panel", "aria-label": "Task context" },
    panelLabel("Task context", el("span", { className: "field-hint", text: "append-only" })),
    entries.length ? el("ol", { className: "context-timeline" }, entries.map((entry) => {
      const color = CONTEXT_COLORS[entry.kind] || "var(--muted)";
      return el("li", { className: "context-entry" },
        el("div", { className: "context-rail", "aria-hidden": "true" }, dot(color)),
        el("div", { className: "context-body" },
          el("div", { className: "context-meta" },
            tint(el("span", { className: "context-kind", text: entry.kind || "context" }), color),
            el("span", { className: "identifier", text: entry.id || "" }),
            el("span", { className: "who" }, `${entry.created_by || "unknown"} · `, relativeTimeElement(entry.created_at)),
          ),
          el("p", { text: entry.message }),
        ),
      );
    })) : el("p", { className: "empty-line", text: "No context entries yet." }),
    form,
  );
}

function rail(ctx, task, status) {
  const user = userFrom(ctx.snapshot, task.assignee);
  const config = stateConfig(ctx.snapshot, task.state);
  const computed = [
    ["Dependency", status.dependency_blocked ? "blocked" : "ready", status.dependency_blocked ? "var(--bad)" : "var(--ok)"],
    ["Claimable", status.claimable ? "yes" : "no", status.claimable ? "var(--ok)" : null],
    ["Terminal", config.terminal ? "yes" : "no", null],
    ["Satisfies dependents", config.dependency_satisfied ? "yes" : "no", null],
  ];
  const references = Array.isArray(task.context_refs) ? task.context_refs : [];
  const knowledge = new Map([...resourcesFrom(ctx.snapshot), ...decisionsFrom(ctx.snapshot)].map((item) => [item.id, item]));
  return el("aside", { className: "detail-rail", "aria-label": "Task metadata" },
    el("section", { className: "panel rail-panel" },
      el("div", { className: "rail-section" },
        el("span", { className: "field-label", text: "Assignee" }),
        el("span", { className: "rail-person" },
          avatar(user, "md"),
          el("span", {},
            el("strong", { text: user ? user.name || user.id : "Unassigned" }),
            el("small", { text: user ? `${user.id} · ${user.kind || "unknown"}` : "unassigned" }),
          ),
        ),
        assignmentEditor(ctx, task),
      ),
      el("div", { className: "rail-section" },
        el("span", { className: "field-label", text: "Computed" }),
        computed.map(([label, value, color]) => el("span", { className: "computed" },
          el("span", { text: label }),
          color ? tint(el("span", { text: value }), color) : el("span", { text: value }),
        )),
      ),
      el("div", { className: "rail-section" },
        el("span", { className: "field-label", text: "Tags" }),
        tagList(task.tags) || el("span", { className: "muted", text: "No tags" }),
      ),
      el("div", { className: "rail-section" },
        el("span", { className: "field-label", text: "Record" }),
        el("dl", { className: "rail-meta" },
          el("dt", { text: "Created" }), el("dd", {}, timeElement(task.created_at)),
          el("dt", { text: "by" }), el("dd", { text: task.created_by || "—" }),
          el("dt", { text: "Updated" }), el("dd", {}, relativeTimeElement(task.updated_at)),
          el("dt", { text: "by" }), el("dd", { text: task.updated_by || "—" }),
        ),
      ),
    ),
    el("section", { className: "panel", "aria-label": "Knowledge" },
      panelLabel("Knowledge"),
      references.length ? el("ul", { className: "rail-refs" }, references.map((reference) => {
        const isDecision = reference.kind === "decision";
        const record = knowledge.get(reference.id);
        return el("li", {}, el("a", { href: isDecision ? decisionHref(ctx.prefix, reference.id) : resourceHref(ctx.prefix, reference.id) },
          el("span", { text: record?.title || reference.id }),
          tint(el("span", { text: `${reference.id} · ${reference.kind} · ${reference.role}` }), isDecision ? "var(--ok)" : "var(--cool)"),
        ));
      })) : el("p", { className: "field-hint", text: "No linked resources or decisions." }),
    ),
  );
}

function assignmentEditor(ctx, task) {
  const users = usersFrom(ctx.snapshot);
  const choices = users.map((user) => ({ value: user.id, label: user.name ? `${user.name} (${user.id})` : user.id }));
  if (task.assignee && !choices.some((item) => item.value === task.assignee)) choices.unshift({ value: task.assignee, label: task.assignee });
  if (!choices.length) return null;
  const selectId = id("assignee");
  const assignee = selectControl(choices, task.assignee || choices[0].value, { id: selectId });
  const form = el("form", {},
    el("label", { className: "sr-only", for: selectId, text: "Project user" }),
    assignee,
    el("div", { className: "form-actions" },
      button("Assign", { kind: "submit", className: "button", disabled: !ctx.actor() }),
      task.assignee ? button("Unassign", { className: "button ghost", disabled: !ctx.actor(), dataset: { unassign: "true" } }) : null,
    ),
  );
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    await submitMutation(ctx, form, {
      operation: "task.assign", task_id: task.id, if_revision: task.revision, assignee: assignee.value,
    }, { assignee: assignee.value }, `Task assigned to ${assignee.value}.`);
  });
  form.querySelector("[data-unassign]")?.addEventListener("click", async () => {
    await submitMutation(ctx, form, {
      operation: "task.unassign", task_id: task.id, if_revision: task.revision,
    }, { assignee: null }, "Task unassigned.");
  });
  return el("details", { className: "editor" }, el("summary", {}, el("span", { className: "field-hint", text: "Change assignment" })), form);
}
