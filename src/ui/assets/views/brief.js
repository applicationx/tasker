import { button, copyText, el, emptyState, pageHeader, segmented, textBlock } from "../dom.js";
import { projectHref, taskHref } from "../router.js";

const FORMATS = [
  { value: "markdown", label: "Markdown", mono: true },
  { value: "json", label: "JSON", mono: true },
  { value: "yaml", label: "YAML", mono: true },
];

export async function renderBrief(ctx) {
  const maxBytes = Math.min(10_000_000, Math.max(256, Number.parseInt(ctx.route.query.get("max_bytes") || "65536", 10) || 65536));
  const format = FORMATS.some((item) => item.value === ctx.route.query.get("format")) ? ctx.route.query.get("format") : "markdown";
  const taskId = ctx.route.entityId;
  const data = await ctx.api.brief(ctx.prefix, taskId, maxBytes, format, { signal: ctx.signal });
  const rendered = renderedBrief(data, format);
  const byteCount = new TextEncoder().encode(rendered).byteLength;
  const truncation = data?.truncation || data?.metadata?.truncation || {};
  const omissions = Array.isArray(truncation.omitted) ? truncation.omitted : [];
  const briefHref = (query) => `${projectHref(ctx.prefix, `tasks/${encodeURIComponent(taskId)}/brief`).slice(1)}?${new URLSearchParams(query)}`;

  const limitInput = el("input", { type: "number", className: "compact-input", min: "256", max: "10000000", step: "1024", value: maxBytes, "aria-label": "Maximum bytes" });
  const limitForm = el("form", { className: "inline-form" }, limitInput, button("Assemble", { kind: "submit", className: "button" }));
  limitForm.addEventListener("submit", (event) => {
    event.preventDefault();
    window.location.hash = briefHref({ format, max_bytes: limitInput.value });
  });

  const root = el("div", {},
    el("div", { className: "detail-toolbar" },
      el("a", { className: "button ghost", href: taskHref(ctx.prefix, taskId), text: `← ${taskId}` }),
    ),
    pageHeader(null, "Agent brief", `The complete contract handed to whoever claims ${taskId}.`, [
      segmented("Brief format", FORMATS, format, (value) => { window.location.hash = briefHref({ format: value, max_bytes: maxBytes }); }),
    ]),
  );

  if (truncation.truncated || omissions.length) {
    root.append(el("section", { className: "notice warning", role: "status" },
      el("strong", { text: "Brief was truncated" }),
      el("p", { text: `${byteCount.toLocaleString()} rendered bytes within a ${maxBytes.toLocaleString()} byte limit.` }),
      omissions.length ? el("ul", { className: "omission-list" }, omissions.map((item) => el("li", { text: omissionText(item) }))) : el("p", { text: "Lower-priority content was omitted." }),
    ));
  }
  root.append(el("div", { className: "brief-meta" },
    el("span", { className: "mono", text: `${(byteCount / 1024).toFixed(1)} KB · ~${Math.round(rendered.length / 4).toLocaleString()} tokens` }),
    el("span", { text: "bounded output · project brief + linked knowledge + dependencies + criteria + context" }),
    el("span", { className: "spacer" }),
    limitForm,
    button("Copy", { className: "button primary", onclick: async () => {
      try { await copyText(rendered); ctx.toast("Brief copied to the clipboard."); }
      catch { ctx.toast("Brief could not be copied.", "error"); }
    } }),
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
