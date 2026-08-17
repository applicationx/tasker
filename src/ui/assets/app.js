import { TaskerApi } from "./api.js";
import { clear, el, errorView, loadingView } from "./dom.js";
import { projectName, projectsFrom, usersFrom } from "./model.js";
import { HashRouter, projectHref } from "./router.js";
import { renderBoard } from "./views/board.js";
import { renderBrief } from "./views/brief.js";
import { renderDependencies } from "./views/dependencies.js";
import { renderHistory } from "./views/history.js";
import { renderDecision, renderKnowledge, renderProjectBrief, renderResource } from "./views/knowledge.js";
import { renderOverview } from "./views/overview.js";
import { renderProjects } from "./views/projects.js";
import { renderTask } from "./views/task.js";
import { renderTasks } from "./views/tasks.js";
import { renderWorkflow } from "./views/workflow.js";

const renderers = {
  projects: renderProjects,
  overview: renderOverview,
  board: renderBoard,
  tasks: renderTasks,
  task: renderTask,
  taskBrief: renderBrief,
  dependencies: renderDependencies,
  knowledge: renderKnowledge,
  resource: renderResource,
  decision: renderDecision,
  projectBrief: renderProjectBrief,
  history: renderHistory,
  workflow: renderWorkflow,
};

class TaskerApp {
  constructor() {
    this.api = new TaskerApi();
    this.router = new HashRouter();
    this.projects = [];
    this.snapshot = null;
    this.route = null;
    this.actorByProject = new Map();
    this.dirty = false;
    this.renderEpoch = 0;
    this.controller = null;
    this.lastPath = "";
    this.lastRefresh = 0;
    this.refreshTimer = null;
    this.stopChanges = null;

    this.view = document.querySelector("[data-route-view]");
    this.main = document.querySelector("#main-content");
    this.projectSwitcher = document.querySelector("[data-project-switcher]");
    this.actorSwitcher = document.querySelector("[data-actor-switcher]");
    this.projectNav = document.querySelector("[data-project-nav]");
    this.navProjectName = document.querySelector("[data-nav-project-name]");
    this.staleNotice = document.querySelector("[data-stale-notice]");
    this.liveRegion = document.querySelector("[data-live-region]");
    this.toastRegion = document.querySelector("[data-toast-region]");
    this.sidebar = document.querySelector("[data-sidebar]");
    this.navToggle = document.querySelector("[data-nav-toggle]");
    this.navScrim = document.querySelector("[data-nav-scrim]");
    this.mobileNav = window.matchMedia("(max-width: 1199px)");
  }

  async init() {
    this.bindShell();
    clear(this.view, loadingView());
    this.main.setAttribute("aria-busy", "true");
    try {
      await this.api.session();
      await this.loadProjects();
      this.stopChanges = this.api.watchChanges((change) => this.onInvalidation(change));
      this.router.start((route) => this.renderRoute(route));
    } catch (error) {
      this.main.setAttribute("aria-busy", "false");
      clear(this.view, errorView(error, () => window.location.reload()));
      this.focusHeading();
    }
  }

  bindShell() {
    document.querySelector("[data-refresh]").addEventListener("click", () => this.refresh());
    this.projectSwitcher.addEventListener("change", () => {
      const prefix = this.projectSwitcher.value;
      if (prefix) this.router.navigate(projectHref(prefix));
      else this.router.navigate("/projects");
    });
    this.actorSwitcher.addEventListener("change", () => {
      if (this.route?.prefix) this.actorByProject.set(this.route.prefix, this.actorSwitcher.value);
      this.announce(this.actorSwitcher.value ? `Acting as ${this.actorSwitcher.selectedOptions[0]?.textContent}.` : "No acting identity selected.");
    });
    this.navToggle.addEventListener("click", () => this.openNav(!document.body.classList.contains("nav-open")));
    this.navScrim.addEventListener("click", () => this.openNav(false));
    this.mobileNav.addEventListener("change", () => this.syncNavAccessibility());
    this.syncNavAccessibility();
    this.sidebar.addEventListener("click", (event) => {
      if (event.target.closest("a")) this.openNav(false);
    });
    document.querySelector("[data-stale-refresh]").addEventListener("click", () => this.refresh());

    document.addEventListener("click", (event) => {
      const link = event.target.closest("a[href^='#']");
      if (!link || !this.dirty || link.getAttribute("href") === window.location.hash) return;
      if (!window.confirm("Leave this view and discard the unsaved draft?")) event.preventDefault();
      else this.setDirty(false);
    }, true);
    document.addEventListener("keydown", (event) => this.handleGlobalKey(event));
    window.addEventListener("beforeunload", (event) => {
      if (!this.dirty) return;
      event.preventDefault();
      event.returnValue = "";
    });
    window.addEventListener("focus", () => this.refreshOnReturn());
    document.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "visible") this.refreshOnReturn();
    });
  }

  async loadProjects(signal) {
    this.projects = await this.api.projects({ signal });
    this.updateProjectSwitcher();
  }

  async renderRoute(route, options = {}) {
    const epoch = ++this.renderEpoch;
    this.controller?.abort();
    this.controller = new AbortController();
    const signal = this.controller.signal;
    const pathChanged = route.path !== this.lastPath;
    if (pathChanged) this.setDirty(false);
    this.route = route;
    this.hideStale();
    this.main.setAttribute("aria-busy", "true");
    clear(this.view, loadingView());
    this.updateNavigation(route);

    try {
      if (route.name === "projects") {
        if (options.reloadProjects) await this.loadProjects(signal);
        this.snapshot = null;
      } else if (route.prefix) {
        this.snapshot = await this.api.snapshot(route.prefix, { signal });
        this.updateProjectContext(route.prefix, this.snapshot);
      }
      if (epoch !== this.renderEpoch) return;
      const renderer = renderers[route.name];
      const node = renderer ? await renderer(this.context(signal)) : this.notFound();
      if (epoch !== this.renderEpoch) return;
      clear(this.view, node);
      this.main.setAttribute("aria-busy", "false");
      this.lastRefresh = Date.now();
      this.lastPath = route.path;
      this.updateNavigation(route);
      document.title = `${this.view.querySelector("h1")?.textContent || "Tasker"} · Tasker`;
      if (pathChanged) this.focusHeading();
    } catch (error) {
      if (error?.name === "AbortError" || epoch !== this.renderEpoch) return;
      this.main.setAttribute("aria-busy", "false");
      clear(this.view, errorView(error, () => this.refresh({ force: true })));
      this.focusHeading();
    }
  }

  context(signal) {
    return {
      api: this.api,
      app: this,
      route: this.route,
      prefix: this.route?.prefix,
      snapshot: this.snapshot,
      projects: this.projects,
      signal,
      actor: () => this.actorSwitcher.value,
      mutate: (payload) => {
        if (!this.actorSwitcher.value) return Promise.reject({ code: "actor_required", message: "Choose an existing project user before saving." });
        return this.api.mutate(this.route.prefix, payload, { signal });
      },
      refresh: (options) => this.refresh(options),
      setDirty: (value) => this.setDirty(value),
      markSaved: (form) => this.markSaved(form),
      toast: (message, kind) => this.toast(message, kind),
      announce: (message) => this.announce(message),
    };
  }

  async refresh(options = {}) {
    if (this.refreshTimer !== null) {
      window.clearTimeout(this.refreshTimer);
      this.refreshTimer = null;
    }
    if (this.dirty && !options.force) {
      if (!window.confirm("Reload from disk and discard the unsaved draft?")) return;
    }
    this.setDirty(false);
    this.hideStale();
    if (!this.route) return;
    if (this.route.name === "projects") return this.renderRoute(this.route, { reloadProjects: true });
    return this.renderRoute(this.route);
  }

  refreshOnReturn() {
    if (!this.route || Date.now() - this.lastRefresh < 500) return;
    if (this.dirty) this.showStale();
    else this.scheduleRefresh(100);
  }

  onInvalidation(change = {}) {
    const changedPrefix = change.project_prefix || change.prefix || change.project;
    this.loadProjects().catch(() => {});
    if (this.route?.name === "projects") {
      this.scheduleRefresh(250, true);
      return;
    }
    if (changedPrefix && changedPrefix !== this.route?.prefix && change.scope !== "root") return;
    if (this.dirty) this.showStale();
    else this.scheduleRefresh(250);
  }

  scheduleRefresh(delay, reloadProjects = false) {
    if (this.refreshTimer !== null) window.clearTimeout(this.refreshTimer);
    this.refreshTimer = window.setTimeout(() => {
      this.refreshTimer = null;
      if (this.dirty) {
        this.showStale();
        return;
      }
      if (this.route?.name === "projects") this.renderRoute(this.route, { reloadProjects });
      else this.refresh({ force: true });
    }, delay);
  }

  updateProjectSwitcher() {
    const projects = projectsFrom(this.projects);
    const current = this.route?.prefix || "";
    clear(this.projectSwitcher, el("option", { value: "", text: "Projects" }));
    for (const project of projects) {
      this.projectSwitcher.append(el("option", { value: project.prefix, text: `${project.prefix} · ${project.name || project.prefix}` }));
    }
    this.projectSwitcher.value = projects.some((project) => project.prefix === current) ? current : "";
  }

  updateProjectContext(prefix, snapshot) {
    this.updateProjectSwitcher();
    this.projectSwitcher.value = prefix;
    const users = usersFrom(snapshot);
    const previous = this.actorByProject.get(prefix);
    const selected = users.some((user) => user.id === previous) ? previous : users[0]?.id || "";
    clear(this.actorSwitcher);
    if (!users.length) this.actorSwitcher.append(el("option", { value: "", text: "No project users" }));
    for (const user of users) {
      this.actorSwitcher.append(el("option", { value: user.id, text: user.name ? `${user.name} (${user.id})` : user.id }));
    }
    this.actorSwitcher.value = selected;
    this.actorSwitcher.disabled = !users.length;
    this.actorByProject.set(prefix, selected);
    this.projectNav.hidden = false;
    this.navProjectName.textContent = projectName(snapshot, prefix);
  }

  updateNavigation(route) {
    const prefix = route.prefix;
    if (!prefix) {
      this.projectNav.hidden = true;
      this.actorSwitcher.disabled = true;
      clear(this.actorSwitcher, el("option", { value: "", text: "No project user" }));
      this.projectSwitcher.value = "";
    } else {
      this.projectNav.hidden = false;
      for (const link of document.querySelectorAll("[data-project-route]")) {
        link.href = projectHref(prefix, link.dataset.projectRoute);
      }
    }
    const section = route.name === "task" || route.name === "taskBrief" ? "tasks"
      : ["resource", "decision", "projectBrief"].includes(route.name) ? "knowledge" : route.name;
    for (const link of document.querySelectorAll(".nav-link")) {
      const active = link.dataset.nav === section || link.dataset.projectRoute === section;
      if (active) link.setAttribute("aria-current", "page");
      else link.removeAttribute("aria-current");
    }
  }

  setDirty(value) {
    this.dirty = Boolean(value);
    if (this.dirty && this.refreshTimer !== null) {
      window.clearTimeout(this.refreshTimer);
      this.refreshTimer = null;
    }
    if (!this.dirty) this.hideStale();
  }

  markSaved(form) {
    form.dataset.dirty = "false";
    const draftsRemain = Boolean(this.view.querySelector('[data-dirty="true"]'));
    this.setDirty(draftsRemain);
    if (draftsRemain) {
      this.showStale();
      this.announce("Change saved. Another unsaved draft remains on this page and was preserved.");
    }
    return draftsRemain;
  }

  showStale() { this.staleNotice.hidden = false; }
  hideStale() { this.staleNotice.hidden = true; }

  focusHeading() {
    window.requestAnimationFrame(() => this.view.querySelector("[data-route-heading]")?.focus());
  }

  announce(message) {
    this.liveRegion.textContent = "";
    window.setTimeout(() => { this.liveRegion.textContent = message; }, 20);
  }

  toast(message, kind = "") {
    const toast = el("div", { className: `toast ${kind}`.trim(), role: kind === "error" ? "alert" : "status", text: message });
    this.toastRegion.append(toast);
    window.setTimeout(() => toast.remove(), 4500);
  }

  openNav(open) {
    const expanded = Boolean(open && this.mobileNav.matches);
    if (!expanded && this.sidebar.contains(document.activeElement)) this.navToggle.focus();
    document.body.classList.toggle("nav-open", expanded);
    this.navScrim.hidden = !expanded;
    this.navToggle.setAttribute("aria-expanded", String(expanded));
    if (expanded) {
      this.sidebar.inert = false;
      this.sidebar.removeAttribute("aria-hidden");
      this.sidebar.setAttribute("role", "dialog");
      this.sidebar.setAttribute("aria-modal", "true");
      this.sidebar.setAttribute("aria-label", "Navigation");
      this.sidebar.querySelector("a")?.focus();
    } else {
      this.sidebar.removeAttribute("role");
      this.sidebar.removeAttribute("aria-modal");
      this.sidebar.removeAttribute("aria-label");
      this.syncNavAccessibility();
    }
  }

  syncNavAccessibility() {
    const closedMobile = this.mobileNav.matches && !document.body.classList.contains("nav-open");
    this.sidebar.inert = closedMobile;
    if (closedMobile) this.sidebar.setAttribute("aria-hidden", "true");
    else this.sidebar.removeAttribute("aria-hidden");
    if (!this.mobileNav.matches) {
      document.body.classList.remove("nav-open");
      this.navScrim.hidden = true;
      this.navToggle.setAttribute("aria-expanded", "false");
    }
  }

  handleGlobalKey(event) {
    if (!document.body.classList.contains("nav-open")) return;
    if (event.key === "Escape") {
      event.preventDefault();
      this.openNav(false);
      this.navToggle.focus();
      return;
    }
    if (event.key !== "Tab") return;
    const focusable = [...this.sidebar.querySelectorAll('a[href], button:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])')];
    if (!focusable.length) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
  }

  notFound() {
    return el("section", { className: "error-panel" },
      el("p", { className: "eyebrow", text: "404" }),
      el("h1", { text: "View not found", tabindex: "-1", dataset: { routeHeading: "true" } }),
      el("p", { text: "This Tasker UI route does not exist." }),
      el("a", { className: "button primary", href: "#/projects", text: "Go to projects" }),
    );
  }
}

const app = new TaskerApp();
app.init();
