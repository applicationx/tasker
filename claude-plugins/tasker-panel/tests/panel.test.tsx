import { expect, test } from 'claude-code/testing'

// Answers shaped exactly as `tasker ... -o json` prints them.
const CLI: Record<string, unknown> = {
  'list projects': [{ name: 'Demo App', prefix: 'DEMO', path: 'C:/board\\demo-app' }],
  'get project DEMO': {
    name: 'Demo App',
    prefix: 'DEMO',
    path: '\\\\?\\C:\\board\\demo-app',
    workflow: {
      initial_state: 'backlog',
      claim_state: 'in_progress',
      states: {
        backlog: { label: 'Backlog', terminal: false, dependency_satisfied: false },
        cancelled: { label: 'Cancelled', terminal: true, dependency_satisfied: false },
        done: { label: 'Done', terminal: true, dependency_satisfied: true },
        in_progress: { label: 'In Progress', terminal: false, dependency_satisfied: false },
      },
      transitions: { backlog: ['in_progress', 'cancelled'], in_progress: ['backlog', 'done', 'cancelled'], done: ['in_progress'] },
    },
  },
  'list tasks -p DEMO': [
    { id: 'DEMO-1', type: 'story', header: 'Design storage layer', state: 'in_progress', assignee: 'claude', dependency_blocked: false },
    { id: 'DEMO-2', type: 'task', header: 'Implement CLI', state: 'backlog', assignee: null, dependency_blocked: true },
  ],
  'get task DEMO-1': {
    id: 'DEMO-1',
    type: 'story',
    header: 'Design storage layer',
    state: 'in_progress',
    assignee: 'claude',
    acceptance_criteria: [
      { id: 'AC-1', text: 'Schema documented', completed: true },
      { id: 'AC-2', text: 'Atomic writes', completed: false },
    ],
    context: [{ id: 'CTX-1', kind: 'decision', message: 'Chose append-only event files', created_at: '2026-11-03T19:58:50.819Z', created_by: 'claude' }],
  },
  'status DEMO-1': { id: 'DEMO-1', blocked_by: [], blocks: ['DEMO-2'], claimable: false },
}

const PANE = {
  plugin: 'tasker-panel',
  component: 'Pane',
  requestId: 'tasker-board',
  props: {
    title: 'Tasker',
    isFocused: false,
    bodyColumns: 60,
    placement: 'dock',
    scroll: { offset: 0, bodyRows: 40 },
    view: {},
  },
} as const

test('the pane shows the claimed task and the board, on every surface', async ($, on) => {
  const statuses: Array<string | undefined> = []
  on('process.run', (_, e) => {
    const key = e.argv.slice(1, -2).join(' ')
    const answer = CLI[key]
    return {
      value: {
        exitCode: answer === undefined ? 2 : 0,
        stdout: answer === undefined ? '' : JSON.stringify(answer),
        stderr: answer === undefined ? `{"error":{"message":"unknown ${key}"}}` : '',
        isStdoutTruncated: false,
        isStderrTruncated: false,
      },
    }
  })
  on('env.get', () => ({ value: undefined }))
  on('fs.list', () => ({ value: [] }))
  on('ui.status', (_, e) => {
    statuses.push(e.text)
    return { value: undefined }
  })
  on('command.register', () => ({ value: { command: 'tasker-panel' } }))
  on('tool.register', () => ({ value: { tool: 'mcp__tasker-panel__focus' } }))
  on('ui.open', () => ({ value: { isPlaced: true } }))
  on('session.start', (_, e) => ({ cwd: e.cwd }))

  await $.session.start({ cwd: '/', surface: 'desktop', isInteractive: true })
  expect(statuses.at(-1)).toBe('DEMO-1 In Progress · AC 1/2')

  for (const surface of ['terminal', 'desktop', 'vscode', 'mobile'] as const) {
    const ui = await $.ui.mount({ ...PANE, surface })
    expect(await ui.find({ type: 'Text', text: /DEMO-1 Design storage layer/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /AC-2 Atomic writes/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /Chose append-only event files/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /blocks DEMO-2/ })).toBeDefined()
    expect(await ui.find({ key: 'task-DEMO-2' })).toBeDefined()
    await ui.unmount()
  }
})
