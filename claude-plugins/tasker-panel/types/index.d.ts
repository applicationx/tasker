/** What the person or the agent asked the panel to follow; empty follows the session. */
export type Pin = { root?: string; bin?: string; project?: string; task?: string }

/** The project and task the latest tasker tool calls touched. */
export type Seen = { project?: string; task?: string }

export type BoardTask = {
  id: string
  type: string
  header: string
  state: string
  assignee: string | null
  dependencyBlocked: boolean
}

export type BoardColumn = {
  state: string
  label: string
  terminal: boolean
  tasks: BoardTask[]
}

export type Criterion = { id: string; text: string; completed: boolean }

export type Blocker = { id: string; header: string; state: string; satisfied: boolean }

export type ContextEntry = { id: string; kind: string; message: string; createdAt: string; createdBy: string }

export type FocusTask = {
  id: string
  type: string
  header: string
  state: string
  stateLabel: string
  assignee: string | null
  criteria: Criterion[]
  blockedBy: Blocker[]
  blocks: string[]
  context: ContextEntry[]
  claimable: boolean
}

export type Snapshot = {
  project: { name: string; prefix: string; tasksDir: string } | null
  columns: BoardColumn[]
  focus: FocusTask | null
  error: string | null
  refreshedAt: string
}

declare module 'claude-code' {
  interface PluginState {
    'tasker-panel': {
      snapshot: Shaped<Snapshot | null>
      pin: Shaped<Pin>
      seen: Shaped<Seen>
    }
  }
}
