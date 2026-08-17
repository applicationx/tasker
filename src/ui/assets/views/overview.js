import { badge, el, emptyState, pageHeader, plural, sectionHeading, timeElement, toneFor } from "../dom.js";
import {
  decisionsFrom, eventsFrom, projectFrom, projectName, resourcesFrom, stateConfig,
  stateEntries, stateLabel, taskRecords, validationFrom,
} from "../model.js";
import { taskHref } from "../router.js";
import { statusBadge, validationNotice } from "./shared.js";

export function renderOverview(ctx) {
  const project = projectFrom(ctx.snapshot);
  const records = taskRecords(ctx.snapshot);
  const states = stateEntries(ctx.snapshot);
  const resources = resourcesFrom(ctx.snapshot);
  const decisions = decisionsFrom(ctx.snapshot);
  const recent = eventsFrom(ctx.snapshot?.recent_events);
  const terminal = records.filter(({ task }) => stateConfig(ctx.snapshot, task.state).terminal).length;
  const open = records.length - terminal;
  const blocked = records.filter(({ status }) => status.dependency_blocked).length;
  const claimable = records.filter(({ status }) => status.claimable).length;
  const working = records.filter(({ task }) => task.assignee && !stateConfig(ctx.snapshot, task.state).terminal);
  const knownKnowledge = new Set([...resources, ...decisions].map((item) => item.id));
  const brokenContextRefs = records.flatMap(({ task }) => task.context_refs || []).filter((reference) => !knownKnowledge.has(reference.id)).length;
  const reportedBroken = project.knowledge?.broken_reference_count ?? ctx.snapshot?.broken_reference_count;
  const brokenReferences = reportedBroken ?? brokenContextRefs;

  const root = el("div", {},
    pageHeader(
      project.prefix || ctx.prefix,
      projectName(ctx.snapshot),
      "A current view of workflow, active ownership, dependencies, and project knowledge.",
    ),
    validationNotice(validationFrom(ctx.snapshot)),
    el("div", { className: "grid grid-4" },
      metric("Open work", open, plural(records.length, "task") + " total"),
      metric("Terminal", terminal, "Derived from configured workflow"),
      metric("Claimable", claimable, "Authoritative Tasker status", "claimable"),
      metric("Needs attention", blocked, "Dependency-blocked tasks", blocked ? "attention" : ""),
    ),
  );

  root.append(sectionHeading("Workflow distribution", `${states.length} configured ${states.length === 1 ? "state" : "states"}`));
  const distribution = el("div", { className: "distribution" });
  states.forEach(([key, config], index) => {
    const count = records.filter(({ task }) => task.state === key).length;
    distribution.append(el("div", { className: `card distribution-item tone-${index % 6}` },
      el("span", { className: "soft", text: config.label || key }),
      el("strong", { text: count }),
      config.terminal ? el("span", { className: "muted", text: "Terminal" }) : null,
    ));
  });
  root.append(states.length ? distribution : emptyState("No workflow states", "The project configuration does not expose any workflow states."));

  root.append(sectionHeading("Working now", plural(working.length, "assigned non-terminal task")));
  if (!working.length) {
    root.append(emptyState("No assigned work in progress", "Assigned, non-terminal tasks will appear here."));
  } else {
    root.append(el("section", { className: "card card-body" },
      el("ul", { className: "compact-list" }, working.slice(0, 12).map(({ task, status }) => el("li", {},
        el("div", { className: "list-main" },
          el("a", { href: taskHref(ctx.prefix, task.id), text: task.header }),
          el("div", { className: "list-meta" },
            el("span", { className: "identifier", text: task.id }),
            el("span", { text: task.assignee }),
            status.dependency_blocked ? el("span", { text: "Dependency blocked" }) : null,
          ),
        ),
        statusBadge(ctx.snapshot, task.state),
      ))),
    ));
  }

  root.append(sectionHeading("Knowledge", `${resources.length} resources · ${decisions.length} decisions`));
  root.append(el("div", { className: "grid grid-3" },
    knowledgeMetric("Resources", resources.length, resources.filter((item) => item.status === "active").length, "active"),
    knowledgeMetric("Decisions", decisions.length, decisions.filter((item) => item.status === "accepted").length, "accepted"),
    knowledgeMetric("Broken references", brokenReferences, null, reportedBroken === undefined ? "task context references" : "task or source references"),
  ));

  root.append(sectionHeading("Recent history", recent.length ? plural(recent.length, "event") : "No recent events"));
  if (!recent.length) {
    root.append(emptyState("No recent events", "Immutable changelog events will appear after project changes."));
  } else {
    root.append(el("ol", { className: "event-list" }, recent.slice(0, 8).map((event) => el("li", { className: "card event" },
      timeElement(event.timestamp),
      el("div", {},
        el("div", { className: "event-action", text: event.action || "project.changed" }),
        el("div", { className: "event-copy", text: event.task_id ? `${event.task_id} · ${event.actor?.name || event.actor?.id || "unknown actor"}` : event.actor?.name || event.actor?.id || "unknown actor" }),
      ),
      event.task_revision !== undefined ? badge(`r${event.task_revision}`, toneFor(event.action)) : null,
    ))));
  }
  return root;
}

function metric(label, value, detail, extra = "") {
  return el("section", { className: `card metric ${extra}`.trim() },
    el("span", { className: "metric-label", text: label }),
    el("strong", { className: "metric-value", text: value }),
    el("span", { className: "metric-detail", text: detail }),
  );
}

function knowledgeMetric(label, total, subset, subsetLabel) {
  return el("section", { className: "card metric" },
    el("span", { className: "metric-label", text: label }),
    el("strong", { className: "metric-value", text: total }),
    el("span", { className: "metric-detail", text: subset === null ? subsetLabel : `${subset} ${subsetLabel}` }),
  );
}
