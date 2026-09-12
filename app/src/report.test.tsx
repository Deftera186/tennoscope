import { cleanup, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const backend = vi.hoisted(() => ({
  getSetupStatus: vi.fn(), acceptRiskDisclosure: vi.fn(), getView: vi.fn(), refreshInventory: vi.fn(), refreshPrices: vi.fn(),
  marketStatus: vi.fn(), marketSignIn: vi.fn(), marketLinkToken: vi.fn(), marketSignOut: vi.fn(),
  refreshOrders: vi.fn(), removeOrder: vi.fn(), setOrderQuantity: vi.fn(),
  setMarketPresence: vi.fn(), createOrder: vi.fn(),
  collectReport: vi.fn(), collectReportText: vi.fn(),
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
import type { AppView } from './backend'

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

async function openDiagnostics() {
  const user = userEvent.setup()
  render(<App />)
  await screen.findByText('TennoScope')
  await user.click(screen.getByRole('button', { name: 'Diagnostics' }))
  return user
}

describe('report block on Diagnostics', () => {
  afterEach(() => { cleanup() })
  beforeEach(() => {
    vi.clearAllMocks()
    localStorage.clear()
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    backend.getView.mockResolvedValue(makeView(readyHealth()))
    backend.marketStatus.mockResolvedValue(makeView(readyHealth()))
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

  it('copy shows the COPIED — PASTE IT INTO THE DIAGNOSTICS FIELD status', async () => {
    const health = readyHealth()
    health.market = { state: 'failed', message: 'market offline', last_success: null }
    backend.getView.mockResolvedValue(makeView(health))
    report.copyReport.mockResolvedValue(undefined)
    const user = await openDiagnostics()
    await user.click(within(screen.getByRole('group', { name: 'Report a problem' })).getByRole('button', { name: 'Copy diagnostics' }))
    expect(report.copyReport).toHaveBeenCalledOnce()
    expect(screen.getByText('COPIED — PASTE IT INTO THE DIAGNOSTICS FIELD OF THE ISSUE FORM.')).toBeVisible()
  })

  it('save shows the folder path and the sanitized note when EE.log is included', async () => {
    const health = readyHealth()
    health.acquisition_stages = [{ stage: 'schema_validation', state: 'failed', message: 'bad' }]
    backend.getView.mockResolvedValue(makeView(health))
    report.saveReport.mockResolvedValue({ folder_path: '/tmp/reports/2026-08-05-141233', report_text: 'x', ee_log_included: true })
    const user = await openDiagnostics()
    await user.click(within(screen.getByRole('group', { name: 'Report a problem' })).getByRole('button', { name: 'Save logs' }))
    expect(report.saveReport).toHaveBeenCalledOnce()
    expect(screen.getByText(/SAVED TO \/tmp\/reports\/2026-08-05-141233/)).toBeVisible()
    expect(screen.getByText(/EE\.LOG INCLUDED \(SANITIZED\) — SAFE TO ATTACH TO THE ISSUE\./)).toBeVisible()
  })

  it('open issue calls openIssue', async () => {
    const health = readyHealth()
    health.catalog = { state: 'failed', message: 'catalog missing', last_success: null }
    backend.getView.mockResolvedValue(makeView(health))
    report.openIssue.mockResolvedValue(undefined)
    const user = await openDiagnostics()
    await user.click(within(screen.getByRole('group', { name: 'Report a problem' })).getByRole('button', { name: 'Open an issue' }))
    expect(report.openIssue).toHaveBeenCalledOnce()
  })
})
