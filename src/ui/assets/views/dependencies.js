import { el, emptyState, pageHeader, plural, sectionHeading, tint } from "../dom.js";
import { stateColor, taskRecord, taskRecords } from "../model.js";
import { taskHref } from "../router.js";

const NODE_WIDTH = 186;
const NODE_HEIGHT = 66;
const GAP_X = 58;
const GAP_Y = 30;
const PAD = 30;
const SVG = "http://www.w3.org/2000/svg";

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
    null,
    "Dependencies",
    "Prerequisites flow left to right. Dashed edges are unresolved. Blocking dependencies are distinct from non-blocking relations.",
  ));
  if (!edges.length) {
    root.append(emptyState("No task dependencies", "Dependencies added through the Tasker CLI will appear here."));
    return root;
  }

  root.append(graph(ctx, edges));
  root.append(el("div", { className: "graph-legend", "aria-hidden": "true" },
    el("span", {}, el("span", { className: "legend-line" }), "satisfied"),
    el("span", {}, el("span", { className: "legend-line unresolved" }), "unresolved"),
    el("span", { text: `${plural(edges.length, "edge")} · Tasker rejects dependency cycles on write.` }),
  ));

  root.append(sectionHeading("Dependency list", "Prerequisite first, dependent second"));
  root.append(el("section", { className: "panel" },
    el("ul", { className: "dependency-list" }, edges.map((edge) => {
      const dependencyTask = taskRecord(ctx.snapshot, edge.dependency)?.task;
      return el("li", {},
        dependencyTask
          ? el("a", { href: taskHref(ctx.prefix, dependencyTask.id) }, el("span", { className: "identifier", text: dependencyTask.id }), ` ${dependencyTask.header}`)
          : el("span", { className: "identifier", text: `${edge.dependency} (missing)` }),
        el("span", { className: "dependency-word", text: "is required by" }),
        el("a", { href: taskHref(ctx.prefix, edge.task.id) }, el("span", { className: "identifier", text: edge.task.id }), ` ${edge.task.header}`),
        tint(el("span", { className: "state-label", text: edge.blocked ? "unresolved" : "satisfied" }), edge.blocked ? "var(--bad)" : "var(--ok)"),
      );
    })),
  ));
  return root;
}

// Layered layout: each task sits one column right of its deepest prerequisite;
// rows are ordered by the mean row of prerequisites to limit edge crossings.
function graph(ctx, edges) {
  const ids = [...new Set(edges.flatMap((edge) => [edge.dependency, edge.task.id]))];
  const prerequisites = new Map(ids.map((id) => [id, []]));
  for (const edge of edges) prerequisites.get(edge.task.id).push(edge.dependency);
  const depth = new Map();
  const visiting = new Set();
  const depthOf = (id) => {
    if (depth.has(id)) return depth.get(id);
    if (visiting.has(id)) return 0;
    visiting.add(id);
    const value = prerequisites.get(id).reduce((max, dependency) => Math.max(max, depthOf(dependency) + 1), 0);
    visiting.delete(id);
    depth.set(id, value);
    return value;
  };
  ids.forEach(depthOf);

  const columns = [];
  for (const id of ids) (columns[depth.get(id)] ||= []).push(id);
  const row = new Map();
  columns.forEach((column) => {
    const weight = (id) => {
      const rows = prerequisites.get(id).map((dependency) => row.get(dependency)).filter((value) => value !== undefined);
      return rows.length ? rows.reduce((sum, value) => sum + value, 0) / rows.length : Number.POSITIVE_INFINITY;
    };
    column.sort((a, b) => weight(a) - weight(b) || a.localeCompare(b, undefined, { numeric: true }));
    column.forEach((id, index) => row.set(id, index));
  });

  const position = (id) => ({
    x: PAD + depth.get(id) * (NODE_WIDTH + GAP_X),
    y: PAD + row.get(id) * (NODE_HEIGHT + GAP_Y),
  });
  const width = PAD * 2 + columns.length * NODE_WIDTH + (columns.length - 1) * GAP_X;
  const height = PAD * 2 + Math.max(...columns.map((column) => column.length)) * (NODE_HEIGHT + GAP_Y) - GAP_Y;

  const svg = document.createElementNS(SVG, "svg");
  svg.setAttribute("width", String(width));
  svg.setAttribute("height", String(height));
  svg.setAttribute("aria-hidden", "true");
  for (const edge of edges) {
    const from = position(edge.dependency);
    const to = position(edge.task.id);
    const x1 = from.x + NODE_WIDTH;
    const y1 = from.y + NODE_HEIGHT / 2;
    const x2 = to.x;
    const y2 = to.y + NODE_HEIGHT / 2;
    const bend = Math.max(24, (x2 - x1) / 2);
    const path = document.createElementNS(SVG, "path");
    path.setAttribute("d", `M ${x1} ${y1} C ${x1 + bend} ${y1}, ${x2 - bend} ${y2}, ${x2} ${y2}`);
    path.setAttribute("fill", "none");
    path.setAttribute("stroke", edge.blocked ? "oklch(0.55 0.09 22)" : "oklch(0.45 0.008 80)");
    path.setAttribute("stroke-width", "1.5");
    if (edge.blocked) path.setAttribute("stroke-dasharray", "5 5");
    svg.append(path);
  }

  const canvas = el("div", { className: "graph-canvas" }, svg);
  canvas.style.width = `${width}px`;
  canvas.style.height = `${height}px`;
  for (const id of ids) {
    const task = taskRecord(ctx.snapshot, id)?.task;
    const { x, y } = position(id);
    const node = el("a", { className: "graph-node", href: taskHref(ctx.prefix, id), title: task ? `${id} · ${task.header}` : `${id} (missing)` },
      el("span", { className: "identifier", text: id }),
      el("span", { text: task?.header || "Missing task" }),
    );
    tint(node, task ? stateColor(ctx.snapshot, task.state) : "var(--bad)");
    node.style.left = `${x}px`;
    node.style.top = `${y}px`;
    canvas.append(node);
  }
  return el("div", { className: "graph-frame", tabindex: "0", "aria-label": "Dependency graph. The list below contains the same information." }, canvas);
}
