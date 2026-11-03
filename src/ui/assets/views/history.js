import { avatar, button, el, emptyState, jsonBlock, pageHeader, timeElement } from "../dom.js";
import { eventsFrom, userFrom } from "../model.js";
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
  const root = el("div", {}, pageHeader(null, "History", "Every mutation, immutable, with the revision it produced."));
  root.append(historyFilters(ctx, actionFilter, taskFilter, limit));
  if (!events.length) {
    root.append(emptyState("No events on this page", actionFilter || taskFilter ? "Change the filters or page to see other events." : "Project mutations will append immutable changelog events."));
    return root;
  }
  root.append(el("div", { className: "event-table" }, el("ol", { className: "event-list" }, events.map((event) => {
    const actorId = event.actor?.id;
    const user = actorId ? { ...(userFrom(ctx.snapshot, actorId) || {}), name: event.actor?.name || actorId } : null;
    const detail = changeSummary(event.changes);
    return el("li", { className: "event" },
      timeElement(event.timestamp),
      el("span", { className: "event-actor" }, avatar(user, "xs"), el("span", { text: user?.name || "Unknown actor" })),
      el("span", { className: "event-copy" },
        el("span", { className: "mono", text: event.action || "unknown.action" }),
        event.task_id ? [el("span", { className: "sep", text: " · " }), el("a", { className: "mono", href: taskHref(ctx.prefix, event.task_id), text: event.task_id })] : null,
        detail ? [el("span", { className: "sep", text: " · " }), detail] : null,
      ),
      el("span", { className: "event-rev", text: event.task_revision !== undefined && event.task_revision !== null ? `rev ${event.task_revision}` : "" }),
      event.changes && Object.keys(event.changes).length ? el("details", {},
        el("summary", { text: "Recorded changes" }),
        jsonBlock(event.changes),
      ) : null,
    );
  }))));
  root.append(el("div", { className: "results-meta" },
    el("span", { text: `Events ${offset + 1}–${offset + events.length}` }),
    historyPagination(ctx, offset, limit, eventsFrom(data).length, actionFilter, taskFilter),
  ));
  return root;
}

// One-line human summary of before/after changes; the full payload stays in the details disclosure.
function changeSummary(changes) {
  if (!changes || typeof changes !== "object") return "";
  const parts = [];
  for (const [field, value] of Object.entries(changes).slice(0, 2)) {
    if (value && typeof value === "object" && ("from" in value || "to" in value)) {
      parts.push(`${field}: ${short(value.from)} → ${short(value.to)}`);
    } else {
      parts.push(field);
    }
  }
  const extra = Object.keys(changes).length - parts.length;
  return parts.join(", ") + (extra > 0 ? ` +${extra}` : "");
}

function short(value) {
  if (value === null || value === undefined) return "none";
  const text = typeof value === "object" ? JSON.stringify(value) : String(value);
  return text.length > 40 ? `${text.slice(0, 40)}…` : text;
}

function historyFilters(ctx, action, task, limit) {
  const actionInput = el("input", { type: "search", className: "search-input", value: action, placeholder: "Action contains… (task.transition)", "aria-label": "Action contains" });
  const taskInput = el("input", { type: "search", className: "compact-input", value: task, placeholder: `${ctx.prefix}-12`, "aria-label": "Task ID contains" });
  const form = el("form", { className: "filter-bar", role: "search" },
    actionInput, taskInput,
    button("Filter", { kind: "submit", className: "button" }),
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
  if (offset > 0) nav.append(el("a", { href: href(Math.max(0, offset - limit)), text: "← Newer" }));
  if (received === limit) nav.append(el("a", { href: href(offset + limit), text: "Older →" }));
  return nav;
}
