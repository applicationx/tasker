import { avatar, el, emptyState, pageHeader, plural, relativeTime, segmented, tint } from "../dom.js";
import {
  acceptanceLabel, isTerminal, stateColor, stateEntries, stateLabel, taskRecords, userFrom, usersFrom,
  validationFrom, workflowFrom,
} from "../model.js";
import { projectHref, taskHref } from "../router.js";
import { validationNotice } from "./shared.js";

export function renderOverview(ctx) {
  const records = taskRecords(ctx.snapshot);
  const mode = ctx.route.query.get("view") === "people" ? "people" : "pulse";
  const root = el("div", {},
    pageHeader(null, "Overview", "Where the project stands right now, and who has what.", [
      segmented("Overview layout", [
        { value: "pulse", label: "Pulse" }, { value: "people", label: "Who's on what" },
      ], mode, (value) => ctx.app.router.navigate(`${projectHref(ctx.prefix, "overview").slice(1)}${value === "people" ? "?view=people" : ""}`, true)),
    ]),
    validationNotice(validationFrom(ctx.snapshot)),
    distributionPanel(ctx, records),
  );
  if (!records.length) {
    root.append(emptyState("No tasks yet", "Create tasks with the Tasker CLI; they appear here as soon as they land on disk."));
    return root;
  }
  root.append(mode === "people" ? peopleLanes(ctx, records) : pulse(ctx, records));
  return root;
}

function distributionPanel(ctx, records) {
  const states = stateEntries(ctx.snapshot);
  const counts = new Map(states.map(([key]) => [key, 0]));
  for (const { task } of records) counts.set(task.state, (counts.get(task.state) || 0) + 1);
  const present = [...counts].filter(([, count]) => count > 0);
  const open = records.filter(({ task }) => !isTerminal(ctx.snapshot, task.state)).length;
  const closed = records.length - open;
  return el("section", { className: "distribution-panel", "aria-labelledby": "distribution-heading" },
    el("div", { className: "distribution-head" },
      el("h2", { id: "distribution-heading", className: "eyebrow-heading", text: "Distribution" }),
      el("span", { className: "distribution-totals" }, el("strong", { text: open }), " open · ", el("strong", { text: closed }), " closed"),
    ),
    el("div", { className: "distribution-bar", "aria-hidden": "true" }, present.map(([key, count]) => {
      const bar = tint(el("span", { title: `${stateLabel(ctx.snapshot, key)} · ${count}` }), stateColor(ctx.snapshot, key));
      bar.style.setProperty("--flex", String(count));
      return bar;
    })),
    el("ul", { className: "distribution-legend" }, present.map(([key, count]) => el("li", {},
      tint(el("span", { className: "dot", "aria-hidden": "true" }), stateColor(ctx.snapshot, key)),
      stateLabel(ctx.snapshot, key),
      el("strong", { text: count }),
    ))),
  );
}

function pulse(ctx, records) {
  const claimState = workflowFrom(ctx.snapshot).claim_state;
  const inFlight = records.filter(({ task }) => task.state === claimState);
  const open = records.filter(({ task }) => !isTerminal(ctx.snapshot, task.state));
  const attention = open.filter(({ task, status }) => status.dependency_blocked && (task.assignee || task.state === claimState));
  const claimable = open.filter(({ task, status }) => status.claimable && !task.assignee);
  const blocked = open.filter(({ status }) => status.dependency_blocked).length;

  const flight = el("section", { className: "stack-tight", "aria-labelledby": "in-flight-heading" },
    el("h2", { id: "in-flight-heading", className: "eyebrow-heading", text: "In flight" }),
  );
  if (!inFlight.length) flight.append(el("p", { className: "empty-line", text: `Nothing is in ${stateLabel(ctx.snapshot, claimState)} right now.` }));
  for (const { task } of inFlight) flight.append(flightCard(ctx, task));

  const side = el("div", { className: "stack" });
  if (attention.length) {
    side.append(el("section", { className: "side-panel attention", "aria-labelledby": "attention-heading" },
      el("h2", { id: "attention-heading", text: "Needs a human" }),
      el("ul", { className: "side-list" }, attention.map(({ task, status }) => el("li", {},
        el("a", { href: taskHref(ctx.prefix, task.id) },
          el("span", { className: "side-title strong", text: task.header }),
          el("span", { className: "side-meta" }, el("span", { className: "mono", text: task.id }),
            ` · ${task.assignee || "unassigned"} is blocked on ${(status.unresolved_dependencies || []).join(", ") || "a dependency"}`),
        ),
      ))),
    ));
  }
  side.append(el("section", { className: "side-panel", "aria-labelledby": "claimable-heading" },
    el("h2", { id: "claimable-heading", text: "Claimable now" }),
    claimable.length
      ? el("ul", { className: "side-list" }, claimable.slice(0, 10).map(({ task }) => el("li", {},
        el("a", { className: "inline", href: taskHref(ctx.prefix, task.id) },
          el("span", { className: "identifier", text: task.id }),
          el("span", { className: "side-title", text: task.header }),
        ),
      )))
      : el("p", { className: "side-meta", text: "No unassigned task is claimable." }),
    el("p", { className: "side-foot", text: blocked ? `${plural(blocked, "more task")} ${blocked === 1 ? "is" : "are"} dependency-blocked.` : "Nothing is dependency-blocked." }),
  ));
  return el("div", { className: "pulse-layout" }, flight, side);
}

function flightCard(ctx, task) {
  const user = userFrom(ctx.snapshot, task.assignee);
  const criteria = task.acceptance_criteria || [];
  const done = criteria.filter((item) => item.completed).length;
  const context = task.context || [];
  const last = context.length ? String(context[context.length - 1].message || "") : "";
  return el("a", { className: "flight-card", href: taskHref(ctx.prefix, task.id) },
    el("div", { className: "flight-head" },
      avatar(user, "md"),
      el("span", { className: "flight-copy" },
        el("span", { className: "flight-title", text: task.header }),
        el("span", { className: "flight-meta", text: `${user ? user.name || user.id : "Unassigned"} · ${user?.kind || "unassigned"} · updated ${relativeTime(task.updated_at)}` }),
      ),
      el("span", { className: "identifier", text: task.id }),
    ),
    el("div", { className: "flight-foot" },
      criteria.length ? el("span", { className: "ac-dots", "aria-hidden": "true" }, criteria.map((item) => el("span", { className: item.completed ? "done" : "" }))) : null,
      el("span", { className: "flight-ac", text: criteria.length ? `${done} of ${criteria.length} criteria met` : "no criteria yet" }),
      last ? el("span", { className: "flight-note", text: `“${last.length > 92 ? `${last.slice(0, 92)}…` : last}”` }) : null,
    ),
  );
}

function peopleLanes(ctx, records) {
  const claimState = workflowFrom(ctx.snapshot).claim_state;
  const users = usersFrom(ctx.snapshot);
  const open = records.filter(({ task }) => !isTerminal(ctx.snapshot, task.state));
  const lanes = users.map((user) => ({ user, items: open.filter(({ task }) => task.assignee === user.id) }));
  const unknown = [...new Set(open.map(({ task }) => task.assignee).filter((id) => id && !users.some((user) => user.id === id)))];
  for (const id of unknown) lanes.push({ user: userFrom(ctx.snapshot, id), items: open.filter(({ task }) => task.assignee === id) });
  lanes.sort((a, b) => b.items.length - a.items.length);
  if (!lanes.length) return emptyState("No project users", "Users appear here once they are created or claim work.");
  return el("div", { className: "stack-tight" }, lanes.map(({ user, items }) => {
    const running = items.filter(({ task }) => task.state === claimState).length;
    return el("section", { className: "lane", "aria-label": user.name || user.id },
      el("div", { className: "lane-person" },
        avatar(user, "lg"),
        el("span", {},
          el("span", { className: "lane-name", text: user.name || user.id }),
          el("span", { className: "lane-kind", text: `${user.id} · ${user.kind || "unknown"}` }),
          el("span", { className: "lane-summary", text: running ? `${running} running · ${items.length} held` : `${items.length} held, none running` }),
        ),
      ),
      el("div", { className: "lane-items" }, items.length ? items.map(({ task }) => {
        const color = stateColor(ctx.snapshot, task.state);
        return tint(el("a", { className: "row-link", href: taskHref(ctx.prefix, task.id) },
          el("span", { className: "identifier", text: task.id }),
          el("span", { className: "row-title", text: task.header }),
          tint(el("span", { className: "state-label", text: stateLabel(ctx.snapshot, task.state) }), color),
          el("span", { className: "row-ac", text: acceptanceLabel(task) }),
        ), color);
      }) : el("div", { className: "empty-line", text: "Idle — nothing claimed" })),
    );
  }));
}
