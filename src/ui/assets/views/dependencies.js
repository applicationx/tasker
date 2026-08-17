import { badge, el, emptyState, pageHeader, plural, sectionHeading } from "../dom.js";
import { taskRecord, taskRecords } from "../model.js";
import { taskHref } from "../router.js";

export function renderDependencies(ctx) {
  const records = taskRecords(ctx.snapshot);
  const edges = records.flatMap(({ task, status }) => (task.dependencies || []).map((dependency) => ({
    task,
    dependency,
    blocked: Array.isArray(status?.unresolved_dependencies)
      ? status.unresolved_dependencies.includes(dependency)
      : Boolean(status?.dependency_blocked),
  })));
  const root = el("div", {}, pageHeader(
    ctx.prefix,
    "Dependencies",
    "Blocking dependencies are distinct from non-blocking relations. The list below is the accessible source of truth for this visualization.",
  ));
  if (!edges.length) {
    root.append(emptyState("No task dependencies", "Dependencies added through the Tasker CLI will appear here."));
    return root;
  }

  root.append(sectionHeading("Dependency map", plural(edges.length, "dependency edge")));
  root.append(el("div", { className: "card dependency-map", "aria-hidden": "true" }, edges.map((edge) => {
    const dependencyTask = taskRecord(ctx.snapshot, edge.dependency)?.task;
    return el("div", { className: "graph-edge" },
      el("div", { className: "graph-node", text: dependencyTask ? `${dependencyTask.id} · ${dependencyTask.header}` : `${edge.dependency} · missing` }),
      el("span", { className: "graph-arrow", text: "→" }),
      el("div", { className: "graph-node", text: `${edge.task.id} · ${edge.task.header}` }),
    );
  })));

  root.append(sectionHeading("Accessible dependency list", "Dependency first, dependent second"));
  root.append(el("section", { className: "card card-body" },
    el("ul", { className: "dependency-list" }, edges.map((edge) => {
      const dependencyTask = taskRecord(ctx.snapshot, edge.dependency)?.task;
      return el("li", {},
        el("div", {},
          dependencyTask
            ? el("a", { href: taskHref(ctx.prefix, dependencyTask.id), text: `${dependencyTask.id} · ${dependencyTask.header}` })
            : el("span", { className: "identifier", text: `${edge.dependency} (missing)` }),
        ),
        el("span", { className: "dependency-word", text: "is required by" }),
        el("div", {},
          el("a", { href: taskHref(ctx.prefix, edge.task.id), text: `${edge.task.id} · ${edge.task.header}` }),
          edge.blocked ? badge("Unresolved", "blocked") : badge("Satisfied", "success"),
        ),
      );
    })),
  ));
  return root;
}
