import { avatar, button, clear, el, emptyState, pageHeader } from "../dom.js";
import { acceptanceLabel, isTerminal, taskRecords, userFrom } from "../model.js";
import { projectHref, taskHref } from "../router.js";
import { statusBadge, typePill } from "./shared.js";

const FILTERS = [
  { key: "unassigned", label: "Unassigned", test: ({ task }) => !task.assignee },
  { key: "ready", label: "Dependency-ready", test: ({ task, status }, ctx) => !status.dependency_blocked && !isTerminal(ctx.snapshot, task.state) },
  { key: "blocked", label: "Dependency-blocked", test: ({ status }) => Boolean(status.dependency_blocked) },
  { key: "claimable", label: "Claimable", test: ({ status }) => Boolean(status.claimable) },
  { key: "agents", label: "Held by agents", test: ({ task }, ctx) => userFrom(ctx.snapshot, task.assignee)?.kind === "agent" },
  { key: "open", label: "Open only", test: ({ task }, ctx) => !isTerminal(ctx.snapshot, task.state) },
];

export function renderTasks(ctx) {
  const query = ctx.route.query;
  const records = taskRecords(ctx.snapshot);
  const state = {
    q: query.get("q") || "",
    filter: FILTERS.some((item) => item.key === query.get("filter")) ? query.get("filter") : "",
    offset: Math.max(0, Number.parseInt(query.get("offset") || "0", 10) || 0),
    limit: Math.min(200, Math.max(1, Number.parseInt(query.get("limit") || "50", 10) || 50)),
    // Exact-match filters remain addressable by URL for links and scripts.
    exact: { state: query.get("state") || "", assignee: query.get("assignee") || "", tag: query.get("tag") || "" },
  };

  const root = el("div", {}, pageHeader(null, "Tasks", "Search covers headers, descriptions, tags, criteria, context and references."));
  if (!records.length) {
    root.append(emptyState("No tasks yet", "Create tasks with the Tasker CLI, then refresh this view."));
    return root;
  }

  const search = el("input", {
    type: "search", className: "search-input", value: state.q, placeholder: "Search tasks…", "aria-label": "Search tasks",
  });
  const chips = FILTERS.map((filter) => el("button", {
    type: "button", className: "filter-chip", text: filter.label, "aria-pressed": String(state.filter === filter.key),
    onclick: () => {
      state.filter = state.filter === filter.key ? "" : filter.key;
      state.offset = 0;
      update();
    },
  }));
  const results = el("div", {});
  root.append(el("div", { className: "filter-bar", role: "search" }, search, chips), results);

  const update = () => {
    chips.forEach((chip, index) => chip.setAttribute("aria-pressed", String(state.filter === FILTERS[index].key)));
    const filtered = records.filter((record) => matches(ctx, record, state));
    if (state.offset >= filtered.length) state.offset = 0;
    clear(results, table(ctx, records.length, filtered, state, () => update()));
    window.history.replaceState(null, "", hrefFor(ctx, state));
  };
  search.addEventListener("input", () => {
    state.q = search.value;
    state.offset = 0;
    update();
  });
  update();
  return root;
}

function matches(ctx, record, state) {
  const { task } = record;
  const haystack = [
    task.id, task.type, task.header, task.description, ...(task.tags || []),
    ...(task.acceptance_criteria || []).map((item) => item.text),
    ...(task.context || []).map((item) => item.message),
    ...(task.context_refs || []).map((item) => item.id),
  ].join(" ").toLowerCase();
  const terms = state.q.toLowerCase().split(/\s+/).filter(Boolean);
  if (!terms.every((term) => haystack.includes(term))) return false;
  const filter = FILTERS.find((item) => item.key === state.filter);
  if (filter && !filter.test(record, ctx)) return false;
  const { exact } = state;
  if (exact.state && task.state !== exact.state) return false;
  if (exact.assignee === "__unassigned" && task.assignee) return false;
  if (exact.assignee && exact.assignee !== "__unassigned" && task.assignee !== exact.assignee) return false;
  if (exact.tag && !(task.tags || []).includes(exact.tag.toLowerCase())) return false;
  return true;
}

function table(ctx, total, filtered, state, rerender) {
  const visible = filtered.slice(state.offset, state.offset + state.limit);
  const wrap = el("div", { className: "data-table" },
    el("div", { className: "data-head", "aria-hidden": "true" },
      ["ID", "Task", "State", "Assignee", "Criteria", "Dependencies"].map((title) => el("span", { text: title })),
    ),
  );
  if (!visible.length) {
    wrap.append(el("div", { className: "data-empty" },
      el("p", { text: "No tasks match this search." }),
      button("Clear search and filters", { className: "button ghost", onclick: () => {
        window.location.hash = projectHref(ctx.prefix, "tasks").slice(1);
      } }),
    ));
    return wrap;
  }
  wrap.append(el("ul", { className: "data-body", "aria-label": "Tasks" }, visible.map((record) => el("li", {}, row(ctx, record)))));
  const pages = Math.ceil(filtered.length / state.limit);
  const current = Math.floor(state.offset / state.limit);
  const pager = el("nav", { className: "pagination", "aria-label": "Task result pages" });
  if (pages > 1) {
    for (let page = 0; page < pages; page += 1) {
      if (page === current) pager.append(el("span", { "aria-current": "page", text: page + 1 }));
      else pager.append(el("a", { href: hrefFor(ctx, { ...state, offset: page * state.limit }), text: page + 1, onclick: (event) => {
        event.preventDefault();
        state.offset = page * state.limit;
        rerender();
      } }));
    }
  }
  wrap.append(el("div", { className: "data-foot" },
    el("span", { text: `Showing ${state.offset + 1}–${state.offset + visible.length} of ${filtered.length}${filtered.length !== total ? ` (${total} total)` : ""}` }),
    pager,
  ));
  return wrap;
}

function row(ctx, { task, status = {} }) {
  const user = userFrom(ctx.snapshot, task.assignee);
  const dependencies = task.dependencies || [];
  const unresolved = status.unresolved_dependencies || [];
  const depLabel = !dependencies.length ? "—" : status.dependency_blocked ? `blocked · ${unresolved.join(", ")}` : "ready";
  const depClass = !dependencies.length ? "" : status.dependency_blocked ? "bad" : "ok";
  return el("a", { className: "data-row", href: taskHref(ctx.prefix, task.id) },
    el("span", { className: "identifier", text: task.id }),
    el("span", { className: "task-title" },
      typePill(task),
      el("span", { text: task.header }),
      (task.tags || []).slice(0, 2).map((tag) => el("span", { className: "tag", text: tag })),
    ),
    statusBadge(ctx.snapshot, task.state),
    el("span", { className: `assignee ${user ? "" : "none"}`.trim() }, avatar(user, "sm"), el("span", { text: user ? user.name || user.id : "Unassigned" })),
    el("span", { className: "ac", text: acceptanceLabel(task) }),
    el("span", { className: `deps ${depClass}`.trim(), text: depLabel }),
  );
}

function hrefFor(ctx, state) {
  const query = new URLSearchParams();
  if (state.q.trim()) query.set("q", state.q.trim());
  if (state.filter) query.set("filter", state.filter);
  for (const [key, value] of Object.entries(state.exact)) if (value) query.set(key, value);
  if (state.offset) query.set("offset", String(state.offset));
  if (state.limit !== 50) query.set("limit", String(state.limit));
  const suffix = query.toString();
  return `${projectHref(ctx.prefix, "tasks")}${suffix ? `?${suffix}` : ""}`;
}
