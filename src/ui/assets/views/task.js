import {
  badge, button, el, emptyState, id, labeledField, pageHeader, revisionBadge, sectionHeading,
  selectControl, setupTabs, tagList, timeElement,
} from "../dom.js";
import {
  assigneeName, normalizeTaskDetail, stateEntries, stateLabel, usersFrom, workflowFrom,
} from "../model.js";
import { decisionHref, projectHref, resourceHref, taskHref } from "../router.js";
import {
  dirtyForm, metadataList, mutationUnavailable, parseTags, statusBadge, submitMutation,
  tagsInputValue,
} from "./shared.js";

export async function renderTask(ctx) {
  const value = await ctx.api.task(ctx.prefix, ctx.route.entityId, { signal: ctx.signal });
  const { task, status } = normalizeTaskDetail(value, ctx.snapshot, ctx.route.entityId);
  if (!task) throw { code: "task_not_found", message: `${ctx.route.entityId} was not found.` };

  const users = usersFrom(ctx.snapshot);
  const root = el("div", {},
    pageHeader(
      task.id,
      task.header,
      task.description || "No task description.",
      [revisionBadge(task.revision), el("a", { className: "button", href: `${taskHref(ctx.prefix, task.id)}/brief`, text: "Open agent brief" })],
    ),
  );

  const tabsId = id("task-tabs");
  const documentTab = el("button", { className: "tab", type: "button", role: "tab", id: `${tabsId}-document-tab`, "aria-controls": `${tabsId}-document`, text: "Document" });
  const workingTab = el("button", { className: "tab", type: "button", role: "tab", id: `${tabsId}-working-tab`, "aria-controls": `${tabsId}-working`, text: "Working" });
  const documentPanel = el("section", { className: "tab-panel", role: "tabpanel", id: `${tabsId}-document`, "aria-labelledby": documentTab.id });
  const workingPanel = el("section", { className: "tab-panel", role: "tabpanel", id: `${tabsId}-working`, "aria-labelledby": workingTab.id });

  documentPanel.append(contentEditor(ctx, task), referencesPanel(ctx, task));
  workingPanel.append(
    statePanel(ctx, task, status),
    assignmentPanel(ctx, task, users),
    acceptancePanel(ctx, task),
    contextPanel(ctx, task),
  );

  const tabs = el("div", { className: "tabs", role: "tablist", "aria-label": "Task detail sections" }, documentTab, workingTab);
  const rail = el("aside", { className: "card metadata-rail", "aria-label": "Task metadata" },
    metadataList([
      ["State", statusBadge(ctx.snapshot, task.state)],
      ["Assignee", assigneeName(ctx.snapshot, task.assignee)],
      ["Revision", revisionBadge(task.revision)],
      ["Created", task.created_at ? timeElement(task.created_at) : "—"],
      ["Created by", task.created_by || "—"],
      ["Updated", task.updated_at ? timeElement(task.updated_at) : "—"],
      ["Updated by", task.updated_by || "—"],
      ["Tags", tagList(task.tags) || "—"],
    ]),
  );
  root.append(el("div", { className: "detail-layout" },
    el("div", { className: "detail-main" }, tabs, documentPanel, workingPanel),
    rail,
  ));
  setupTabs(root, ctx.route.query.get("tab") === "working" ? 1 : 0);
  return root;
}

function contentEditor(ctx, task) {
  const header = el("input", { type: "text", value: task.header, required: true, maxlength: "500" });
  const description = el("textarea", { text: task.description || "", rows: "8" });
  const tags = el("input", { type: "text", value: tagsInputValue(task.tags), placeholder: "frontend, urgent" });
  const form = el("form", { className: "card form-card" },
    el("h2", { text: "Task content" }),
    el("p", { className: "panel-intro", text: "Edit the task document, then save all three fields together. There is no autosave." }),
    mutationUnavailable(ctx),
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
  return form;
}

function statePanel(ctx, task, status) {
  const workflow = workflowFrom(ctx.snapshot);
  const allowed = Array.isArray(status.allowed_transitions)
    ? status.allowed_transitions
    : workflow.transitions?.[task.state] || [];
  const target = selectControl(allowed.map((state) => ({ value: state, label: stateLabel(ctx.snapshot, state) })), allowed[0] || "", { disabled: !allowed.length });
  const form = el("form", { className: "card quick-panel" },
    el("h2", { text: "State" }),
    el("p", { className: "panel-intro", text: "Only configured transitions are available. Tasker revalidates dependencies and acceptance criteria on save." }),
    mutationUnavailable(ctx),
    el("div", { className: "list-meta" }, el("span", { text: "Current" }), statusBadge(ctx.snapshot, task.state)),
    allowed.length
      ? el("div", { className: "form-grid" }, labeledField("Transition to", target))
      : el("p", { className: "muted", text: "No transitions are currently allowed." }),
    allowed.length ? el("div", { className: "form-actions" },
      button("Apply transition", { kind: "submit", className: "button primary", disabled: !ctx.actor() }),
      el("span", { className: "form-status", dataset: { formStatus: "true" } }),
    ) : null,
  );
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    await submitMutation(ctx, form, {
      operation: "task.transition", task_id: task.id, if_revision: task.revision, target_state: target.value,
    }, { target_state: target.value }, `Task transitioned to ${stateLabel(ctx.snapshot, target.value)}.`);
  });
  return form;
}

function assignmentPanel(ctx, task, users) {
  const choices = users.map((user) => ({ value: user.id, label: user.name ? `${user.name} (${user.id})` : user.id }));
  if (task.assignee && !choices.some((item) => item.value === task.assignee)) choices.unshift({ value: task.assignee, label: task.assignee });
  const assignee = selectControl(choices, task.assignee || choices[0]?.value || "", { disabled: !choices.length });
  const form = el("form", { className: "card quick-panel" },
    el("h2", { text: "Assignment" }),
    el("p", { className: "panel-intro", text: "Assign only an existing project user. Creating users remains a separate CLI action." }),
    mutationUnavailable(ctx),
    choices.length ? labeledField("Project user", assignee) : el("p", { className: "muted", text: "This project has no users." }),
    el("div", { className: "form-actions" },
      choices.length ? button("Assign", { kind: "submit", className: "button primary", disabled: !ctx.actor() }) : null,
      task.assignee ? button("Unassign", { className: "button", disabled: !ctx.actor(), dataset: { unassign: "true" } }) : null,
      el("span", { className: "form-status", dataset: { formStatus: "true" } }),
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
  return form;
}

function acceptancePanel(ctx, task) {
  const criteria = Array.isArray(task.acceptance_criteria) ? task.acceptance_criteria : [];
  const panel = el("section", { className: "card quick-panel" },
    el("h2", { text: "Acceptance criteria" }),
    el("p", { className: "panel-intro", text: "Checkboxes commit one criterion at a time. Criterion text is immutable; replace it by removing and adding." }),
    mutationUnavailable(ctx),
  );
  if (!criteria.length) panel.append(el("p", { className: "muted", text: "No acceptance criteria." }));
  else {
    const list = el("ul", { className: "criteria-list" });
    for (const criterion of criteria) {
      const checkboxId = id("criterion");
      const checkbox = el("input", { id: checkboxId, type: "checkbox", checked: criterion.completed, disabled: !ctx.actor() });
      const row = el("li", {},
        checkbox,
        el("div", { className: "criterion-copy" },
          el("label", { for: checkboxId, text: criterion.text }),
          el("span", { className: "criterion-meta", text: criterion.completed_at ? `${criterion.id} · completed by ${criterion.completed_by || "unknown"}` : criterion.id }),
        ),
        button("Remove", { className: "text-button", disabled: !ctx.actor(), "aria-label": `Remove ${criterion.id}`, dataset: { removeCriterion: criterion.id } }),
      );
      checkbox.addEventListener("change", async () => {
        const desired = checkbox.checked;
        checkbox.checked = !desired;
        await submitMutation(ctx, panel, {
          operation: "task.acceptance.set", task_id: task.id, criterion_id: criterion.id,
          completed: desired, if_revision: task.revision,
        }, { criterion_id: criterion.id, completed: desired }, desired ? "Criterion completed." : "Criterion reopened.");
      });
      row.querySelector("[data-remove-criterion]").addEventListener("click", async () => {
        if (!window.confirm(`Remove ${criterion.id}? Criterion text cannot be recovered from the task file.`)) return;
        await submitMutation(ctx, panel, {
          operation: "task.acceptance.remove", task_id: task.id, criterion_id: criterion.id, if_revision: task.revision,
        }, { criterion_id: criterion.id }, "Criterion removed.");
      });
      list.append(row);
    }
    panel.append(list);
  }
  const text = el("input", { type: "text", required: true, placeholder: "A measurable completion checkpoint" });
  const addForm = el("form", {}, labeledField("New criterion", text),
    el("div", { className: "form-actions" },
      button("Add criterion", { kind: "submit", className: "button", disabled: !ctx.actor() }),
      el("span", { className: "form-status", dataset: { formStatus: "true" } }),
    ),
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

function contextPanel(ctx, task) {
  const entries = Array.isArray(task.context) ? task.context : [];
  const kind = selectControl([
    { value: "context", label: "Context" }, { value: "progress", label: "Progress" }, { value: "decision", label: "Decision" },
  ]);
  const message = el("textarea", { required: true, rows: "5", placeholder: "Add durable context for the next agent or collaborator…" });
  const form = el("form", { className: "card quick-panel" },
    el("h2", { text: "Context timeline" }),
    el("p", { className: "panel-intro", text: "Entries are append-only and cannot be edited or deleted." }),
    entries.length ? el("ol", { className: "timeline" }, entries.map((entry) => el("li", {},
      el("div", { className: "timeline-meta" }, badge(entry.kind || "context", 1), ` ${entry.id || ""} · ${entry.created_by || "unknown"} · `, timeElement(entry.created_at)),
      el("p", { text: entry.message }),
    ))) : el("p", { className: "muted", text: "No context entries yet." }),
    mutationUnavailable(ctx),
    el("div", { className: "form-grid" },
      labeledField("Entry kind", kind),
      labeledField("Message", message, { full: true }),
    ),
    el("div", { className: "form-actions" },
      button("Append context", { kind: "submit", className: "button primary", disabled: !ctx.actor() }),
      el("span", { className: "form-status", dataset: { formStatus: "true" } }),
    ),
  );
  dirtyForm(form, ctx);
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    await submitMutation(ctx, form, {
      operation: "task.context.append", task_id: task.id, kind: kind.value,
      message: message.value.trim(), if_revision: task.revision,
    }, { kind: kind.value, message: message.value }, "Context appended.");
  });
  return form;
}

function referencesPanel(ctx, task) {
  const dependencies = Array.isArray(task.dependencies) ? task.dependencies : [];
  const relations = Array.isArray(task.relations) ? task.relations : [];
  const references = Array.isArray(task.context_refs) ? task.context_refs : [];
  return el("section", { className: "card card-body" },
    sectionHeading("Connections", "Read-only in the Web UI"),
    el("div", { className: "grid grid-3" },
      connectionGroup("Dependencies", dependencies.length ? dependencies.map((dependency) => el("li", {}, el("a", { href: taskHref(ctx.prefix, dependency), text: dependency }))) : []),
      connectionGroup("Relations", relations.length ? relations.map((relation) => el("li", {},
        el("a", { href: taskHref(ctx.prefix, relation.target), text: relation.target }),
        el("span", { className: "muted", text: relation.type || relation.kind }),
      )) : []),
      connectionGroup("Knowledge", references.length ? references.map((reference) => {
        const isDecision = reference.kind === "decision" || /-D\d+$/.test(reference.id);
        return el("li", {},
          el("a", { href: isDecision ? decisionHref(ctx.prefix, reference.id) : resourceHref(ctx.prefix, reference.id), text: reference.id }),
          el("span", { className: "muted", text: reference.role || reference.kind }),
        );
      }) : []),
    ),
  );
}

function connectionGroup(title, items) {
  return el("div", {}, el("h3", { text: title }), items.length
    ? el("ul", { className: "compact-list" }, items)
    : el("p", { className: "muted", text: "None" }));
}
