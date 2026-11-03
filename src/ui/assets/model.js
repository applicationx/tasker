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

// Configured states in workflow order: breadth-first from the initial state along
// configured transitions, then any unreachable states in key order.
export function stateEntries(snapshot) {
  const workflow = workflowFrom(snapshot);
  const states = workflow.states || {};
  const ordered = [];
  const queue = workflow.initial_state in states ? [workflow.initial_state] : [];
  const seen = new Set(queue);
  while (queue.length) {
    const key = queue.shift();
    ordered.push(key);
    for (const target of workflow.transitions?.[key] || []) {
      if (target in states && !seen.has(target)) { seen.add(target); queue.push(target); }
    }
  }
  const nonTerminal = ordered.filter((key) => !states[key].terminal);
  const terminal = ordered.filter((key) => states[key].terminal);
  const rest = Object.keys(states).filter((key) => !seen.has(key));
  return [...nonTerminal, ...rest, ...terminal].map((key) => [key, states[key]]);
}

const STATE_PALETTE = ["var(--cool)", "var(--bad)", "var(--violet)", "var(--olive)"];

// Colours derive from configured semantics only: initial, claim, terminal and dependency_satisfied.
export function stateColor(snapshot, state) {
  const workflow = workflowFrom(snapshot);
  const config = workflow.states?.[state];
  if (!config) return "var(--faint)";
  if (config.terminal) return config.dependency_satisfied ? "var(--ok)" : "var(--dim-state)";
  if (state === workflow.claim_state) return "var(--accent)";
  if (state === workflow.initial_state) return "var(--neutral-state)";
  const others = stateEntries(snapshot)
    .filter(([key, item]) => !item.terminal && key !== workflow.claim_state && key !== workflow.initial_state)
    .map(([key]) => key);
  return STATE_PALETTE[Math.max(0, others.indexOf(state)) % STATE_PALETTE.length];
}

export function isTerminal(snapshot, state) {
  return Boolean(stateConfig(snapshot, state).terminal);
}

export function userFrom(snapshot, userId) {
  if (!userId) return null;
  return usersFrom(snapshot).find((user) => user.id === userId) || { id: userId, name: userId, kind: "unknown" };
}

export function acceptanceLabel(task, separator = "/") {
  const criteria = Array.isArray(task?.acceptance_criteria) ? task.acceptance_criteria : [];
  if (!criteria.length) return "—";
  return `${criteria.filter((item) => item.completed).length}${separator}${criteria.length}`;
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
