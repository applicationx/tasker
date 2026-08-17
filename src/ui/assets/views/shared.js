import {
  badge, button, conflictBanner, copyText, draftText, el, inlineError, removeInlineError,
  revisionBadge, setPending, tagList, timeElement, toneFor,
} from "../dom.js";
import { assigneeName, stateEntries, stateLabel } from "../model.js";
import { taskHref } from "../router.js";

export function statusBadge(snapshot, state) {
  const keys = stateEntries(snapshot).map(([key]) => key);
  return badge(stateLabel(snapshot, state), toneFor(state, keys));
}

export function taskCard(ctx, record) {
  const { task, status = {} } = record;
  return el("a", { className: "card card-link task-card", href: taskHref(ctx.prefix, task.id) },
    el("span", { className: "identifier", text: task.id }),
    el("h3", { text: task.header }),
    tagList(task.tags),
    el("div", { className: "task-card-footer" },
      el("span", { text: assigneeName(ctx.snapshot, task.assignee) }),
      status.dependency_blocked ? badge("Blocked", "blocked") : statusBadge(ctx.snapshot, task.state),
    ),
  );
}

export function mutationUnavailable(ctx) {
  if (ctx.actor()) return null;
  return el("div", { className: "notice warning", role: "status" },
    "Choose an existing project user in “Acting as” before making changes. Identities are attribution, not authentication.",
  );
}

export async function submitMutation(ctx, form, payload, fields, successMessage) {
  form.querySelector("[data-conflict-banner]")?.remove();
  removeInlineError(form);
  setPending(form, true);
  try {
    await ctx.mutate({ ...payload, actor: ctx.actor() });
    const draftsRemain = ctx.markSaved(form);
    ctx.toast(successMessage);
    if (!draftsRemain) await ctx.refresh({ mutation: true, force: true });
    return true;
  } catch (error) {
    if (error?.code === "revision_conflict" || error?.status === 409) {
      const banner = conflictBanner(error, {
        reload: () => {
          ctx.setDirty(false);
          ctx.refresh({ force: true });
        },
        copy: async () => {
          try {
            await copyText(draftText(fields));
            ctx.toast("Draft copied to the clipboard.");
          } catch {
            inlineError(form, { message: "The draft could not be copied. Select the fields and copy them manually." });
          }
        },
      });
      form.prepend(banner);
      ctx.setDirty(true);
      banner.querySelector("button")?.focus();
    } else {
      inlineError(form, error);
    }
    return false;
  } finally {
    if (form.isConnected) setPending(form, false);
  }
}

export function validationNotice(validation) {
  const errors = Array.isArray(validation?.errors) ? validation.errors : [];
  const warnings = Array.isArray(validation?.warnings) ? validation.warnings : [];
  if (!errors.length && !warnings.length && validation?.valid !== false) return null;
  const message = errors.length
    ? `${errors.length} validation ${errors.length === 1 ? "error needs" : "errors need"} attention. Some views or changes may be unavailable.`
    : `${warnings.length || 1} project validation warning${warnings.length === 1 ? "" : "s"}.`;
  return el("div", { className: `notice ${errors.length ? "error" : "warning"}`, role: errors.length ? "alert" : "status" },
    el("strong", { text: errors.length ? "Project validation failed" : "Validation warning" }),
    el("p", { text: message }),
  );
}

export function metadataList(entries) {
  const list = el("dl", { className: "metadata-list" });
  for (const [term, value] of entries) {
    list.append(el("dt", { text: term }), el("dd", {}, value instanceof Node ? value : String(value ?? "—")));
  }
  return list;
}

export function recordMetadata(record, options = {}) {
  const entries = [
    ["Revision", revisionBadge(record.revision)],
    ["Created", record.created_at ? timeElement(record.created_at) : "—"],
    ["Created by", record.created_by || "—"],
    ["Updated", record.updated_at ? timeElement(record.updated_at) : "—"],
    ["Updated by", record.updated_by || "—"],
  ];
  if (options.prepend) entries.unshift(...options.prepend);
  return metadataList(entries);
}

export function relationshipList(ctx, references) {
  if (!Array.isArray(references) || !references.length) return null;
  return el("ul", { className: "reference-list" }, references.map((reference) => {
    const taskId = reference.task_id || reference.id || reference.source_task_id;
    return el("li", {},
      el("div", { className: "list-main" },
        taskId ? el("a", { href: taskHref(ctx.prefix, taskId), text: taskId }) : el("span", { text: "Task reference" }),
        el("div", { className: "list-meta" }, reference.role || reference.kind || "linked"),
      ),
    );
  }));
}

export function tagsInputValue(tags) {
  return Array.isArray(tags) ? tags.join(", ") : "";
}

export function parseTags(value) {
  return String(value).split(/[\n,]/).map((tag) => tag.trim().toLowerCase()).filter(Boolean).sort().filter((tag, index, values) => index === 0 || tag !== values[index - 1]);
}

export function dirtyForm(form, ctx) {
  form.dataset.dirty = "false";
  const listener = () => {
    form.dataset.dirty = "true";
    ctx.setDirty(true);
  };
  form.addEventListener("input", listener);
  form.addEventListener("change", listener);
  return () => {
    form.removeEventListener("input", listener);
    form.removeEventListener("change", listener);
  };
}
