import { dot, el, emptyState, pageHeader } from "../dom.js";
import { stateColor, stateEntries, taskRecords } from "../model.js";
import { taskCard } from "./shared.js";

export function renderBoard(ctx) {
  const states = stateEntries(ctx.snapshot);
  const records = taskRecords(ctx.snapshot);
  const root = el("div", {},
    pageHeader(null, "Board", "Columns come from the project workflow, not from a fixed list. Open a task to make an explicit transition."),
  );

  if (!states.length) {
    root.append(emptyState("No workflow to display", "Add configured workflow states through the Tasker project file and refresh."));
    return root;
  }

  const board = el("div", { className: "workflow-board" });
  board.style.setProperty("--columns", String(states.length));
  states.forEach(([key, config], index) => {
    const items = records.filter(({ task }) => task.state === key);
    board.append(el("section", { className: "board-column", "aria-labelledby": `board-state-${index}` },
      el("header", { className: "board-column-header" },
        dot(stateColor(ctx.snapshot, key)),
        el("h2", { id: `board-state-${index}`, text: config.label || key }),
        el("span", { className: "board-count", text: items.length, "aria-label": `${items.length} tasks` }),
      ),
      items.length
        ? el("ul", { className: "board-cards" }, items.map((record) => el("li", {}, taskCard(ctx, record))))
        : el("p", { className: "board-empty", text: "Empty" }),
    ));
  });
  root.append(el("div", { className: "board-wrap", tabindex: "0", "aria-label": "Workflow board. Scroll horizontally to see all states." }, board));
  return root;
}
