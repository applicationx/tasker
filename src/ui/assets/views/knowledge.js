import {
  badge, button, el, emptyState, labeledField, pageHeader, revisionBadge, sectionHeading, segmented,
  selectControl, tagList, textBlock, timeElement, tint,
} from "../dom.js";
import {
  decisionsFrom, normalizeDecision, normalizeProjectBrief, normalizeResource, resourcesFrom,
  reverseReferences, sourceType,
} from "../model.js";
import { decisionHref, projectHref, resourceHref, taskHref } from "../router.js";
import {
  dirtyForm, mutationUnavailable, parseTags, recordMetadata, relationshipList, submitMutation,
  tagsInputValue,
} from "./shared.js";

export function renderKnowledge(ctx) {
  const tab = ctx.route.query.get("kind") === "decisions" ? "decisions" : "resources";
  const resources = resourcesFrom(ctx.snapshot);
  const decisions = decisionsFrom(ctx.snapshot);
  const root = el("div", {},
    pageHeader(null, "Knowledge", "Short records. Value is in what references them.", [
      el("a", { className: "button ghost", href: projectHref(ctx.prefix, "knowledge/project-brief"), text: "Project brief" }),
      segmented("Knowledge kind", [
        { value: "resources", label: `Resources · ${resources.length}` },
        { value: "decisions", label: `Decisions · ${decisions.length}` },
      ], tab, (value) => ctx.app.router.navigate(`${projectHref(ctx.prefix, "knowledge").slice(1)}${value === "decisions" ? "?kind=decisions" : ""}`, true)),
    ]),
  );

  if (tab === "resources") {
    if (!resources.length) root.append(emptyState("No resources", "Create managed or referenced resources with the Tasker CLI."));
    else root.append(el("ul", { className: "knowledge-list" }, resources.map((resource) => knowledgeCard(ctx, {
      href: resourceHref(ctx.prefix, resource.id), record: resource, color: "var(--cool)",
      summary: [resource.kind, sourceType(resource), resource.source_path, resource.include_in_project_brief ? "included in project brief" : null],
    }))));
  } else if (!decisions.length) {
    root.append(emptyState("No decisions", "Proposed and settled decisions created with the Tasker CLI will appear here."));
  } else {
    root.append(el("ul", { className: "knowledge-list" }, decisions.map((decision) => knowledgeCard(ctx, {
      href: decisionHref(ctx.prefix, decision.id), record: decision, color: "var(--ok)",
      projectWide: decision.scope === "project",
      summary: [decision.decided_by ? `decided by ${decision.decided_by}` : null, decision.superseded_by ? `superseded by ${decision.superseded_by}` : null],
    }))));
  }
  return root;
}

export async function renderResource(ctx) {
  const data = await ctx.api.resource(ctx.prefix, ctx.route.entityId, { signal: ctx.signal });
  const resource = normalizeResource(data);
  const type = sourceType(resource);
  const editable = type === "managed" && resource.kind === "research" && resource.status !== "archived";
  const references = reverseReferences(ctx.snapshot, resource.id);
  const root = el("div", {}, pageHeader(
    resource.id,
    resource.title || resource.id,
    `${resource.kind || "resource"} · ${resource.status || "unknown"} · ${type}`,
    [revisionBadge(resource.revision), el("a", { className: "button", href: projectHref(ctx.prefix, "knowledge"), text: "All knowledge" })],
  ));

  if (type === "referenced") {
    root.append(el("div", { className: "notice warning source-banner", role: "status" },
      el("span", { "aria-hidden": "true", text: "↗" }),
      el("div", {},
        el("strong", { text: "Referenced file — read-only" }),
        el("span", { text: `Tasker reads ${resource.source_path || "the configured source"}. This Web UI never edits the referenced file or changes its path.` }),
      ),
    ));
  } else if (!editable) {
    root.append(el("div", { className: "notice", role: "status", text: resource.status === "archived"
      ? "Archived resources are read-only in the Web UI. Tasker does not hard-delete knowledge records."
      : "Only existing managed research resources are editable in this Web UI. Use the Tasker CLI for other resource kinds." }));
  }

  const content = editable ? researchEditor(ctx, resource) : el("section", { className: "card card-body" },
    el("h2", { text: "Content" }),
    textBlock(resource.content || ""),
  );
  const rail = el("aside", { className: "card metadata-rail", "aria-label": "Resource metadata" }, recordMetadata(resource, { prepend: [
    ["Kind", resource.kind || "—"], ["Status", badge(resource.status || "unknown", resource.status === "active" ? "success" : 1)],
    ["Source", type], ["Path", resource.source_path || "Managed by Tasker"],
    ["Brief", resource.include_in_project_brief ? "Included" : "Not included"],
    ["Tags", tagList(resource.tags) || "—"],
  ] }));
  root.append(el("div", { className: "detail-layout" }, el("div", { className: "detail-main" }, content), rail));

  root.append(sectionHeading("Used by tasks", `${references.length} reverse ${references.length === 1 ? "reference" : "references"}`));
  root.append(references.length ? el("section", { className: "card card-body" }, relationshipList(ctx, references)) : emptyState("No task references", "No task currently links this resource through a context reference."));
  return root;
}

export async function renderDecision(ctx) {
  const data = await ctx.api.decision(ctx.prefix, ctx.route.entityId, { signal: ctx.signal });
  const decision = normalizeDecision(data);
  const references = reverseReferences(ctx.snapshot, decision.id);
  const root = el("div", {}, pageHeader(
    decision.id,
    decision.title || decision.id,
    `${decision.status || "decision"} · ${decision.scope || "unknown scope"}`,
    [revisionBadge(decision.revision), el("a", { className: "button", href: projectHref(ctx.prefix, "knowledge"), text: "All knowledge" })],
  ));
  root.append(el("div", { className: "notice", role: "status", text: "Decisions and lifecycle actions are read-only in the Web UI. Use the Tasker CLI to propose, accept, reject, edit, or supersede decisions." }));
  const railEntries = [
    ["Status", badge(decision.status || "unknown", decision.status === "accepted" ? "success" : decision.status === "rejected" ? "danger" : 1)],
    ["Scope", decision.scope || "—"],
    ["Tags", tagList(decision.tags) || "—"],
  ];
  if (decision.decided_at) railEntries.push(["Decided", timeElement(decision.decided_at)]);
  if (decision.decided_by) railEntries.push(["Decided by", decision.decided_by]);
  if (decision.supersedes?.length) railEntries.push(["Supersedes", decision.supersedes.join(", ")]);
  if (decision.superseded_by) railEntries.push(["Superseded by", decision.superseded_by]);
  root.append(el("div", { className: "detail-layout" },
    el("section", { className: "card card-body detail-main" }, el("h2", { text: "Decision record" }), textBlock(decision.content || "")),
    el("aside", { className: "card metadata-rail", "aria-label": "Decision metadata" }, recordMetadata(decision, { prepend: railEntries })),
  ));
  root.append(sectionHeading("Used by tasks", `${references.length} reverse ${references.length === 1 ? "reference" : "references"}`));
  root.append(references.length ? el("section", { className: "card card-body" }, relationshipList(ctx, references)) : emptyState("No task references", "No task currently links this decision through a context reference."));
  return root;
}

export async function renderProjectBrief(ctx) {
  const data = await ctx.api.projectBrief(ctx.prefix, { signal: ctx.signal });
  const brief = normalizeProjectBrief(data);
  const content = brief.content || brief.body || "";
  const root = el("div", {}, pageHeader(
    ctx.prefix,
    "Project brief",
    "PROJECT.md is read-only in the Web UI so manually curated project intent remains explicit.",
    [el("a", { className: "button", href: projectHref(ctx.prefix, "knowledge"), text: "All knowledge" })],
  ));
  root.append(el("div", { className: "notice", role: "status", text: "Edit PROJECT.md with your local editor or the Tasker CLI. The complete text is displayed without Markdown execution." }));
  root.append(content ? textBlock(content) : emptyState("Project brief is empty", "Initialize or edit PROJECT.md through the Tasker CLI, then refresh."));
  return root;
}

function researchEditor(ctx, resource) {
  const title = el("input", { type: "text", value: resource.title || "", required: true });
  const status = selectControl([{ value: "draft", label: "Draft" }, { value: "active", label: "Active" }], resource.status);
  const tags = el("input", { type: "text", value: tagsInputValue(resource.tags), placeholder: "research, api" });
  const include = el("input", { type: "checkbox", checked: resource.include_in_project_brief });
  const body = el("textarea", { className: "code-editor", text: resource.content || "", maxlength: "1048576", spellcheck: "true" });
  const form = el("form", { className: "card form-card" },
    el("h2", { text: "Managed research editor" }),
    el("p", { className: "panel-intro", text: "All fields save together, explicitly. There is no autosave, preview execution, or background write." }),
    mutationUnavailable(ctx),
    el("div", { className: "form-grid" },
      labeledField("Title", title),
      labeledField("Status", status),
      labeledField("Tags", tags, { full: true, hint: "Comma-separated; normalized by Tasker." }),
      el("label", { className: "checkbox-field full" }, include, el("span", { text: "Include active research in the project brief" })),
      labeledField("Markdown body", body, { full: true, hint: "Displayed and edited as plain text. Maximum 1 MiB." }),
    ),
    el("div", { className: "form-actions" },
      button("Save research", { kind: "submit", className: "button primary", disabled: !ctx.actor() }),
      el("span", { className: "form-status", dataset: { formStatus: "true" } }),
    ),
  );
  dirtyForm(form, ctx);
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const fields = {
      title: title.value.trim(), status: status.value, tags: parseTags(tags.value),
      include_in_project_brief: include.checked, content: body.value,
    };
    if (!fields.title) return title.focus();
    await submitMutation(ctx, form, {
      operation: "research.update", resource_id: resource.id, if_revision: resource.revision, patch: fields,
    }, fields, "Managed research saved.");
  });
  return form;
}

function knowledgeCard(ctx, { href, record, color, summary, projectWide = false }) {
  const references = reverseReferences(ctx.snapshot, record.id);
  return el("li", {}, tint(el("article", { className: "knowledge-card" },
    el("div", { className: "knowledge-head" },
      el("span", { className: "identifier", text: record.id }),
      el("a", { href, text: record.title || record.id }),
      badge(record.status || "unknown", ["active", "accepted"].includes(record.status) ? "success" : record.status === "rejected" ? "danger" : 1),
      projectWide ? el("span", { className: "pill accent", text: "project-wide" }) : null,
    ),
    el("p", { className: "knowledge-summary", text: summary.filter(Boolean).join(" · ") || "—" }),
    tagList(record.tags),
    el("div", { className: "referenced-by" },
      el("span", { text: references.length ? "Referenced by" : "Not referenced by any task" }),
      references.map((reference) => el("a", { className: "ref-chip", href: taskHref(ctx.prefix, reference.task_id) },
        el("span", { className: "mono", text: reference.task_id }), reference.role)),
    ),
  ), color));
}
