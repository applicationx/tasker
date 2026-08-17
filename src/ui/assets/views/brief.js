import { button, copyText, el, emptyState, labeledField, pageHeader, selectControl, textBlock } from "../dom.js";
import { projectHref, taskHref } from "../router.js";

export async function renderBrief(ctx) {
  const maxBytes = Math.min(10_000_000, Math.max(256, Number.parseInt(ctx.route.query.get("max_bytes") || "65536", 10) || 65536));
  const format = ["markdown", "json", "yaml"].includes(ctx.route.query.get("format")) ? ctx.route.query.get("format") : "markdown";
  const taskId = ctx.route.entityId;
  const data = await ctx.api.brief(ctx.prefix, taskId, maxBytes, format, { signal: ctx.signal });
  const rendered = renderedBrief(data, format);
  const byteCount = new TextEncoder().encode(rendered).byteLength;
  const truncation = data?.truncation || data?.metadata?.truncation || {};
  const omissions = Array.isArray(truncation.omitted) ? truncation.omitted : [];

  const root = el("div", {},
    pageHeader(
      taskId,
      "Agent brief",
      "A deterministic, bounded assembly of task context and project knowledge. Truncation and omissions remain visible.",
      [el("a", { className: "button", href: taskHref(ctx.prefix, taskId), text: "Back to task" })],
    ),
  );
  const formatSelect = selectControl([
    { value: "markdown", label: "Markdown" }, { value: "json", label: "JSON" }, { value: "yaml", label: "YAML" },
  ], format);
  const limitInput = el("input", { type: "number", min: "256", max: "10000000", step: "1024", value: maxBytes });
  const form = el("form", { className: "card brief-toolbar" },
    labeledField("Format", formatSelect),
    labeledField("Maximum bytes", limitInput),
    el("div", { className: "button-row" },
      button("Assemble", { kind: "submit", className: "button primary" }),
      button("Copy", { onclick: async () => {
        try { await copyText(rendered); ctx.toast("Brief copied to the clipboard."); }
        catch { ctx.toast("Brief could not be copied.", "error"); }
      } }),
    ),
  );
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const query = new URLSearchParams({ format: formatSelect.value, max_bytes: limitInput.value });
    window.location.hash = `${projectHref(ctx.prefix, `tasks/${encodeURIComponent(taskId)}/brief`).slice(1)}?${query}`;
  });
  root.append(form);

  if (truncation.truncated || omissions.length) {
    root.append(el("section", { className: "notice warning", role: "status" },
      el("strong", { text: "Brief was truncated" }),
      el("p", { text: `${byteCount.toLocaleString()} rendered bytes within a ${maxBytes.toLocaleString()} byte limit.` }),
      omissions.length ? el("ul", { className: "omission-list" }, omissions.map((item) => el("li", { text: omissionText(item) }))) : el("p", { text: "Lower-priority content was omitted." }),
    ));
  }
  root.append(el("div", { className: "results-meta" },
    el("span", { text: `${byteCount.toLocaleString()} rendered bytes` }),
    el("span", { text: format.toUpperCase() }),
  ));
  root.append(rendered ? textBlock(rendered) : emptyState("Brief is empty", "No content was returned for this task brief."));
  return root;
}

function renderedBrief(data, format) {
  if (typeof data === "string") return data;
  for (const key of ["rendered", "output", "content", "brief"]) {
    if (typeof data?.[key] === "string") return data[key];
  }
  if (format === "json") return JSON.stringify(data, null, 2);
  if (format === "yaml" && typeof data?.yaml === "string") return data.yaml;
  return JSON.stringify(data, null, 2);
}

function omissionText(item) {
  if (typeof item === "string") return item;
  return [item?.section || item?.path || "content", item?.id, item?.reason].filter(Boolean).join(" · ");
}
