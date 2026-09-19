import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const backend = vi.hoisted(() => ({
  getSetupStatus: vi.fn(), setAccessMode: vi.fn(), authorizeScreenCapture: vi.fn(), getView: vi.fn(), refreshInventory: vi.fn(), refreshPrices: vi.fn(),
  marketStatus: vi.fn(), marketSignIn: vi.fn(), marketLinkToken: vi.fn(), marketSignOut: vi.fn(),
  refreshOrders: vi.fn(), removeOrder: vi.fn(), setOrderQuantity: vi.fn(),
  setMarketPresence: vi.fn(), createOrder: vi.fn(),
  collectReport: vi.fn(), collectReportText: vi.fn(),
  getRewardDiagnosticStatus: vi.fn(), startRewardDiagnostic: vi.fn(), stopRewardDiagnostic: vi.fn(),
}))
const overlay = vi.hoisted(() => ({ showRewardOverlay: vi.fn(), hideRewardOverlay: vi.fn() }))
const report = vi.hoisted(() => ({
  copyReport: vi.fn(), saveReport: vi.fn(), openIssue: vi.fn(), ISSUE_URL: 'https://example.com/issues/new',
}))
const windowApi = vi.hoisted(() => ({
  minimizeWindow: vi.fn(),
  toggleMaximizeWindow: vi.fn(),
  closeWindow: vi.fn(),
  readWindowMaximized: vi.fn().mockResolvedValue(false),
  watchWindowResized: vi.fn().mockResolvedValue(() => {}),
}))
vi.mock('./backend', () => backend)
vi.mock('./overlay', () => overlay)
vi.mock('./report', () => report)
vi.mock('./window', () => windowApi)

import App from './App'
import type { AppView, DiagnosticStatus } from './backend'

function makeView(health: AppView['health']): AppView {
  return {
    collection: { items: [], total_entries: 0, snapshot: null },
    reward: { cards: [], best_value_index: null, best_ducat_index: null },
    market_account: {
      link: 'unlinked', orders: [], listed_platinum: 0, flagged: 0, listable: [],
      presence: { status: null, wanted: null, auto: false },
    },
    health,
  }
}

const readyHealth = (): AppView['health'] => ({
  game_reader: { state: 'ready', message: 'ok', last_success: null },
  log_monitor: { state: 'ready', message: 'ok', last_success: null },
  capture: { state: 'ready', message: 'ok', last_success: null },
  catalog: { state: 'ready', message: 'ok', last_success: null },
  market: { state: 'ready', message: 'ok', last_success: null },
  collection_prices: { state: 'ready', message: 'ok', last_success: null },
  database: { state: 'ready', message: 'ok', last_success: null },
  market_account: { state: 'ready', message: 'ok', last_success: null },
  acquisition_stages: [],
})

const diagnostic = (updates: Partial<DiagnosticStatus> = {}): DiagnosticStatus => ({
  available: true, recording: false, samples: 0, build_id: 'issue12-build', message: 'Ready to record.',
  ...updates,
})

async function openDiagnostics() {
  const user = userEvent.setup()
  render(<App />)
  await screen.findByText('TennoScope')
  await user.click(screen.getByRole('button', { name: 'Diagnostics' }))
  return user
}

describe('report block on Diagnostics', () => {
  afterEach(() => { cleanup(); vi.useRealTimers() })
  beforeEach(() => {
    vi.clearAllMocks()
    localStorage.clear()
    backend.getSetupStatus.mockResolvedValue({ setup_complete: true, access_mode: 'full', desktop_capture_action_available: true })
    backend.getView.mockResolvedValue(makeView(readyHealth()))
    backend.marketStatus.mockResolvedValue(makeView(readyHealth()))
    backend.getRewardDiagnosticStatus.mockResolvedValue(diagnostic({ available: false }))
  })

  it('is hidden when every system is ready', async () => {
    await openDiagnostics()
    expect(screen.queryByRole('group', { name: 'Report a problem' })).toBeNull()
  })

  it('is hidden when the only non-ready state is idle', async () => {
    const health = readyHealth()
    health.market_account = { state: 'idle', message: 'not linked', last_success: null }
    backend.getView.mockResolvedValue(makeView(health))
    await openDiagnostics()
    expect(screen.queryByRole('group', { name: 'Report a problem' })).toBeNull()
  })

  it('appears when a system is degraded', async () => {
    const health = readyHealth()
    health.market = { state: 'degraded', message: 'market offline', last_success: '2026-07-27' }
    backend.getView.mockResolvedValue(makeView(health))
    await openDiagnostics()
    expect(screen.getByRole('group', { name: 'Report a problem' })).toBeVisible()
  })

  it('appears when an acquisition stage failed', async () => {
    const health = readyHealth()
    health.acquisition_stages = [
      { stage: 'schema_validation', state: 'failed', message: 'Inventory snapshot was invalid' },
    ]
    backend.getView.mockResolvedValue(makeView(health))
    await openDiagnostics()
    expect(screen.getByRole('group', { name: 'Report a problem' })).toBeVisible()
  })

  it('is hidden when the only broken rows have never worked this session', async () => {
    const health = readyHealth()
    health.game_reader = { state: 'degraded', message: 'waiting', last_success: null }
    health.log_monitor = { state: 'degraded', message: 'EE.log not found', last_success: null }
    backend.getView.mockResolvedValue(makeView(health))
    await openDiagnostics()
    expect(screen.queryByRole('group', { name: 'Report a problem' })).toBeNull()
  })

  it('offers recording only on a diagnostic build, after showing the privacy warning', async () => {
    backend.getRewardDiagnosticStatus.mockResolvedValue(diagnostic())
    backend.startRewardDiagnostic.mockResolvedValue(diagnostic({ recording: true, message: 'Waiting for rewards.' }))
    backend.stopRewardDiagnostic.mockResolvedValue(diagnostic({ samples: 2, message: 'Stopped by you.' }))
    const user = await openDiagnostics()
    const block = await screen.findByRole('group', { name: 'Report a problem' })
    const start = within(block).getByRole('button', { name: 'Record reward diagnostic' })
    expect(start).toHaveAccessibleDescription(/names.*chat.*overlapping windows/i)
    expect(block).toHaveTextContent('issue12-build')
    expect(within(block).queryByRole('button', { name: 'Stop recording' })).toBeNull()

    await user.click(start)
    expect(await within(block).findByRole('button', { name: 'Stop recording' })).toBeEnabled()
    expect(start).toBeDisabled()
    await user.click(within(block).getByRole('button', { name: 'Stop recording' }))
    expect(start).toBeEnabled()
    expect(block).toHaveTextContent(/Stopped.*2 samples/)
  })

  it('does not permit recording in Companion', async () => {
    backend.getSetupStatus.mockResolvedValue({ setup_complete: true, access_mode: 'companion', desktop_capture_action_available: false })
    backend.getRewardDiagnosticStatus.mockResolvedValue(diagnostic())
    await openDiagnostics()
    expect(await screen.findByRole('button', { name: 'Record reward diagnostic' })).toBeDisabled()
  })

  it('reflects a backend auto-stop without another user action', async () => {
    vi.useFakeTimers()
    backend.getRewardDiagnosticStatus.mockResolvedValue(diagnostic({ recording: true, samples: 1 }))
    render(<App />)
    await act(async () => {})
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Diagnostics' })) })
    expect(screen.getByRole('button', { name: 'Stop recording' })).toBeEnabled()
    backend.getRewardDiagnosticStatus.mockResolvedValue(diagnostic({ samples: 3, message: 'Reward screen closed.' }))
    await act(async () => { await vi.advanceTimersByTimeAsync(2500) })
    expect(screen.queryByRole('button', { name: 'Stop recording' })).toBeNull()
    expect(screen.getByRole('button', { name: 'Record reward diagnostic' })).toBeEnabled()
    expect(screen.getByRole('group', { name: 'Report a problem' })).toHaveTextContent(/Stopped.*3 samples/)
  })
  it('does not replace a recording command result with an older status poll', async () => {
    vi.useFakeTimers()
    backend.getRewardDiagnosticStatus.mockResolvedValue(diagnostic())
    render(<App />)
    await act(async () => {})
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Diagnostics' })) })
    let resolvePoll!: (value: DiagnosticStatus) => void
    backend.getRewardDiagnosticStatus.mockImplementationOnce(() => new Promise<DiagnosticStatus>(resolve => { resolvePoll = resolve }))
    await act(async () => { await vi.advanceTimersByTimeAsync(1000) })
    backend.startRewardDiagnostic.mockResolvedValue(diagnostic({ recording: true }))
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Record reward diagnostic' })) })
    await act(async () => { resolvePoll(diagnostic()) })
    expect(screen.getByRole('button', { name: 'Stop recording' })).toBeEnabled()
  })

  it('keeps a rejected start visible without claiming recording began', async () => {
    backend.getRewardDiagnosticStatus.mockResolvedValue(diagnostic())
    backend.startRewardDiagnostic.mockRejectedValue(new Error('Capture access is unavailable.'))
    const user = await openDiagnostics()
    await user.click(await screen.findByRole('button', { name: 'Record reward diagnostic' }))
    expect(screen.getByRole('group', { name: 'Report a problem' })).toHaveTextContent('Capture access is unavailable.')
    expect(screen.queryByRole('button', { name: 'Stop recording' })).toBeNull()
    expect(screen.getByRole('button', { name: 'Record reward diagnostic' })).toBeEnabled()
  })

  it('identifies saved evidence without calling the diagnostic package safe or redacted', async () => {
    backend.getRewardDiagnosticStatus.mockResolvedValue(diagnostic({ samples: 2 }))
    report.saveReport.mockResolvedValue({ folder_path: '/tmp/reports/2026-08-05-141233', report_text: 'x', ee_log_included: true, reward_diagnostic_samples: 2 })
    const user = await openDiagnostics()
    const block = await screen.findByRole('group', { name: 'Report a problem' })
    await user.click(within(block).getByRole('button', { name: 'Save logs' }))
    expect(block).toHaveTextContent('/tmp/reports/2026-08-05-141233')
    expect(block).toHaveTextContent(/2 reward diagnostic samples/i)
    expect(block).toHaveTextContent(/review.*images/i)
    expect(block).not.toHaveTextContent(/safe to attach|package.*redacted/i)
  })

})
