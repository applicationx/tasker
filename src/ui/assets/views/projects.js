import { badge, el, emptyState, pageHeader, plural } from "../dom.js";
import { projectsFrom, totalProjectStats } from "../model.js";
import { projectHref } from "../router.js";

export function renderProjects(ctx) {
  const projects = projectsFrom(ctx.projects);
  const root = el("div", {},
    pageHeader("Workspace", "Choose a project", "Projects are read directly from your configured Tasker root. Manual edits and Git checkouts appear on refresh."),
  );

  if (!projects.length) {
    root.append(emptyState(
      "No Tasker projects found",
      "Create a project with the Tasker CLI in this projects root, then refresh this page. The Web UI does not create projects.",
    ));
    return root;
  }

  const grid = el("div", { className: "grid grid-3", "aria-label": "Tasker projects" });
  for (const project of projects) {
    const stats = project.stats || project.task_counts || {};
    const total = totalProjectStats(project);
    const stateStats = Object.entries(stats).slice(0, 4);
    grid.append(el("a", { className: "card card-link project-card", href: projectHref(project.prefix) },
      el("div", { className: "project-card-top" },
        el("div", {},
          el("span", { className: "identifier", text: project.prefix }),
          el("h2", { text: project.name || project.prefix }),
        ),
        badge(plural(total, "task"), 1),
      ),
      el("p", { className: "project-path", text: project.path || "Local project" }),
      el("div", { className: "project-stats" },
        stateStats.length
          ? stateStats.map(([state, count]) => el("span", {}, el("strong", { text: count }), ` ${state}`))
          : el("span", { text: total ? plural(total, "task") : "No tasks yet" }),
      ),
    ));
  }
  root.append(grid);
  return root;
}
