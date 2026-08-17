let nextId = 0;

export function id(prefix = "ui") {
  nextId += 1;
  return `${prefix}-${nextId}`;
}

export function el(tag, attributes = {}, ...children) {
  const node = document.createElement(tag);
  for (const [name, value] of Object.entries(attributes || {})) {
    if (value === undefined || value === null || value === false) continue;
    if (name === "className") node.className = value;
    else if (name === "text") node.textContent = String(value);
    else if (name === "checked") node.checked = Boolean(value);
    else if (name === "disabled") node.disabled = Boolean(value);
    else if (name === "value") node.value = String(value);
    else if (name === "dataset") {
      for (const [key, item] of Object.entries(value)) node.dataset[key] = String(item);
    } else if (name.startsWith("on") && typeof value === "function") {
      node.addEventListener(name.slice(2).toLowerCase(), value);
    } else if (value === true) node.setAttribute(name, "");
    else node.setAttribute(name, String(value));
  }
  append(node, children);
  return node;
}

export function append(parent, children) {
  for (const child of children.flat(Infinity)) {
    if (child === undefined || child === null || child === false) continue;
    parent.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return parent;
}

export function fragment(...children) {
  const value = document.createDocumentFragment();
  append(value, children);
  return value;
}

export function clear(node, ...children) {
  node.replaceChildren();
  append(node, children);
  return node;
}

export function pageHeader(eyebrow, title, summary, actions = []) {
  const heading = el("h1", { text: title, tabindex: "-1", dataset: { routeHeading: "true" } });
  return el("header", { className: "page-header" },
    el("div", {},
      eyebrow ? el("p", { className: "eyebrow", text: eyebrow }) : null,
      heading,
      summary ? el("p", { className: "page-summary", text: summary }) : null,
    ),
    actions.length ? el("div", { className: "header-actions" }, actions) : null,
  );
}

export function sectionHeading(title, note = "") {
  return el("div", { className: "section-heading" },
    el("h2", { text: title }),
    note ? el("p", { text: note }) : null,
  );
}

export function button(label, options = {}) {
  const { kind = "button", className = "button", ...attrs } = options;
  return el("button", { type: kind, className, ...attrs }, label);
}

export function linkButton(label, href, className = "button") {
  return el("a", { href, className }, label);
}

export function badge(label, tone = 0, extra = "") {
  const toneClass = typeof tone === "number" ? `tone-${tone % 6}` : tone;
  return el("span", { className: `badge ${toneClass} ${extra}`.trim(), text: label });
}

export function revisionBadge(revision) {
  return el("span", { className: "revision-badge", text: `revision ${revision ?? "—"}` });
}

export function tagList(tags = []) {
  if (!Array.isArray(tags) || !tags.length) return null;
  return el("ul", { className: "tag-list", "aria-label": "Tags" },
    tags.map((tag) => el("li", { className: "tag", text: tag })),
  );
}

export function emptyState(title, message, action = null) {
  return el("section", { className: "empty-state card" },
    el("div", {}, el("h2", { text: title }), el("p", { text: message }), action ? el("div", { className: "form-actions" }, action) : null),
  );
}

export function notice(message, kind = "") {
  return el("div", { className: `notice ${kind}`.trim(), role: kind === "error" ? "alert" : "status", text: message });
}

export function errorView(error, retry) {
  const code = error?.code || "request_failed";
  const message = error?.message || "Tasker could not load this view.";
  return el("section", { className: "error-panel", role: "alert" },
    el("p", { className: "eyebrow", text: "Unable to load" }),
    el("h1", { text: "Something needs attention", tabindex: "-1", dataset: { routeHeading: "true" } }),
    el("p", { text: message }),
    el("p", { className: "error-code", text: code }),
    retry ? button("Try again", { className: "button primary", onclick: retry }) : null,
  );
}

export function loadingView() {
  return el("div", { "aria-label": "Loading view", role: "status" },
    el("span", { className: "sr-only", text: "Loading" }),
    el("div", { className: "skeleton title" }),
    el("div", { className: "skeleton line short" }),
    el("div", { className: "grid grid-4 loading-grid" },
      [0, 1, 2, 3].map(() => el("div", { className: "skeleton skeleton-card" })),
    ),
  );
}

export function labeledField(labelText, control, options = {}) {
  const label = el("label", { className: `field ${options.full ? "full" : ""}`.trim() },
    el("span", { text: labelText }),
    control,
    options.hint ? el("span", { className: "field-hint", text: options.hint }) : null,
  );
  return label;
}

export function selectControl(options, selected = "", attributes = {}) {
  const select = el("select", attributes);
  for (const option of options) {
    const value = typeof option === "string" ? option : option.value;
    const label = typeof option === "string" ? option : option.label;
    select.append(el("option", { value, text: label, selected: String(value) === String(selected) }));
  }
  return select;
}

export function setPending(container, pending, label = "Saving…") {
  container.setAttribute("aria-busy", String(pending));
  for (const control of container.querySelectorAll("button, input, select, textarea")) {
    if (pending) {
      control.dataset.wasDisabled = String(control.disabled);
      control.disabled = true;
    } else {
      control.disabled = control.dataset.wasDisabled === "true";
      delete control.dataset.wasDisabled;
    }
  }
  const status = container.querySelector("[data-form-status]");
  if (status) status.textContent = pending ? label : "";
}

export function inlineError(container, error) {
  let target = container.querySelector("[data-inline-error]");
  if (!target) {
    target = el("div", { className: "notice error", role: "alert", dataset: { inlineError: "true" } });
    container.prepend(target);
  }
  target.textContent = error?.message || "The change could not be saved.";
}

export function removeInlineError(container) {
  container.querySelector("[data-inline-error]")?.remove();
}

export function conflictBanner(error, handlers) {
  const expected = error?.details?.expected_revision ?? error?.expected_revision;
  const current = error?.details?.current_revision ?? error?.current_revision;
  const detail = expected !== undefined && current !== undefined
    ? `You edited revision ${expected}; the current revision is ${current}.`
    : "The file changed after this editor loaded.";
  return el("div", { className: "conflict-banner", role: "alert", dataset: { conflictBanner: "true" } },
    el("strong", { text: "Your draft was not saved" }),
    el("p", { text: `${detail} Your input remains in this form.` }),
    el("div", { className: "button-row" },
      button("Reload latest", { className: "button", onclick: handlers.reload }),
      button("Copy draft", { className: "button", onclick: handlers.copy }),
    ),
  );
}

export async function copyText(text) {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }
  const area = el("textarea", { value: text, className: "sr-only", "aria-hidden": "true" });
  document.body.append(area);
  area.select();
  document.execCommand("copy");
  area.remove();
}

export function formatDate(value, style = "short") {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return String(value);
  const options = style === "long"
    ? { dateStyle: "medium", timeStyle: "medium" }
    : { dateStyle: "medium", timeStyle: "short" };
  return new Intl.DateTimeFormat(undefined, options).format(date);
}

export function timeElement(value, style = "short") {
  return el("time", { datetime: value || "", text: formatDate(value, style), title: value || "" });
}

export function jsonBlock(value) {
  return el("pre", { className: "json-block", text: JSON.stringify(value ?? {}, null, 2) });
}

export function textBlock(value, className = "markdown-text") {
  return el("pre", { className, text: value || "" });
}

export function plural(count, singular, pluralValue = `${singular}s`) {
  return `${count} ${count === 1 ? singular : pluralValue}`;
}

export function setupTabs(root, initial = 0) {
  const tabs = [...root.querySelectorAll('[role="tab"]')];
  const panels = [...root.querySelectorAll('[role="tabpanel"]')];
  const activate = (index, focus = false) => {
    tabs.forEach((tab, itemIndex) => {
      const selected = itemIndex === index;
      tab.setAttribute("aria-selected", String(selected));
      tab.tabIndex = selected ? 0 : -1;
      panels[itemIndex].hidden = !selected;
    });
    if (focus) tabs[index]?.focus();
  };
  tabs.forEach((tab, index) => {
    tab.addEventListener("click", () => activate(index));
    tab.addEventListener("keydown", (event) => {
      let target = index;
      if (event.key === "ArrowRight") target = (index + 1) % tabs.length;
      else if (event.key === "ArrowLeft") target = (index - 1 + tabs.length) % tabs.length;
      else if (event.key === "Home") target = 0;
      else if (event.key === "End") target = tabs.length - 1;
      else return;
      event.preventDefault();
      activate(target, true);
    });
  });
  activate(Math.min(initial, tabs.length - 1));
}

export function toneFor(value, keys = []) {
  const index = keys.indexOf(value);
  if (index >= 0) return index % 6;
  let hash = 0;
  for (const character of String(value || "")) hash = ((hash << 5) - hash + character.charCodeAt(0)) | 0;
  return Math.abs(hash) % 6;
}

export function draftText(fields) {
  return Object.entries(fields).map(([name, value]) => `${name}:\n${Array.isArray(value) ? value.join(", ") : value ?? ""}`).join("\n\n");
}
