import { badge, button, el, emptyState, jsonBlock, labeledField, pageHeader, timeElement, toneFor } from "../dom.js";
import { eventsFrom } from "../model.js";
import { projectHref, taskHref } from "../router.js";

export async function renderHistory(ctx) {
  const offset = Math.max(0, Number.parseInt(ctx.route.query.get("offset") || "0", 10) || 0);
  const limit = Math.min(200, Math.max(1, Number.parseInt(ctx.route.query.get("limit") || "100", 10) || 100));
  const actionFilter = ctx.route.query.get("action") || "";
  const taskFilter = ctx.route.query.get("task") || "";
  const data = await ctx.api.events(ctx.prefix, offset, limit, { signal: ctx.signal });
  const events = eventsFrom(data).filter((event) =>
    (!actionFilter || String(event.action || "").toLowerCase().includes(actionFilter.toLowerCase()))
    && (!taskFilter || String(event.task_id || "").toLowerCase().includes(taskFilter.toLowerCase())),
  );
  const root = el("div", {}, pageHeader(
    ctx.prefix,
    "Immutable history",
    "Each entry is a changelog event read from disk. History is never edited from the Web UI.",
  ));
  root.append(historyFilters(ctx, actionFilter, taskFilter, limit));
  if (!events.length) {
    root.append(emptyState("No events on this page", actionFilter || taskFilter ? "Change the filters or page to see other events." : "Project mutations will append immutable changelog events."));
    return root;
  }
  root.append(el("div", { className: "results-meta" },
    el("span", { text: `Events ${offset + 1}–${offset + events.length}` }),
    historyPagination(ctx, offset, limit, eventsFrom(data).length, actionFilter, taskFilter),
  ));
  root.append(el("ol", { className: "event-list" }, events.map((event) => el("li", { className: "card event" },
    timeElement(event.timestamp, "long"),
    el("div", {},
      el("div", { className: "event-action", text: event.action || "unknown.action" }),
      el("div", { className: "event-copy" },
        event.actor?.name || event.actor?.id || "Unknown actor",
        event.task_id ? " · " : null,
        event.task_id ? el("a", { href: taskHref(ctx.prefix, event.task_id), text: event.task_id }) : null,
      ),
    ),
    event.task_revision !== undefined && event.task_revision !== null ? badge(`r${event.task_revision}`, toneFor(event.action)) : null,
    event.changes && Object.keys(event.changes).length ? el("details", {},
      el("summary", { text: "View recorded changes" }),
      jsonBlock(event.changes),
    ) : null,
  ))));
  return root;
}

function historyFilters(ctx, action, task, limit) {
  const actionInput = el("input", { type: "search", value: action, placeholder: "resource.updated" });
  const taskInput = el("input", { type: "search", value: task, placeholder: `${ctx.prefix}-12` });
  const form = el("form", { className: "card filter-bar", role: "search" },
    el("div", { className: "filter-grid" },
      labeledField("Action contains", actionInput),
      labeledField("Task ID contains", taskInput),
      button("Apply filters", { kind: "submit", className: "button primary" }),
    ),
  );
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const query = new URLSearchParams({ limit: String(limit) });
    if (actionInput.value.trim()) query.set("action", actionInput.value.trim());
    if (taskInput.value.trim()) query.set("task", taskInput.value.trim());
    window.location.hash = `${projectHref(ctx.prefix, "history").slice(1)}?${query}`;
  });
  return form;
}

function historyPagination(ctx, offset, limit, received, action, task) {
  const nav = el("nav", { className: "pagination", "aria-label": "History pages" });
  const href = (nextOffset) => {
    const query = new URLSearchParams({ offset: String(nextOffset), limit: String(limit) });
    if (action) query.set("action", action);
    if (task) query.set("task", task);
    return `${projectHref(ctx.prefix, "history")}?${query}`;
  };
  if (offset > 0) nav.append(el("a", { className: "button", href: href(Math.max(0, offset - limit)), text: "Previous" }));
  if (received === limit) nav.append(el("a", { className: "button", href: href(offset + limit), text: "Next" }));
  return nav;
}
