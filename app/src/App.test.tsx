import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const backend = vi.hoisted(() => ({
  getSetupStatus: vi.fn(), acceptRiskDisclosure: vi.fn(), getView: vi.fn(), refreshInventory: vi.fn(),
}))
vi.mock('./backend', () => backend)

import App from './App'

const view = {
  collection: { items: [], total_entries: 0 },
  reward: { cards: [], best_value_index: null },
  health: {
    game_reader: { state: 'degraded', message: 'Warframe is not running', last_success: null },
    log_monitor: { state: 'ready', message: 'EE.log monitor ready', last_success: null },
    capture: { state: 'degraded', message: 'Not connected', last_success: null },
    catalog: { state: 'ready', message: 'Catalog ready', last_success: '1' },
    market: { state: 'degraded', message: 'Not connected', last_success: null },
    database: { state: 'ready', message: 'SQLite database available', last_success: null },
    acquisition_stages: [{ stage: 'schema_validation', state: 'failed', message: 'inventory snapshot was invalid' }],
  },
}

describe('App setup flow', () => {
  afterEach(cleanup)
  beforeEach(() => {
    vi.clearAllMocks()
    backend.getView.mockResolvedValue(view)
    backend.refreshInventory.mockResolvedValue(view)
  })

  it('requires the one-time risk disclosure before enabling acquisition', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: false })
    backend.acceptRiskDisclosure.mockResolvedValue({ risk_accepted: true })
    render(<App />)
    expect(await screen.findByRole('heading', { name: 'Read-only game access' })).toBeInTheDocument()
    expect(screen.getByText(/account-policy or anti-cheat risk/i)).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: 'Accept and continue' }))
    expect(backend.acceptRiskDisclosure).toHaveBeenCalledOnce()
    expect(await screen.findByRole('heading', { name: 'Collection' })).toBeInTheDocument()
  })

  it('opens the collection and honest health view after prior acceptance', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    render(<App />)
    expect(await screen.findByRole('heading', { name: 'Collection' })).toBeInTheDocument()
    expect(screen.getByText('Warframe is not running')).toBeInTheDocument()
    expect(screen.getByText('0 items')).toBeInTheDocument()
    expect(screen.getByLabelText('Schema validation health: failed')).toHaveTextContent('inventory snapshot was invalid')
  })

  it('polls for backend changes without overlap and stops after unmount', async () => {
    vi.useFakeTimers()
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    let release: ((value: typeof view) => void) | undefined
    backend.getView
      .mockResolvedValueOnce(view)
      .mockImplementationOnce(() => new Promise(resolve => { release = resolve }))
    const rendered = render(<App />)
    await act(async () => {})
    expect(backend.getView).toHaveBeenCalledTimes(1)
    await act(async () => { await vi.advanceTimersByTimeAsync(2500) })
    expect(backend.getView).toHaveBeenCalledTimes(2)
    await act(async () => { await vi.advanceTimersByTimeAsync(10_000) })
    expect(backend.getView).toHaveBeenCalledTimes(2)
    const changed = { ...view, collection: { items: [], total_entries: 42 } }
    await act(async () => { release?.(changed) })
    expect(screen.getByText('42 items')).toBeInTheDocument()
    rendered.unmount()
    await act(async () => { await vi.advanceTimersByTimeAsync(10_000) })
    expect(backend.getView).toHaveBeenCalledTimes(2)
    vi.useRealTimers()
  })

  it('does not let an older poll overwrite a newer manual refresh', async () => {
    vi.useFakeTimers()
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    let releasePoll: ((value: typeof view) => void) | undefined
    backend.getView.mockResolvedValueOnce(view).mockImplementationOnce(() => new Promise(resolve => { releasePoll = resolve }))
    backend.refreshInventory.mockResolvedValue({ ...view, collection: { items: [], total_entries: 9 } })
    render(<App />)
    await act(async () => {})
    await act(async () => { await vi.advanceTimersByTimeAsync(2500) })
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Refresh inventory' })) })
    expect(screen.getByText('9 items')).toBeInTheDocument()
    await act(async () => { releasePoll?.({ ...view, collection: { items: [], total_entries: 1 } }) })
    expect(screen.getByText('9 items')).toBeInTheDocument()
    vi.useRealTimers()
  })

  it('does not let delayed startup overwrite a newer poll', async () => {
    vi.useFakeTimers()
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    let releaseStartup: ((value: typeof view) => void) | undefined
    backend.getView
      .mockImplementationOnce(() => new Promise(resolve => { releaseStartup = resolve }))
      .mockResolvedValueOnce({ ...view, collection: { items: [], total_entries: 5 } })
    render(<App />)
    await act(async () => {})
    await act(async () => { await vi.advanceTimersByTimeAsync(2500) })
    expect(screen.getByText('5 items')).toBeInTheDocument()
    await act(async () => { releaseStartup?.({ ...view, collection: { items: [], total_entries: 1 } }) })
    expect(screen.getByText('5 items')).toBeInTheDocument()
    vi.useRealTimers()
  })

  it('does not start a scheduled poll while manual refresh is in flight', async () => {
    vi.useFakeTimers()
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    backend.getView.mockResolvedValue(view)
    let releaseManual: ((value: typeof view) => void) | undefined
    backend.refreshInventory.mockImplementationOnce(() => new Promise(resolve => { releaseManual = resolve }))
    render(<App />)
    await act(async () => {})
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Refresh inventory' })) })
    await act(async () => { await vi.advanceTimersByTimeAsync(5000) })
    expect(backend.getView).toHaveBeenCalledTimes(1)
    await act(async () => { releaseManual?.({ ...view, collection: { items: [], total_entries: 8 } }) })
    expect(screen.getByText('8 items')).toBeInTheDocument()
    vi.useRealTimers()
  })

  it('resumes polling after a rejected manual refresh', async () => {
    vi.useFakeTimers()
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    backend.getView.mockResolvedValueOnce(view).mockResolvedValueOnce({ ...view, collection: { items: [], total_entries: 6 } })
    backend.refreshInventory.mockRejectedValueOnce(new Error('synthetic'))
    render(<App />)
    await act(async () => {})
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Refresh inventory' })) })
    await act(async () => { await vi.advanceTimersByTimeAsync(2500) })
    expect(backend.getView).toHaveBeenCalledTimes(2)
    expect(screen.getByText('6 items')).toBeInTheDocument()
    vi.useRealTimers()
  })
})
