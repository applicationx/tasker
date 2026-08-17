import { el, emptyState, pageHeader } from "../dom.js";
import { stateEntries, taskRecords } from "../model.js";
import { taskCard } from "./shared.js";

export function renderBoard(ctx) {
  const states = stateEntries(ctx.snapshot);
  const records = taskRecords(ctx.snapshot);
  const root = el("div", {},
    pageHeader(
      ctx.prefix,
      "Workflow board",
      "Cards follow the configured workflow. Open a task to make an explicit transition; board movement never mutates files.",
    ),
  );

  if (!states.length) {
    root.append(emptyState("No workflow to display", "Add configured workflow states through the Tasker project file and refresh."));
    return root;
  }

  const board = el("div", { className: "workflow-board" });
  states.forEach(([key, config], index) => {
    const items = records.filter(({ task }) => task.state === key);
    board.append(el("section", { className: `board-column tone-${index % 6}`, "aria-labelledby": `board-state-${index}` },
      el("header", { className: "board-column-header" },
        el("h2", { id: `board-state-${index}`, text: config.label || key }),
        el("span", { className: "board-count", text: items.length, "aria-label": `${items.length} tasks` }),
      ),
      items.length
        ? el("div", { className: "board-cards" }, items.map((record) => taskCard(ctx, record)))
        : el("p", { className: "board-empty", text: "No tasks in this state" }),
    ));
  });
  root.append(el("div", { className: "board-wrap", tabindex: "0", "aria-label": "Workflow board. Scroll horizontally to see all states." }, board));
  return root;
}
