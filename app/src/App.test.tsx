import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const backend = vi.hoisted(() => ({
  getSetupStatus: vi.fn(), acceptRiskDisclosure: vi.fn(), getView: vi.fn(), refreshInventory: vi.fn(), refreshPrices: vi.fn(),
  marketStatus: vi.fn(), marketSignIn: vi.fn(), marketLinkToken: vi.fn(), marketSignOut: vi.fn(),
  refreshOrders: vi.fn(), removeOrder: vi.fn(), setOrderQuantity: vi.fn(),
}))
const overlay = vi.hoisted(() => ({ showRewardOverlay: vi.fn(), hideRewardOverlay: vi.fn() }))
vi.mock('./backend', () => backend)
vi.mock('./overlay', () => overlay)

import App from './App'
import type { AppView } from './backend'

const view: AppView = {
  collection: {
    items: [
      { id: 'rhino', name: 'Rhino', category: 'frame', quantity: 1, mastered: true, live: false, priceable: true },
      { id: 'braton', name: 'Braton', category: 'weapon', quantity: 3, mastered: true, live: false, priceable: true },
      { id: 'carrier', name: 'Carrier', category: 'companion', quantity: 1, mastered: false, live: false, priceable: true },
      { id: 'lex-prime-receiver', name: 'Lex Prime Receiver', category: 'prime_part', quantity: 1, mastered: false, platinum: 19, live: false, priceable: true, monthly_trades: 4 },
      { id: 'lith-a1', name: 'Lith A1 Relic', category: 'relic', quantity: 7, mastered: false, platinum: 20, live: true, priceable: true, monthly_trades: 3 },
      { id: 'argon-crystal', name: 'Argon Crystal', category: 'resource', quantity: 4, mastered: false, live: false, priceable: true },
      { id: 'forma-blueprint', name: 'Forma Blueprint', category: 'blueprint', quantity: 0, mastered: false, live: false, priceable: false },
      // Owned, and no name rule reaches a warframe.market listing for it. The page control must
      // leave it out: counting it promised a price for an item no request is ever made about.
      { id: 'bad-baby', name: 'Bad Baby', category: 'vehicle', quantity: 1, mastered: false, live: false, priceable: false },
      // Priced at exactly 0 -- a real, tradeable price, distinct from an item with no listing at
      // all. Exercises the `?? -1` sentinel in the value sort: a `?? 0` bug would tie this with
      // every unpriced item instead of ranking it above all of them.
      { id: 'zenith-prime-receiver', name: 'Zenith Prime Receiver', category: 'prime_part', quantity: 1, mastered: false, platinum: 0, live: false, priceable: true },
    ],
    total_entries: 8,
  },
  reward: {
    cards: [
      { name: 'Forma Blueprint', platinum: 12, ducats: 25, owned: 0, mastery_relevant: false, confidence: 1 },
      { name: 'Lex Prime Receiver', platinum: 8, ducats: 15, owned: 1, mastery_relevant: true, confidence: 1 },
      { name: 'Rare Prime Set', platinum: 30, ducats: 100, owned: 0, mastery_relevant: false, confidence: 0.79 },
      { name: 'Paris Prime String', platinum: 6, ducats: 45, owned: 1, mastery_relevant: false, confidence: 1 },
    ],
    best_value_index: 0,
    best_ducat_index: 3,
  },
  market_account: {
    link: 'unlinked',
    orders: [],
    listed_platinum: 0,
    listable: [],
    presence: { status: null, auto: false },
    flagged: 0,
  },
  health: {
    game_reader: { state: 'degraded', message: 'Warframe is not running', last_success: null },
    log_monitor: { state: 'ready', message: 'EE.log monitor ready', last_success: null },
    capture: { state: 'degraded', message: 'Capture waiting', last_success: null },
    catalog: { state: 'ready', message: 'Catalog ready', last_success: '1' },
    market: { state: 'degraded', message: 'Market offline', last_success: null },
    collection_prices: { state: 'ready', message: 'Priced from the 2026-07-27 price dump (3 items)', last_success: '2026-07-27' },
    database: { state: 'ready', message: 'SQLite database available', last_success: null },
    market_account: { state: 'idle', message: 'Not linked', last_success: null },
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
    localStorage.clear()  // The price floor outlives a render, which is the point of it.
    backend.getView.mockResolvedValue(view)
    backend.refreshInventory.mockResolvedValue(view)
    backend.refreshPrices.mockResolvedValue(view)
    backend.marketStatus.mockResolvedValue(view)
    backend.marketSignIn.mockResolvedValue(view)
    backend.marketLinkToken.mockResolvedValue(view)
    backend.marketSignOut.mockResolvedValue(view)
    backend.refreshOrders.mockResolvedValue(view)
    backend.removeOrder.mockResolvedValue(view)
    backend.setOrderQuantity.mockResolvedValue(view)
    overlay.showRewardOverlay.mockResolvedValue(undefined)
    overlay.hideRewardOverlay.mockResolvedValue(undefined)
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
    expect(screen.getByText('50% of mastery-eligible items')).toBeInTheDocument()
    expect(screen.getByRole('list', { name: 'Collection items' })).toHaveClass('collection-grid')
    expect(screen.getByRole('article', { name: 'Rhino' })).toHaveTextContent('Mastered')
  })

  // Three copies of a mod at three ranks are three holdings at three prices. The cards must be
  // tellable apart by name alone, and the one nobody quotes must read as bracketed rather than
  // borrow either end.
  it('draws a rank per card and brackets the rank the market does not quote', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    backend.getView.mockResolvedValue({
      ...view,
      collection: {
        total_entries: 3,
        items: [
          { id: 'serration', name: 'Serration', category: 'mod', quantity: 3, mastered: false, platinum: 3, live: false, priceable: true },
          { id: 'serration#7', name: 'Serration', category: 'mod', quantity: 1, mastered: false, platinum: 3, platinum_ceiling: 48, rank: 7, max_rank: 10, live: false, priceable: true },
          { id: 'serration#10', name: 'Serration', category: 'mod', quantity: 1, mastered: false, platinum: 48, rank: 10, max_rank: 10, live: false, priceable: true },
        ],
      },
    })
    render(<App />)
    await screen.findByRole('heading', { name: 'Your collection' })

    expect(screen.getByRole('article', { name: 'Serration' })).toHaveTextContent('Owned ×3')
    expect(screen.getByRole('article', { name: 'Serration, Rank 7/10' })).toHaveTextContent('3–48')
    const maxed = screen.getByRole('article', { name: 'Serration, Rank 10/10' })
    expect(maxed).toHaveTextContent('48')
    expect(within(maxed).getByText('Rank 10/10')).toHaveClass('maxed')
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
    for (const label of ['Frame', 'Weapon', 'Companion', 'Prime Part', 'Relic', 'Resource', 'Blueprint', 'Vehicle', 'Mod', 'Arcane']) {
      expect(screen.getByRole('button', { name: label })).toBeInTheDocument()
    }
    await userEvent.click(within(screen.getByRole('group', { name: 'Sort collection' })).getByRole('button', { name: 'Quantity' }))
    const cards = screen.getAllByRole('article').filter(node => node.closest('[aria-label="Collection items"]'))
    expect(cards[0]).toHaveAccessibleName('Lith A1 Relic')
  })

  it('renders honest loading, empty, and error states', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    let resolveView: ((value: AppView) => void) | undefined
    backend.getView.mockImplementationOnce(() => new Promise(resolve => { resolveView = resolve }))
    render(<App />)
    expect(await screen.findByText('Loading your local collection…')).toBeInTheDocument()
    await act(async () => resolveView?.({ ...view, collection: { items: [], total_entries: 0 }, reward: { cards: [], best_value_index: null, best_ducat_index: null } }))
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
    for (const label of ['Game reader', 'EE.log', 'Reward observer', 'Catalog', 'Market data', 'Database', 'Process discovery', 'Memory read', 'Authorization scan', 'Inventory fetch', 'Schema validation']) {
      expect(within(panel).getByText(label)).toBeInTheDocument()
    }
    // Rows keep their success time in their own source's format -- the market account writes Unix
    // seconds -- so the row resolves it to something a reader can act on rather than printing the
    // stamp it was handed.
    expect(within(panel).getAllByText(/Last success: .*\d{4}/).length).toBeGreaterThan(0)
    expect(within(panel).queryByText('Last success: 1')).not.toBeInTheDocument()
    expect(panel).not.toHaveTextContent(/accountId|nonce|authorization token/i)
    // Diagnostics reports live health; the overlay preview is a setup affordance and lives in Settings.
    expect(within(panel).queryByRole('button', { name: /reward overlay/i })).not.toBeInTheDocument()
  })

  it('renders zero to four reward decisions with value, ownership, and mastery indicators', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    render(<App />)
    await screen.findByRole('heading', { name: 'Your collection' })
    await userEvent.click(screen.getByRole('button', { name: 'Rewards' }))
    const advisor = screen.getByRole('region', { name: 'Reward advisor' })
    expect(within(advisor).getAllByRole('article')).toHaveLength(4)
    expect(within(advisor).getByRole('article', { name: 'Forma Blueprint' })).toHaveTextContent('Top plat')
    expect(within(advisor).getByRole('article', { name: 'Lex Prime Receiver' })).toHaveTextContent('Owned ×1')
    expect(within(advisor).getByRole('article', { name: 'Lex Prime Receiver' })).toHaveTextContent('Mastery needed')
    // Paris Prime String carries the most ducats while Forma Blueprint is worth the most platinum:
    // both have to be callable, because the player picks between them for reasons we cannot see.
    expect(within(advisor).getByRole('article', { name: 'Paris Prime String' })).toHaveTextContent('Top ducats')
    expect(within(advisor).getByRole('article', { name: 'Rare Prime Set' })).toHaveTextContent('Uncertain ·')
    expect(within(advisor).getByRole('article', { name: 'Rare Prime Set' })).not.toHaveTextContent('Top plat')

    // Each reading carries the game's own currency icon beside it, so the two columns are told
    // apart by the shape the player already knows and not only by a hue and a 9px word.
    const card = within(advisor).getByRole('article', { name: 'Forma Blueprint' })
    expect(card.querySelectorAll('[data-metal="plat"]')).toHaveLength(1)
    expect(card.querySelectorAll('[data-metal="ducat"]')).toHaveLength(1)
  })

  it('separates the controls from the notices, and keeps both reachable', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    render(<App />)
    await screen.findByRole('heading', { name: 'Your collection' })

    // Settings holds what changes behaviour. The overlay preview is a control, not a notice, so it
    // moved here off the page of standing statements.
    await userEvent.click(screen.getByRole('button', { name: 'Settings' }))
    expect(screen.getByRole('heading', { name: 'Settings' })).toBeInTheDocument()
    expect(screen.queryByText(/process inspection may carry/i), 'a disclosure is not a preference').not.toBeInTheDocument()
    // A preview you cannot dismiss is a trap: the same control has to put it away.
    await userEvent.click(screen.getByRole('button', { name: 'Preview reward overlay' }))
    expect(overlay.showRewardOverlay).toHaveBeenCalledOnce()
    await userEvent.click(screen.getByRole('button', { name: 'Hide reward overlay' }))
    expect(overlay.hideRewardOverlay).toHaveBeenCalledOnce()

    await userEvent.click(screen.getByRole('button', { name: 'About' }))
    expect(screen.getByText(/stored on this device/i)).toBeInTheDocument()
    expect(screen.getByText(/process inspection may carry/i)).toBeInTheDocument()
    expect(screen.queryByRole('slider'), 'and a preference is not a disclosure').not.toBeInTheDocument()
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

  // A page refresh is on the wire for about sixteen seconds, and it is only bearable because the
  // prices appear as they land. That is the poll's doing, so the poll has to keep running -- unlike
  // an inventory refresh, which replaces the whole collection and does pause it.
  it('keeps polling while a live price refresh is in flight, so prices appear as they land', async () => {
    vi.useFakeTimers()
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    backend.getView.mockResolvedValue(view)
    backend.refreshPrices.mockImplementationOnce(() => new Promise(() => {}))
    render(<App />)
    await act(async () => {})
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: /Price these/ })) })
    await act(async () => { await vi.advanceTimersByTimeAsync(5000) })
    expect(backend.getView).toHaveBeenCalledTimes(3)
  })

  // The readout belongs to the backend, which is the only party that knows a pass's total.
  it('reports a live pricing pass from the backend, whoever asked for it', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    backend.getView.mockResolvedValue({ ...view, collection: { ...view.collection, pricing: { done: 12, total: 65 } } })
    render(<App/>)

    expect(await screen.findByText(/Checking live prices · 12 of 65/)).toBeInTheDocument()
  })

  // Every pass comes out of one three-requests-a-second budget, so letting a click overlap one
  // already running only makes each slower and leaves the one readout describing two queues.
  it('refuses a page refresh while a background pass is already spending requests', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    backend.getView.mockResolvedValue({ ...view, collection: { ...view.collection, pricing: { done: 3, total: 65 } } })
    render(<App/>)

    expect(await screen.findByRole('button', { name: /Pricing/ })).toBeDisabled()
  })

  it('stops scheduled polling after unmount', async () => {
    vi.useFakeTimers()
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    let release: ((value: AppView) => void) | undefined
    backend.getView.mockResolvedValueOnce(view).mockImplementationOnce(() => new Promise(resolve => { release = resolve }))
    const rendered = render(<App />)
    await act(async () => {})
    await act(async () => { await vi.advanceTimersByTimeAsync(2500) })
    expect(backend.getView).toHaveBeenCalledTimes(2)
    rendered.unmount()
    await act(async () => { release?.(view); await vi.advanceTimersByTimeAsync(10_000) })
    expect(backend.getView).toHaveBeenCalledTimes(2)
  })

  it('does not let delayed startup overwrite a newer poll', async () => {
    vi.useFakeTimers()
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    let releaseStartup: ((value: AppView) => void) | undefined
    backend.getView
      .mockImplementationOnce(() => new Promise(resolve => { releaseStartup = resolve }))
      .mockResolvedValueOnce({ ...view, collection: { items: [], total_entries: 5 } })
    render(<App />)
    await act(async () => {})
    await act(async () => { await vi.advanceTimersByTimeAsync(2500) })
    expect(screen.getByText('5', { selector: '[data-summary="items"] *' })).toBeInTheDocument()
    await act(async () => { releaseStartup?.({ ...view, collection: { items: [], total_entries: 1 } }) })
    expect(screen.getByText('5', { selector: '[data-summary="items"] *' })).toBeInTheDocument()
  })

  it('counts mastery only across mastery-eligible categories', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    backend.getView.mockResolvedValue({
      ...view,
      collection: {
        total_entries: 8,
        items: [
          { id: 'frame', name: 'Frame', category: 'frame', quantity: 1, mastered: true, live: false },
          { id: 'weapon', name: 'Weapon', category: 'weapon', quantity: 1, mastered: false, live: false },
          { id: 'companion', name: 'Companion', category: 'companion', quantity: 1, mastered: false, live: false },
          { id: 'vehicle', name: 'Vehicle', category: 'vehicle', quantity: 1, mastered: true, live: false },
          { id: 'part', name: 'Part', category: 'prime_part', quantity: 1, mastered: false, live: false },
          { id: 'resource', name: 'Resource', category: 'resource', quantity: 1, mastered: false, live: false },
          { id: 'mod', name: 'Mod', category: 'mod', quantity: 12, mastered: false, live: false },
          { id: 'arcane', name: 'Arcane', category: 'arcane', quantity: 3, mastered: false, live: false },
        ],
      },
    })
    render(<App />)
    expect(await screen.findByText('50% of mastery-eligible items')).toBeInTheDocument()
  })

  it('renders canonical artwork, sync freshness, and only one 48-item page', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    backend.getView.mockResolvedValue({
      ...view,
      collection: {
        total_entries: 60,
        snapshot: { observed_at: '2026-07-25T11:56:00Z', game_build: 'build-42', source: 'warframe-memory' },
        items: Array.from({ length: 60 }, (_, index) => ({
          id: `item-${index.toString().padStart(2, '0')}`,
          name: `Item ${index.toString().padStart(2, '0')}`,
          category: 'weapon' as const,
          quantity: 1,
          mastered: false,
          live: false,
          image_url: index === 0 ? 'https://cdn.warframestat.us/img/Braton.png' : undefined,
        })),
      },
    })
    render(<App />)

    expect(await screen.findByText(/Synced/)).toHaveAttribute('title', expect.stringContaining('warframe-memory'))
    expect(screen.getByRole('img', { name: 'Item 00' })).toHaveAttribute('src', 'https://cdn.warframestat.us/img/Braton.png')
    expect(screen.getAllByRole('article').filter(node => node.closest('[aria-label="Collection items"]'))).toHaveLength(48)
    expect(screen.getByText('1–48 of 60')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: 'Go to page 2' }))
    expect(screen.getByRole('article', { name: 'Item 59' })).toBeInTheDocument()
    expect(screen.queryByRole('article', { name: 'Item 00' })).not.toBeInTheDocument()
  })

  it('shows the unit price, and the stack total only when more than one is owned', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    render(<App/>)
    const single = await screen.findByRole('article', { name: 'Lex Prime Receiver' })
    expect(within(single).getByText('19')).toBeInTheDocument()
    // The currency is the game's own icon rather than a trailing letter, so it has to still say
    // "platinum" to a reader who cannot see it.
    expect(within(single).getByAltText(/platinum/i)).toBeInTheDocument()
    expect(within(single).queryByText(/total/)).not.toBeInTheDocument()

    const stack = await screen.findByRole('article', { name: 'Lith A1 Relic' })
    expect(within(stack).getByText('20')).toBeInTheDocument()
    expect(within(stack).getByText('140 total')).toBeInTheDocument()
    // One mark leads the pair: the unit price and the stack total are the same currency.
    expect(within(stack).getAllByAltText(/platinum/i)).toHaveLength(1)
  })

  it('says nothing rather than zero for an item with no price', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    render(<App/>)
    const unpriced = await screen.findByRole('article', { name: 'Rhino' })
    expect(within(unpriced).queryByText(/p$/)).not.toBeInTheDocument()
  })

  // The badge said "LIVE" with nothing on screen to explain it. A date explains itself.
  it('states where the daily prices came from', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    render(<App/>)
    expect(await screen.findByText(/27 Jul/)).toBeInTheDocument()
  })

  it('marks a card checked live with its freshness, not a badge', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    render(<App/>)
    const live = await screen.findByRole('article', { name: 'Lith A1 Relic' })
    const daily = await screen.findByRole('article', { name: 'Lex Prime Receiver' })

    expect(within(live).getByText(/checked/i)).toBeInTheDocument()
    expect(within(live).queryByText('Live')).not.toBeInTheDocument()
    expect(within(daily).queryByText(/checked/i)).not.toBeInTheDocument()
  })

  // Someone who clicks it should not have to guess whether it prices the page or the collection --
  // or find that two of the items it counted were never going to be asked about. The fixture's
  // visible page carries eight owned items; the quantity-0 Forma Blueprint and the unresolvable
  // Bad Baby are both left out, leaving seven the backend will actually send.
  it('names how many items the refresh will price, and counts only ones it can price', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    render(<App/>)
    expect(await screen.findByRole('button', { name: /Price these 7/ })).toBeInTheDocument()
  })

  // A relic no dump in the last month saw trade is unpriced -- and an unpriced item is precisely
  // the one a manual refresh exists for. Sending only already-priced items would close the recovery
  // path against the items that need it.
  it('offers to price an owned item that has no price yet, and never an unowned one', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    const user = userEvent.setup()
    render(<App/>)
    await user.click(await screen.findByRole('button', { name: /Price these/ }))

    const requested = backend.refreshPrices.mock.calls[0][0]
    expect(requested).toContain('rhino')             // owned, no price yet
    expect(requested).not.toContain('forma-blueprint') // quantity 0, not owned
    expect(requested).not.toContain('bad-baby')        // owned, but no listing to ask about
  })

  // Sorting by stack value answers "where is my platinum"; sorting by unit price answers "what is
  // worth the most". The sort is for the second question, and the card still shows the first.
  it('sorts by unit price, not by what the stack is worth', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    backend.getView.mockResolvedValue({
      ...view,
      collection: {
        ...view.collection,
        items: [
          ...view.collection.items,
          { id: 'ash-prime-blueprint', name: 'Ash Prime Blueprint', category: 'blueprint', quantity: 1, mastered: false, platinum: 45, live: false },
        ],
      },
    })
    const user = userEvent.setup()
    render(<App/>)
    await user.click(await screen.findByRole('button', { name: 'Value' }))

    const names = screen.getAllByRole('article').map(article => article.getAttribute('aria-label'))
    expect(names[0]).toBe('Ash Prime Blueprint')  // 45p × 1
    expect(names[1]).toBe('Lith A1 Relic')        // 20p × 7 = 140 total, but 20p each
    expect(names[2]).toBe('Lex Prime Receiver')   // 19p, below the relic it outranks by stack value
    // Zenith is priced at 0 -- it must rank above every unpriced item, not tie with them.
    expect(names[3]).toBe('Zenith Prime Receiver')
    expect(names.at(-1)).toBe('Rhino')
  })

  it('narrows to items that have a price', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    const user = userEvent.setup()
    render(<App/>)
    await user.click(await screen.findByRole('button', { name: 'Tradeable' }))

    const names = screen.getAllByRole('article').map(article => article.getAttribute('aria-label'))
    expect(names).toEqual(['Lex Prime Receiver', 'Lith A1 Relic', 'Zenith Prime Receiver'])
  })

  // The market rate is the plain reading of what is owned; what the market would actually take is
  // the qualification on it, so it is under the figure at the size of one. The fixture caps in both
  // directions: the market takes all 1 Lex Prime Receiver at 19p, and 3 of the 7 Lith A1 at 20p, so
  // 159p at market rate is 79p anybody could actually sell.
  it('leads with the market rate and puts what is sellable under it', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    render(<App/>)
    const worth = await screen.findByTestId('band-worth')
    expect(within(worth).getByText('159'), 'the worth is a figure, in a row of plain counts').toBeInTheDocument()
    // Both figures carry the game's own icon. Without it the second one reads as another item count,
    // which is exactly what the three cells beside it hold.
    expect(within(worth).getAllByAltText(/platinum/i)).toHaveLength(2)
    expect(worth.querySelector('.band-aside')?.textContent, 'the achievable total, the size of a footnote').toBe('79 sellable')
    expect(within(worth).getByText(/only the copies the market buys in a month/i), 'the cap, on the figure it applies to').toBeInTheDocument()
  })

  // A slider whose effect is invisible until you navigate away is a knob, not a control.
  it('leaves out stacks under the price floor, and says so where it is set', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    const user = userEvent.setup()
    render(<App/>)
    await user.click(await screen.findByRole('button', { name: 'Settings' }))

    const slider = screen.getByRole('slider', { name: /minimum platinum/i })
    fireEvent.change(slider, { target: { value: '20' } })
    // Only the 20p Lith A1 clears a 20p floor; the 19p Lex Prime Receiver no longer counts.
    expect(screen.getByText(/1 stacks counted · 60 platinum sellable/)).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Collection' }))
    const worth = await screen.findByTestId('band-worth')
    expect(within(worth).getByText('159'), 'the market rate never moves with the floor').toBeInTheDocument()
    expect(worth.querySelector('.band-aside')?.textContent).toBe('60 sellable')
    expect(within(worth).getByText(/at 20 platinum and over/)).toBeInTheDocument()
  })

  // The page refresh asks about exactly what is on screen, so a filtered view costs only the
  // requests that view is worth. The fixture is padded past one page of tradeable items so the
  // visible page and the full filtered set are provably different arrays.
  it('prices the items currently on screen, and only those', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    const filler = Array.from({ length: 50 }, (_, index) => ({
      id: `filler-${index.toString().padStart(2, '0')}`,
      name: `Filler ${index.toString().padStart(2, '0')}`,
      category: 'weapon' as const,
      quantity: 1,
      mastered: false,
      live: false,
      priceable: true,
      platinum: 5,
    }))
    backend.getView.mockResolvedValue({ ...view, collection: { ...view.collection, items: [...view.collection.items, ...filler] } })
    const user = userEvent.setup()
    render(<App/>)
    await user.click(await screen.findByRole('button', { name: 'Tradeable' }))
    await user.click(await screen.findByRole('button', { name: 'Go to page 2' }))
    await user.click(screen.getByRole('button', { name: /Price these/ }))

    // Page 2 of 53 tradeable items (50 filler + 3 named) holds only the last 5, alphabetically.
    expect(backend.refreshPrices).toHaveBeenCalledWith(['filler-48', 'filler-49', 'lex-prime-receiver', 'lith-a1', 'zenith-prime-receiver'])
  })

  it('routes to the Orders section and counts flagged listings on the nav entry', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    backend.marketStatus.mockResolvedValue({ ...view, market_account: { ...view.market_account, link: 'linked', flagged: 2 } })
    render(<App/>)
    await screen.findByRole('heading', { name: 'Your collection' })
    expect(await screen.findByText('2', { selector: '.punch-count' })).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: 'Orders' }))
    expect(await screen.findByRole('heading', { name: 'Market orders' })).toBeInTheDocument()
  })

  it('shows a listed-order badge on a collection item with a live sell order', async () => {
    backend.getSetupStatus.mockResolvedValue({ risk_accepted: true })
    backend.marketStatus.mockResolvedValue({
      ...view,
      market_account: {
        ...view.market_account,
        link: 'linked',
        orders: [{
          order: { id: 'o1', item_id: 'rhino', kind: 'sell', platinum: 30, quantity: 1, per_trade: 1, visible: true },
          name: 'Rhino',
          status: { state: 'ok' },
        }],
      },
    })
    render(<App/>)
    await screen.findByRole('heading', { name: 'Your collection' })
    expect(await screen.findByText(/listed 30p/i)).toBeInTheDocument()
  })
})
