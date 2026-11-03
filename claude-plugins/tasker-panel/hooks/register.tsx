import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register } from 'claude-code'

import type { BoardColumn, BoardTask, FocusTask, Pin, Seen, Snapshot } from '../types'

// The panel only shells out to the one-shot `tasker` CLI and lists the
// project's tasks/ folder to notice manual edits. It keeps no index: the
// snapshot below is a drawing cache, rebuilt from the CLI on every change.

const PANE = 'tasker-board'
const TITLE = 'Tasker'
const POLL_MS = 2000
const CONTEXT_SHOWN = 4
const COLUMN_SHOWN = 8

const snapshot = atom({ plugin: 'tasker-panel', key: 'snapshot' } as const, null, { shape: 'snapshot-1' })
const pin = atom({ plugin: 'tasker-panel', key: 'pin' } as const, {}, { shape: 'pin-1' })
const seen = atom({ plugin: 'tasker-panel', key: 'seen' } as const, {}, { shape: 'seen-1' })

const ID = /\b([A-Z][A-Z0-9]{1,9})-(\d+)\b/g
// Verbs whose subject is the task the agent is working on, not one it merely created or listed.
const FOCUS_VERBS = /\btasker(?:\.exe)?\s+(?:--root\s+\S+\s+)?(claim|transition|acceptance|context|brief|status|get\s+task)\b/
const FOCUS_TOOLS = /tasker_(claim|transition|acceptance|context_add|brief|status|get_task)$/

let knownPrefixes = new Set<string>()
let lastSignature = ''
let generation = 0

type Json = Record<string, unknown>

class TaskerError extends Error {}

async function tasker($: EngineInterface, args: string[], root?: string): Promise<unknown> {
  const bin = (await read($, pin)).bin ?? (await $.env.get('TASKER_PANEL_BIN')) ?? 'tasker'
  const argv = [bin, ...(root ? ['--root', root] : []), ...args, '-o', 'json']
  const ran = await $.process.run(argv, { timeoutMs: 15000 })
  if (ran.exitCode !== 0) {
    let message = ran.stderr.trim() || `exit ${ran.exitCode}`
    try {
      const error = JSON.parse(message) as { error?: { message?: string } }
      message = error.error?.message ?? message
    } catch {
      // Human-formatted stderr: keep it as is.
    }
    throw new TaskerError(`tasker ${args.join(' ')}: ${message}`)
  }

  return JSON.parse(ran.stdout)
}

function prefixOf(id: string | undefined): string | undefined {
  return id?.replace(/-\d+$/, '')
}

function ids(text: string): string[] {
  return [...text.matchAll(ID)]
    .map(match => match[0])
    .filter(id => {
      const prefix = prefixOf(id) ?? ''
      return knownPrefixes.size > 0 ? knownPrefixes.has(prefix) : prefix !== 'AC' && prefix !== 'CTX'
    })
}

function normalize(path: string): string {
  return path.replace(/^\\\\\?\\/, '').replace(/\\/g, '/')
}

function orderStates(workflow: Json): BoardColumn[] {
  const states = (workflow.states ?? {}) as Record<string, { label?: string; terminal?: boolean; dependency_satisfied?: boolean }>
  const transitions = (workflow.transitions ?? {}) as Record<string, string[]>
  const order: string[] = []
  const queue = [String(workflow.initial_state ?? '')]
  while (queue.length > 0) {
    const state = queue.shift() as string
    if (!(state in states) || order.includes(state)) continue
    order.push(state)
    queue.push(...(transitions[state] ?? []))
  }
  for (const state of Object.keys(states)) if (!order.includes(state)) order.push(state)
  const open = order.filter(state => !states[state]?.terminal)
  const closed = order
    .filter(state => states[state]?.terminal)
    .sort((a, b) => Number(states[b]?.dependency_satisfied ?? false) - Number(states[a]?.dependency_satisfied ?? false))

  return [...open, ...closed].map(state => ({
    state,
    label: states[state]?.label ?? state,
    terminal: states[state]?.terminal ?? false,
    tasks: [],
  }))
}

async function refresh($: EngineInterface): Promise<void> {
  const mine = ++generation
  const { root, bin: _bin, ...pinned } = await read($, pin)
  const lastSeen = await read($, seen)
  let fresh: Snapshot

  try {
    const projects = (await tasker($, ['list', 'projects'], root)) as Array<{ name: string; prefix: string; path: string }>
    knownPrefixes = new Set(projects.map(project => project.prefix))
    const wanted =
      pinned.project ??
      prefixOf(pinned.task) ??
      lastSeen.project ??
      prefixOf(lastSeen.task) ??
      (await $.env.get('TASKER_PROJECT')) ??
      projects[0]?.prefix
    if (wanted === undefined) throw new TaskerError('No Tasker project found from this folder. Run `/tasker-panel <PREFIX>` or `tasker init`.')

    const [project, tasks] = await Promise.all([
      tasker($, ['get', 'project', wanted], root) as Promise<Json>,
      tasker($, ['list', 'tasks', '-p', wanted], root) as Promise<Array<Json>>,
    ])
    const prefix = String(project.prefix)
    const workflow = (project.workflow ?? {}) as Json
    const claimState = String(workflow.claim_state ?? '')
    const columns = orderStates(workflow)
    const board: BoardTask[] = tasks.map(task => ({
      id: String(task.id),
      type: String(task.type ?? 'task'),
      header: String(task.header ?? ''),
      state: String(task.state ?? ''),
      assignee: (task.assignee as string | null) ?? null,
      dependencyBlocked: task.dependency_blocked === true,
    }))
    for (const task of board) {
      let column = columns.find(one => one.state === task.state)
      if (column === undefined) {
        column = { state: task.state, label: task.state, terminal: false, tasks: [] }
        columns.push(column)
      }
      column.tasks.push(task)
    }

    const actor = await $.env.get('TASKER_ACTOR')
    const inProject = (id: string | undefined) => (prefixOf(id) === prefix ? id : undefined)
    const focusId =
      inProject(pinned.task) ??
      inProject(lastSeen.task) ??
      board.find(task => task.state === claimState && actor !== undefined && task.assignee === actor)?.id ??
      board.find(task => task.state === claimState)?.id

    let focus: FocusTask | null = null
    if (focusId !== undefined) {
      const [task, status] = await Promise.all([
        tasker($, ['get', 'task', focusId], root) as Promise<Json>,
        tasker($, ['status', focusId], root) as Promise<Json>,
      ])
      const state = String(task.state)
      focus = {
        id: focusId,
        type: String(task.type ?? 'task'),
        header: String(task.header ?? ''),
        state,
        stateLabel: columns.find(column => column.state === state)?.label ?? state,
        assignee: (task.assignee as string | null) ?? null,
        criteria: ((task.acceptance_criteria ?? []) as Array<Json>).map(criterion => ({
          id: String(criterion.id),
          text: String(criterion.text),
          completed: criterion.completed === true,
        })),
        blockedBy: ((status.blocked_by ?? []) as Array<Json>).map(blocker => ({
          id: String(blocker.id),
          header: String(blocker.header ?? ''),
          state: String(blocker.state ?? ''),
          satisfied: blocker.satisfied === true,
        })),
        blocks: ((status.blocks ?? []) as unknown[]).map(String),
        context: ((task.context ?? []) as Array<Json>).slice(-CONTEXT_SHOWN).reverse().map(entry => ({
          id: String(entry.id),
          kind: String(entry.kind ?? 'note'),
          message: String(entry.message ?? ''),
          createdAt: String(entry.created_at ?? ''),
          createdBy: String(entry.created_by ?? ''),
        })),
        claimable: status.claimable === true,
      }
    }

    const listed = projects.find(one => one.prefix === prefix)
    fresh = {
      project: {
        name: String(project.name),
        prefix,
        tasksDir: `${normalize(listed?.path ?? String(project.path))}/tasks`,
      },
      columns,
      focus,
      error: null,
      refreshedAt: new Date().toISOString(),
    }
  } catch (error) {
    const previous = await read($, snapshot)
    fresh = {
      project: previous?.project ?? null,
      columns: previous?.columns ?? [],
      focus: previous?.focus ?? null,
      error: error instanceof Error ? error.message : String(error),
      refreshedAt: new Date().toISOString(),
    }
  }

  // A newer refresh started meanwhile: its answer wins.
  if (mine !== generation) return
  await update($, snapshot, () => fresh)
  $.ui.status(statusLine(fresh))
}

function acceptance(focus: FocusTask): string {
  const done = focus.criteria.filter(criterion => criterion.completed).length

  return `${done}/${focus.criteria.length}`
}

function statusLine(shot: Snapshot): string | undefined {
  if (shot.focus === null) return shot.project ? `tasker ${shot.project.prefix}: no claimed task` : undefined
  const open = shot.focus.blockedBy.filter(blocker => !blocker.satisfied).length

  return [
    `${shot.focus.id} ${shot.focus.stateLabel}`,
    `AC ${acceptance(shot.focus)}`,
    open > 0 ? `blocked by ${open}` : undefined,
  ]
    .filter(Boolean)
    .join(' · ')
}

function bar(focus: FocusTask, cells: number): string {
  const total = focus.criteria.length
  const done = focus.criteria.filter(criterion => criterion.completed).length
  const filled = total === 0 ? 0 : Math.round((done / total) * cells)

  return '█'.repeat(filled) + '░'.repeat(cells - filled)
}

async function signature($: EngineInterface, dir: string): Promise<string> {
  try {
    const entries = await $.fs.list(dir)
    return entries.map(entry => `${entry.name}:${entry.mtimeMs}:${entry.size}`).sort().join('|')
  } catch {
    return 'missing'
  }
}

function remember(e: { tool: string }, text: string | undefined): Seen | undefined {
  const args = e as unknown as Record<string, unknown>
  const input = JSON.stringify(args)
  const command = typeof args.command === 'string' ? args.command : ''
  const isCli = e.tool === 'Bash' || e.tool === 'PowerShell'
  if (isCli && !/\btasker(?:\.exe)?\b/.test(command)) return undefined
  if (!isCli && !/tasker/i.test(e.tool)) return undefined

  const project = (isCli ? /(?:-p|--project)[\s=]+([A-Z][A-Z0-9]{1,9})\b/.exec(command)?.[1] : /"project":"([A-Z][A-Z0-9]{1,9})"/.exec(input)?.[1]) ?? undefined
  const focuses = isCli ? FOCUS_VERBS.test(command) : FOCUS_TOOLS.test(e.tool)
  let task: string | undefined
  if (focuses) task = ids(isCli ? command : input)[0] ?? ids(/"id":"[^"]+"/.exec(text ?? '')?.[0] ?? '')[0]

  return { project: project ?? prefixOf(task), task }
}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    const started = await next(e)
    await $.command.register({
      name: 'tasker-panel',
      description: 'Show the live Tasker board. Args: [ID | PREFIX | auto] [--root PATH] [--bin PATH]',
    })
    await $.tool.register({
      name: 'focus',
      description:
        'Open the Tasker board panel beside the chat and follow a task or project. Call it once after claiming a Tasker task so the person can watch progress. All fields optional: task (e.g. APP-12), project (prefix), root (board path; only when not discoverable from the working directory), bin (tasker executable; only when `tasker` on PATH is not the one to use).',
      inputSchema: {
        type: 'object',
        properties: {
          task: { type: 'string' },
          project: { type: 'string' },
          root: { type: 'string' },
          bin: { type: 'string' },
        },
      },
    })

    await refresh($)
    const shot = await read($, snapshot)
    // Open unasked only where the pane is a sidebar and a board was found.
    if (shot?.project && e.surface !== null && e.surface !== 'terminal') void $.ui.open({ id: PANE, title: TITLE })

    $.clock.every(POLL_MS, async () => {
      const current = await read($, snapshot)
      if (!current?.project) return
      const latest = await signature($, current.project.tasksDir)
      if (latest === lastSignature) return
      const first = lastSignature === ''
      lastSignature = latest
      if (!first) await refresh($)
    })

    return started
  })

  on('command.run', { command: 'tasker-panel' }, async ($, e) => {
    const words = e.args.trim().split(/\s+/).filter(Boolean)
    const flag = (name: string) => {
      const at = words.indexOf(name)
      return at >= 0 ? words.splice(at, 2)[1] : undefined
    }
    const root = flag('--root')
    const bin = flag('--bin')
    const target = words[0]
    await update($, pin, current => {
      const base: Pin = { root: root ?? current.root, bin: bin ?? current.bin }
      if (target === undefined || target === 'auto') return target === 'auto' ? base : { ...current, ...base }
      return /-\d+$/.test(target) ? { ...base, task: target.toUpperCase() } : { ...base, project: target.toUpperCase() }
    })
    await refresh($)
    await $.ui.open({ id: PANE, title: TITLE })
    const shot = await read($, snapshot)

    return { text: shot?.error ?? `Tasker panel: ${shot?.project?.name ?? 'no project'}${shot?.focus ? `, following ${shot.focus.id}` : ''}.` }
  })

  on('tool.call', { tool: 'mcp__tasker-panel__focus' }, async ($, e) => {
    const input = e as unknown as Pin
    await update($, pin, current => ({
      root: input.root || current.root,
      bin: input.bin || current.bin,
      project: input.project?.toUpperCase() || undefined,
      task: input.task?.toUpperCase() || undefined,
    }))
    await refresh($)
    const { isPlaced } = await $.ui.open({ id: PANE, title: TITLE })
    const shot = await read($, snapshot)
    const text = shot?.error ?? `${isPlaced ? 'Panel open' : 'Panel requested'}: ${statusLine(shot as Snapshot) ?? 'no task'}`

    return { result: text }
  }).catch(() => ({ result: 'The Tasker panel could not refresh; run `/tasker-panel` to retry.' }))

  on('tool.call', async ($, e, next) => {
    const ran = await next(e)
    if (e.tool === 'mcp__tasker-panel__focus') return ran
    const hint = remember(e, ran.text)
    if (hint === undefined) return ran
    await update($, seen, current => ({
      project: hint.project ?? current.project,
      task: hint.task ?? (hint.project && hint.project !== prefixOf(current.task) ? undefined : current.task),
    }))
    // Refresh outside this dispatch so the tool result is not held up by the CLI.
    $.clock.after(1, () => refresh($))

    return ran
  }).catch(($, e, next) => next(e))

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const { Box, Text, Button } = $.ui.resolve(e)
    const shot = await read($, snapshot)
    const pinned = await read($, pin)
    const width = Math.max(24, e.props.bodyColumns ?? 48)
    const follow = (task: string) => async () => {
      await update($, pin, current => ({ ...current, task }))
      await refresh($)
    }
    const unpin = async () => {
      await update($, pin, current => ({ root: current.root, bin: current.bin }))
      await refresh($)
    }

    if (shot === null) return <Text dimColor>Reading the Tasker board…</Text>

    const time = shot.refreshedAt.slice(11, 19)
    const focus = shot.focus

    return (
      <Box flexDirection="column" gap={1}>
        <Box flexDirection="row" justifyContent="space-between" alignItems="center">
          <Text bold wrap="truncate-end">
            {shot.project ? `${shot.project.name} (${shot.project.prefix})` : 'Tasker'}
          </Text>
          <Box flexDirection="row" gap={1}>
            {(pinned.task !== undefined || pinned.project !== undefined) && (
              <Button key="auto" label="follow agent" onPress={unpin} />
            )}
            <Button key="refresh" label="↻" onPress={() => void refresh($)} />
          </Box>
        </Box>
        <Text dimColor>updated {time} UTC</Text>
        {shot.error !== null && <Text color="error">{shot.error}</Text>}

        {focus === null ? (
          <Text dimColor>No claimed task. Claim one with `tasker claim next`, or pick a task below.</Text>
        ) : (
          <Box flexDirection="column" borderStyle="round" borderColor="claude" paddingX={1}>
            <Text bold wrap="wrap">
              {focus.id} {focus.header}
            </Text>
            <Text dimColor>
              {focus.type} · {focus.stateLabel}
              {focus.assignee ? ` · @${focus.assignee}` : ''}
            </Text>
            <Box flexDirection="column" marginTop={1}>
              <Text>
                Acceptance <Text color={focus.criteria.every(c => c.completed) ? 'success' : 'warning'}>{bar(focus, Math.min(20, width - 20))}</Text>{' '}
                {acceptance(focus)}
              </Text>
              {focus.criteria.map(criterion => (
                <Text color={criterion.completed ? 'success' : undefined} dimColor={criterion.completed} wrap="wrap">
                  {criterion.completed ? '✓' : '○'} {criterion.id} {criterion.text}
                </Text>
              ))}
            </Box>
            {(focus.blockedBy.length > 0 || focus.blocks.length > 0) && (
              <Box flexDirection="column" marginTop={1}>
                {focus.blockedBy.map(blocker => (
                  <Text color={blocker.satisfied ? 'success' : 'error'} wrap="truncate-end">
                    {blocker.satisfied ? '✓' : '⛔'} blocked by {blocker.id} {blocker.header} ({blocker.state})
                  </Text>
                ))}
                {focus.blocks.length > 0 && <Text dimColor>blocks {focus.blocks.join(', ')}</Text>}
              </Box>
            )}
            {focus.context.length > 0 && (
              <Box flexDirection="column" marginTop={1}>
                <Text bold>Latest context</Text>
                {focus.context.map(entry => (
                  <Text wrap="wrap">
                    <Text color="suggestion">[{entry.kind}]</Text> {entry.message}{' '}
                    <Text dimColor>
                      {entry.createdBy} {entry.createdAt.slice(11, 16)}
                    </Text>
                  </Text>
                ))}
              </Box>
            )}
          </Box>
        )}

        <Box flexDirection="column">
          {shot.columns
            .filter(column => column.tasks.length > 0)
            .map(column => (
              <Box flexDirection="column" marginBottom={1}>
                <Text bold dimColor={column.terminal}>
                  {column.label} <Text dimColor>{column.tasks.length}</Text>
                </Text>
                {column.tasks.slice(0, column.terminal ? 3 : COLUMN_SHOWN).map(task => (
                  <Button key={`task-${task.id}`} plain onPress={follow(task.id)}>
                    {task.id === focus?.id ? '▶ ' : '  '}
                    {task.id} {task.header}
                    {task.dependencyBlocked ? <Text color="error"> ⛔</Text> : ''}
                    <Text dimColor>
                      {' '}
                      {task.type}
                      {task.assignee ? ` @${task.assignee}` : ''}
                    </Text>
                  </Button>
                ))}
                {column.tasks.length > (column.terminal ? 3 : COLUMN_SHOWN) && (
                  <Text dimColor>  +{column.tasks.length - (column.terminal ? 3 : COLUMN_SHOWN)} more</Text>
                )}
              </Box>
            ))}
        </Box>
      </Box>
    )
  })
}
