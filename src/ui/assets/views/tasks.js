import { badge, button, el, emptyState, labeledField, pageHeader, selectControl, tagList } from "../dom.js";
import { assigneeName, stateEntries, taskRecords, usersFrom } from "../model.js";
import { projectHref, taskHref } from "../router.js";
import { statusBadge } from "./shared.js";

export function renderTasks(ctx) {
  const query = ctx.route.query;
  const records = taskRecords(ctx.snapshot);
  const states = stateEntries(ctx.snapshot);
  const users = usersFrom(ctx.snapshot);
  const filters = {
    q: query.get("q") || "",
    state: query.get("state") || "",
    assignee: query.get("assignee") || "",
    tag: query.get("tag") || "",
    ready: query.get("ready") || "",
    blocked: query.get("dependency_blocked") || "",
  };
  const offset = Math.max(0, Number.parseInt(query.get("offset") || "0", 10) || 0);
  const limit = Math.min(200, Math.max(1, Number.parseInt(query.get("limit") || "50", 10) || 50));
  const filtered = records.filter((record) => matches(record, filters));
  const visible = filtered.slice(offset, offset + limit);

  const root = el("div", {}, pageHeader(ctx.prefix, "Tasks", "Search and filter the current project snapshot. Results refresh from the authoritative filesystem."));
  const form = filterForm(ctx, filters, states, users, limit);
  root.append(form);

  if (!records.length) {
    root.append(emptyState("No tasks yet", "Create tasks with the Tasker CLI, then refresh this view."));
    return root;
  }
  if (!visible.length) {
    root.append(emptyState("No tasks match these filters", "Change or clear the filters to see other project tasks.",
      button("Clear filters", { className: "button", onclick: () => { window.location.hash = projectHref(ctx.prefix, "tasks").slice(1); } }),
    ));
    return root;
  }

  root.append(el("div", { className: "results-meta" },
    el("span", { text: `Showing ${offset + 1}–${Math.min(offset + visible.length, filtered.length)} of ${filtered.length} tasks` }),
    pagination(ctx, filters, offset, limit, filtered.length),
  ));
  root.append(taskTable(ctx, visible), mobileCards(ctx, visible));
  return root;
}

function filterForm(ctx, filters, states, users, limit) {
  const q = el("input", { type: "search", name: "q", value: filters.q, placeholder: "ID, header, description, or tag" });
  const state = selectControl([{ value: "", label: "All states" }, ...states.map(([key, config]) => ({ value: key, label: config.label || key }))], filters.state, { name: "state" });
  const assignee = selectControl([
    { value: "", label: "Any assignee" }, { value: "__unassigned", label: "Unassigned" },
    ...users.map((user) => ({ value: user.id, label: user.name ? `${user.name} (${user.id})` : user.id })),
  ], filters.assignee, { name: "assignee" });
  const tag = el("input", { type: "text", name: "tag", value: filters.tag, placeholder: "Exact tag" });
  const ready = selectControl([{ value: "", label: "Any readiness" }, { value: "true", label: "Claimable" }, { value: "false", label: "Not claimable" }], filters.ready, { name: "ready" });
  const blocked = selectControl([{ value: "", label: "Any dependency state" }, { value: "true", label: "Dependency blocked" }, { value: "false", label: "Not dependency blocked" }], filters.blocked, { name: "dependency_blocked" });
  const form = el("form", { className: "card filter-bar", role: "search" },
    el("div", { className: "filter-grid" },
      el("div", { className: "search-field" }, labeledField("Search tasks", q)),
      labeledField("State", state),
      labeledField("Assignee", assignee),
      labeledField("Tag", tag),
      labeledField("Readiness", ready),
      labeledField("Dependencies", blocked),
      button("Apply filters", { kind: "submit", className: "button primary" }),
    ),
  );
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const data = new FormData(form);
    const query = new URLSearchParams();
    for (const key of ["q", "state", "assignee", "tag", "ready", "dependency_blocked"]) {
      const value = String(data.get(key) || "").trim();
      if (value) query.set(key, value);
    }
    query.set("limit", String(limit));
    window.location.hash = `${projectHref(ctx.prefix, "tasks").slice(1)}?${query}`;
  });
  return form;
}

function matches({ task, status = {} }, filters) {
  const haystack = [task.id, task.header, task.description, ...(task.tags || [])].join(" ").toLowerCase();
  const terms = filters.q.toLowerCase().split(/\s+/).filter(Boolean);
  if (!terms.every((term) => haystack.includes(term))) return false;
  if (filters.state && task.state !== filters.state) return false;
  if (filters.assignee === "__unassigned" && task.assignee) return false;
  if (filters.assignee && filters.assignee !== "__unassigned" && task.assignee !== filters.assignee) return false;
  if (filters.tag && !(task.tags || []).includes(filters.tag.toLowerCase())) return false;
  if (filters.ready && Boolean(status.claimable) !== (filters.ready === "true")) return false;
  if (filters.blocked && Boolean(status.dependency_blocked) !== (filters.blocked === "true")) return false;
  return true;
}

function taskTable(ctx, records) {
  return el("div", { className: "table-wrap task-table" },
    el("table", {},
      el("caption", { className: "sr-only", text: "Filtered project tasks" }),
      el("thead", {}, el("tr", {}, ["Task", "State", "Assignee", "Tags", "Readiness"].map((title) => el("th", { scope: "col", text: title })))),
      el("tbody", {}, records.map(({ task, status }) => el("tr", {},
        el("td", {}, el("a", { href: taskHref(ctx.prefix, task.id), text: task.header }), el("div", { className: "identifier", text: task.id })),
        el("td", {}, statusBadge(ctx.snapshot, task.state)),
        el("td", { text: assigneeName(ctx.snapshot, task.assignee) }),
        el("td", {}, tagList(task.tags) || el("span", { className: "muted", text: "—" })),
        el("td", {}, status.dependency_blocked ? badge("Blocked", "blocked") : status.claimable ? badge("Claimable", "success") : el("span", { className: "muted", text: "—" })),
      ))),
    ),
  );
}

function mobileCards(ctx, records) {
  return el("div", { className: "task-cards-mobile", "aria-label": "Filtered project tasks" }, records.map(({ task, status }) => el("a", { className: "card card-link task-card", href: taskHref(ctx.prefix, task.id) },
    el("span", { className: "identifier", text: task.id }),
    el("h3", { text: task.header }),
    el("div", { className: "list-meta" }, statusBadge(ctx.snapshot, task.state), el("span", { text: assigneeName(ctx.snapshot, task.assignee) })),
    tagList(task.tags),
    status.dependency_blocked ? el("div", { className: "notice error", text: "Dependency blocked" }) : null,
  )));
}

function pagination(ctx, filters, offset, limit, total) {
  const group = el("nav", { className: "pagination", "aria-label": "Task results pages" });
  if (offset > 0) group.append(el("a", { className: "button", href: pageHref(ctx, filters, Math.max(0, offset - limit), limit), text: "Previous" }));
  if (offset + limit < total) group.append(el("a", { className: "button", href: pageHref(ctx, filters, offset + limit, limit), text: "Next" }));
  return group;
}

function pageHref(ctx, filters, offset, limit) {
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(filters)) {
    const apiKey = key === "blocked" ? "dependency_blocked" : key;
    if (value) query.set(apiKey, value);
  }
  query.set("offset", String(offset));
  query.set("limit", String(limit));
  return `${projectHref(ctx.prefix, "tasks")}?${query}`;
}
