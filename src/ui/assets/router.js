const ROUTES = [
  ["projects", /^\/projects\/?$/],
  ["overview", /^\/p\/([^/]+)\/overview\/?$/],
  ["board", /^\/p\/([^/]+)\/board\/?$/],
  ["tasks", /^\/p\/([^/]+)\/tasks\/?$/],
  ["taskBrief", /^\/p\/([^/]+)\/tasks\/([^/]+)\/brief\/?$/],
  ["task", /^\/p\/([^/]+)\/tasks\/([^/]+)\/?$/],
  ["dependencies", /^\/p\/([^/]+)\/dependencies\/?$/],
  ["knowledge", /^\/p\/([^/]+)\/knowledge\/?$/],
  ["resource", /^\/p\/([^/]+)\/knowledge\/resources\/([^/]+)\/?$/],
  ["decision", /^\/p\/([^/]+)\/knowledge\/decisions\/([^/]+)\/?$/],
  ["projectBrief", /^\/p\/([^/]+)\/knowledge\/project-brief\/?$/],
  ["history", /^\/p\/([^/]+)\/history\/?$/],
  ["workflow", /^\/p\/([^/]+)\/workflow\/?$/],
];

export class HashRouter {
  #listener = null;
  #handler;

  constructor() {
    this.#handler = () => this.#dispatch();
  }

  start(listener) {
    this.#listener = listener;
    window.addEventListener("hashchange", this.#handler);
    if (!window.location.hash || window.location.hash === "#") {
      window.location.replace(`${window.location.pathname}${window.location.search}#/projects`);
    } else {
      this.#dispatch();
    }
  }

  stop() {
    window.removeEventListener("hashchange", this.#handler);
  }

  navigate(path, replace = false) {
    const hash = path.startsWith("#") ? path : `#${path.startsWith("/") ? path : `/${path}`}`;
    if (replace) window.location.replace(hash);
    else window.location.hash = hash.slice(1);
  }

  current() {
    return parseRoute(window.location.hash);
  }

  #dispatch() {
    this.#listener?.(this.current());
  }
}

export function parseRoute(hash) {
  const raw = (hash || "#/projects").replace(/^#/, "");
  const queryAt = raw.indexOf("?");
  const path = queryAt >= 0 ? raw.slice(0, queryAt) : raw;
  const query = new URLSearchParams(queryAt >= 0 ? raw.slice(queryAt + 1) : "");
  for (const [name, pattern] of ROUTES) {
    const match = pattern.exec(path);
    if (!match) continue;
    try {
      return {
        name,
        path,
        query,
        prefix: match[1] ? decodeURIComponent(match[1]) : null,
        entityId: match[2] ? decodeURIComponent(match[2]) : null,
      };
    } catch {
      return { name: "notFound", path, query, prefix: null, entityId: null };
    }
  }
  return { name: "notFound", path, query, prefix: null, entityId: null };
}

export function projectHref(prefix, suffix = "overview") {
  return `#/p/${encodeURIComponent(prefix)}/${suffix}`;
}

export function taskHref(prefix, taskId) {
  return projectHref(prefix, `tasks/${encodeURIComponent(taskId)}`);
}

export function resourceHref(prefix, resourceId) {
  return projectHref(prefix, `knowledge/resources/${encodeURIComponent(resourceId)}`);
}

export function decisionHref(prefix, decisionId) {
  return projectHref(prefix, `knowledge/decisions/${encodeURIComponent(decisionId)}`);
}
