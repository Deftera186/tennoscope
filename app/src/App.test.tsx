import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const backend = vi.hoisted(() => ({
  getSetupStatus: vi.fn(), acceptRiskDisclosure: vi.fn(), getView: vi.fn(), refreshInventory: vi.fn(),
}))
vi.mock('./backend', () => backend)

import App from './App'
import type { AppView } from './backend'

const view: AppView = {
  collection: {
    items: [
      { id: 'rhino', name: 'Rhino', category: 'frame', quantity: 1, mastered: true },
      { id: 'braton', name: 'Braton', category: 'weapon', quantity: 3, mastered: true },
      { id: 'carrier', name: 'Carrier', category: 'companion', quantity: 1, mastered: false },
      { id: 'lex-prime-receiver', name: 'Lex Prime Receiver', category: 'prime_part', quantity: 1, mastered: false },
      { id: 'lith-a1', name: 'Lith A1 Relic', category: 'relic', quantity: 7, mastered: false },
      { id: 'argon-crystal', name: 'Argon Crystal', category: 'resource', quantity: 4, mastered: false },
      { id: 'forma-blueprint', name: 'Forma Blueprint', category: 'blueprint', quantity: 0, mastered: false },
      { id: 'bad-baby', name: 'Bad Baby', category: 'vehicle', quantity: 1, mastered: false },
    ],
    total_entries: 8,
  },
  reward: {
    cards: [
      { name: 'Forma Blueprint', platinum: 12, ducats: 25, owned: 0, mastery_relevant: false, confidence: 1 },
      { name: 'Lex Prime Receiver', platinum: 8, ducats: 15, owned: 1, mastery_relevant: true, confidence: 1 },
      { name: 'Rare Prime Set', platinum: 30, ducats: 100, owned: 0, mastery_relevant: false, confidence: 0.79 },
    ],
    best_value_index: 0,
  },
  health: {
    game_reader: { state: 'degraded', message: 'Warframe is not running', last_success: null },
    log_monitor: { state: 'ready', message: 'EE.log monitor ready', last_success: null },
    capture: { state: 'degraded', message: 'Capture waiting', last_success: null },
    catalog: { state: 'ready', message: 'Catalog ready', last_success: '1' },
    market: { state: 'degraded', message: 'Market offline', last_success: null },
    database: { state: 'ready', message: 'SQLite database available', last_success: null },
    acquisition_stages: [
      { stage: 'process_discovery', state: 'ready', message: 'Game process found' },
      { stage: 'memory_read', state: 'ready', message: 'Readable regions found' },
      { stage: 'authorization_scan', state: 'ready', message: 'Authorization discovered' },
      { stage: 'inventory_fetch', state: 'ready', message: 'Inventory fetched' },
      { stage: 'schema_validation', state: 'failed', message: 'Inventory snapshot was invalid' },
    ],
  },
}

describe('MVP desktop interface', () => {
  afterEach(() => { cleanup(); vi.useRealTimers() })
  beforeEach(() => {
    vi.clearAllMocks()
    backend.getView.mockResolvedValue(view)
    backend.refreshInventory.mockResolvedValue(view)
  })

  it('requires an accessible one-time risk disclosure before enabling acquisition', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: false })
    backend.acceptRiskDisclosure.mockResolvedValue({ risk_accepted: true })
    render(<App />)
    expect(await screen.findByRole('heading', { name: 'Read-only game access' })).toBeInTheDocument()
    expect(screen.getByText(/account-policy or anti-cheat risk/i)).toBeInTheDocument()
    expect(screen.getByText(/never logs or uploads/i)).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: 'Accept risk and continue' }))
    expect(backend.acceptRiskDisclosure).toHaveBeenCalledOnce()
    expect(await screen.findByRole('heading', { name: 'Your collection' })).toBeInTheDocument()
  })

  it('shows useful collection summary and responsive navigation semantics', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    render(<App />)
    expect(await screen.findByRole('heading', { name: 'Your collection' })).toBeInTheDocument()
    expect(screen.getByRole('navigation', { name: 'Primary' })).toBeInTheDocument()
    expect(screen.getByText('8', { selector: '[data-summary="items"] *' })).toBeInTheDocument()
    expect(screen.getByText('2', { selector: '[data-summary="mastered"] *' })).toBeInTheDocument()
    expect(screen.getByRole('list', { name: 'Collection items' })).toHaveClass('collection-grid')
    expect(screen.getByRole('article', { name: 'Rhino' })).toHaveTextContent('Mastered')
  })

  it('filters by search, category, and ownership without losing canonical names', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    render(<App />)
    await screen.findByRole('heading', { name: 'Your collection' })
    const search = screen.getByRole('searchbox', { name: 'Search collection' })
    await userEvent.type(search, 'lex prime')
    expect(screen.getByRole('article', { name: 'Lex Prime Receiver' })).toBeInTheDocument()
    expect(screen.queryByRole('article', { name: 'Rhino' })).not.toBeInTheDocument()
    await userEvent.clear(search)
    await userEvent.click(screen.getByRole('button', { name: 'Prime Part' }))
    expect(screen.getByRole('article', { name: 'Lex Prime Receiver' })).toBeInTheDocument()
    expect(screen.queryByRole('article', { name: 'Braton' })).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: 'All categories' }))
    await userEvent.click(screen.getByRole('button', { name: 'Missing' }))
    expect(screen.getByRole('article', { name: 'Forma Blueprint' })).toHaveTextContent('Missing')
  })

  it('supports every stable category and sortable collection results', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    render(<App />)
    await screen.findByRole('heading', { name: 'Your collection' })
    for (const label of ['Frame', 'Weapon', 'Companion', 'Prime Part', 'Relic', 'Resource', 'Blueprint', 'Vehicle']) {
      expect(screen.getByRole('button', { name: label })).toBeInTheDocument()
    }
    await userEvent.selectOptions(screen.getByRole('combobox', { name: 'Sort collection' }), 'quantity-desc')
    const cards = screen.getAllByRole('article').filter(node => node.closest('[aria-label="Collection items"]'))
    expect(cards[0]).toHaveAccessibleName('Lith A1 Relic')
  })

  it('renders honest loading, empty, and error states', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    let resolveView: ((value: AppView) => void) | undefined
    backend.getView.mockImplementationOnce(() => new Promise(resolve => { resolveView = resolve }))
    render(<App />)
    expect(await screen.findByText('Loading your local collection…')).toBeInTheDocument()
    await act(async () => resolveView?.({ ...view, collection: { items: [], total_entries: 0 }, reward: { cards: [], best_value_index: null } }))
    expect(screen.getByText(/No inventory items yet/i)).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: 'Rewards' }))
    expect(screen.getByText(/No reward choices detected/i)).toBeInTheDocument()
    backend.refreshInventory.mockRejectedValueOnce(new Error('synthetic'))
    await userEvent.click(screen.getByRole('button', { name: 'Refresh inventory' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Inventory refresh failed')
  })

  it('shows all diagnostics and acquisition stages without credential labels', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    render(<App />)
    await screen.findByRole('heading', { name: 'Your collection' })
    await userEvent.click(screen.getByRole('button', { name: 'Diagnostics' }))
    const panel = screen.getByRole('region', { name: 'Diagnostics' })
    for (const label of ['Game reader', 'EE.log', 'Catalog', 'Database', 'Process discovery', 'Memory read', 'Authorization scan', 'Inventory fetch', 'Schema validation']) {
      expect(within(panel).getByText(label)).toBeInTheDocument()
    }
    expect(panel).not.toHaveTextContent(/accountId|nonce|authorization token/i)
  })

  it('renders zero to four reward decisions with value, ownership, and mastery indicators', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    render(<App />)
    await screen.findByRole('heading', { name: 'Your collection' })
    await userEvent.click(screen.getByRole('button', { name: 'Rewards' }))
    const advisor = screen.getByRole('region', { name: 'Reward advisor' })
    expect(within(advisor).getAllByRole('article')).toHaveLength(3)
    expect(within(advisor).getByRole('article', { name: 'Forma Blueprint' })).toHaveTextContent('Best value')
    expect(within(advisor).getByRole('article', { name: 'Lex Prime Receiver' })).toHaveTextContent('Owned ×1')
    expect(within(advisor).getByRole('article', { name: 'Lex Prime Receiver' })).toHaveTextContent('Mastery')
    expect(within(advisor).getByRole('article', { name: 'Rare Prime Set' })).toHaveTextContent('79% confidence')
  })

  it('keeps risk disclosure and local-first details available from settings', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    render(<App />)
    await screen.findByRole('heading', { name: 'Your collection' })
    await userEvent.click(screen.getByRole('button', { name: 'Settings' }))
    expect(screen.getByRole('heading', { name: 'Settings & about' })).toBeInTheDocument()
    expect(screen.getByText(/stored on this device/i)).toBeInTheDocument()
    expect(screen.getByText(/process inspection may carry/i)).toBeInTheDocument()
  })

  it('refreshes inventory and announces live state', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    render(<App />)
    await screen.findByRole('heading', { name: 'Your collection' })
    await userEvent.click(screen.getByRole('button', { name: 'Refresh inventory' }))
    expect(backend.refreshInventory).toHaveBeenCalledOnce()
    expect(screen.getByRole('status')).toHaveTextContent(/Watching|Attention/)
  })

  it('does not let an older poll overwrite a newer manual refresh', async () => {
    vi.useFakeTimers()
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    let releasePoll: ((value: AppView) => void) | undefined
    backend.getView.mockResolvedValueOnce(view).mockImplementationOnce(() => new Promise(resolve => { releasePoll = resolve }))
    backend.refreshInventory.mockResolvedValue({ ...view, collection: { items: [], total_entries: 9 } })
    render(<App />)
    await act(async () => {})
    await act(async () => { await vi.advanceTimersByTimeAsync(2500) })
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Refresh inventory' })) })
    expect(screen.getByText('9', { selector: '[data-summary="items"] *' })).toBeInTheDocument()
    await act(async () => { releasePoll?.({ ...view, collection: { items: [], total_entries: 1 } }) })
    expect(screen.getByText('9', { selector: '[data-summary="items"] *' })).toBeInTheDocument()
  })

  it('does not start a scheduled poll while manual refresh is in flight and resumes after rejection', async () => {
    vi.useFakeTimers()
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    backend.getView.mockResolvedValue(view)
    let rejectManual: ((reason: Error) => void) | undefined
    backend.refreshInventory.mockImplementationOnce(() => new Promise((_resolve, reject) => { rejectManual = reject }))
    render(<App />)
    await act(async () => {})
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Refresh inventory' })) })
    await act(async () => { await vi.advanceTimersByTimeAsync(5000) })
    expect(backend.getView).toHaveBeenCalledTimes(1)
    await act(async () => { rejectManual?.(new Error('synthetic')) })
    await act(async () => { await vi.advanceTimersByTimeAsync(2500) })
    expect(backend.getView).toHaveBeenCalledTimes(2)
  })
})
