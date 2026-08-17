export function listFrom(value, key) {
  if (Array.isArray(value)) return value;
  if (Array.isArray(value?.[key])) return value[key];
  if (Array.isArray(value?.items)) return value.items;
  return [];
}

export function projectsFrom(value) {
  return listFrom(value, "projects");
}

export function projectFrom(snapshot) {
  return snapshot?.project || snapshot?.project_view || snapshot?.config || {};
}

export function workflowFrom(snapshot) {
  const project = projectFrom(snapshot);
  return project.workflow || project.configuration?.workflow || project.configuration || snapshot?.workflow || { states: {}, transitions: {} };
}

export function stateEntries(snapshot) {
  return Object.entries(workflowFrom(snapshot).states || {});
}

export function stateConfig(snapshot, state) {
  return workflowFrom(snapshot).states?.[state] || { label: state, terminal: false, dependency_satisfied: false };
}

export function stateLabel(snapshot, state) {
  return stateConfig(snapshot, state).label || state || "Unknown";
}

export function taskRecords(snapshot) {
  return listFrom(snapshot?.tasks, "tasks").map((item) => {
    if (item?.task) return { task: item.task, status: item.status || item.task_status || {} };
    return { task: item, status: item?.status_view || {} };
  }).filter((item) => item.task?.id);
}

export function taskRecord(snapshot, taskId) {
  return taskRecords(snapshot).find(({ task }) => task.id === taskId);
}

export function normalizeTaskDetail(value, snapshot, taskId) {
  const fallback = taskRecord(snapshot, taskId) || {};
  const task = value?.task || value?.data?.task || (value?.id ? value : fallback.task);
  const status = value?.status || value?.task_status || value?.status_view || fallback.status || {};
  return { task, status };
}

export function usersFrom(snapshot) {
  return listFrom(snapshot?.users, "users").filter((user) => user?.id);
}

export function resourcesFrom(snapshot) {
  return listFrom(snapshot?.resources, "resources").filter((resource) => resource?.id);
}

export function decisionsFrom(snapshot) {
  return listFrom(snapshot?.decisions, "decisions").filter((decision) => decision?.id);
}

export function eventsFrom(value) {
  return listFrom(value, "events");
}

export function validationFrom(snapshot) {
  return snapshot?.validation || { valid: true, errors: [], warnings: [] };
}

export function sourceType(resource) {
  return resource?.source_type || (resource?.source_path ? "referenced" : "managed");
}

export function normalizeResource(value) {
  return value?.resource || value?.document || value?.data?.resource || value || {};
}

export function normalizeDecision(value) {
  return value?.decision || value?.document || value?.data?.decision || value || {};
}

export function normalizeProjectBrief(value) {
  if (typeof value === "string") return { content: value };
  return value?.project_brief || value || {};
}

export function reverseReferences(snapshot, id) {
  const map = snapshot?.reverse_context_refs || snapshot?.reverse_references || {};
  return Array.isArray(map[id]) ? map[id] : [];
}

export function assigneeName(snapshot, assignee) {
  if (!assignee) return "Unassigned";
  const user = usersFrom(snapshot).find((item) => item.id === assignee);
  return user?.name ? `${user.name} (${user.id})` : assignee;
}

export function projectPrefix(snapshot, fallback = "") {
  return projectFrom(snapshot).prefix || projectFrom(snapshot).id || fallback;
}

export function projectName(snapshot, fallback = "Project") {
  return projectFrom(snapshot).name || fallback;
}

export function totalProjectStats(project) {
  const stats = project?.stats || project?.task_counts || {};
  return Object.values(stats).reduce((total, count) => total + (Number(count) || 0), 0);
}
