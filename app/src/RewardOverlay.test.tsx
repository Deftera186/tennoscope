import { cleanup, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const backend = vi.hoisted(() => ({ getView: vi.fn(), getVersionInfo: vi.fn(), updateCheck: vi.fn(), updateDownloadAndInstall: vi.fn() }))
const overlay = vi.hoisted(() => ({ hideRewardOverlay: vi.fn() }))
const events = vi.hoisted(() => ({ listener: undefined as undefined | (() => void), listen: vi.fn() }))
vi.mock('./backend', () => backend)
vi.mock('./overlay', () => overlay)
vi.mock('@tauri-apps/api/event', () => ({
  listen: events.listen.mockImplementation((_event: string, listener: () => void) => {
    events.listener = listener
    return Promise.resolve(() => { events.listener = undefined })
  }),
}))

import { AppRoute } from './Root'
import { routeForPath } from './routing'
import type { RewardCard } from './backend'

const overlayView = {
  collection: { items: [], total_entries: 0 },
  reward: {
    cards: [
      { name: 'Certain', platinum: 10, ducats: 15, owned: 0, mastery: { state: 'mastered' }, confidence: 1 },
      { name: 'Uncertain', platinum: 100, ducats: 100, owned: 0, confidence: 0.7 },
    ],
    best_value_index: 1,
    best_ducat_index: 1,
  },
  health: {
    game_reader: { state: 'ready', message: 'ready', last_success: null },
    log_monitor: { state: 'ready', message: 'ready', last_success: null },
    capture: { state: 'degraded', message: 'waiting', last_success: null },
    catalog: { state: 'ready', message: 'ready', last_success: null },
    market: { state: 'degraded', message: 'waiting', last_success: null },
    collection_prices: { state: 'idle', message: 'Collection price dump has not loaded yet', last_success: null },
    database: { state: 'ready', message: 'ready', last_success: null },
    acquisition_stages: [],
  },
}

/** The Lex Prime recipe as the backend resolves it: the blueprint and receiver held, the barrel this reward fills. */
const set = [
  { name: 'Blueprint', image: 'blueprint.png', uses: 1, held: 1, this: false },
  { name: 'Barrel', image: 'GenericGunPrimeBarrel.png', uses: 1, held: 0, this: true },
  { name: 'Receiver', image: 'GenericGunPrimeReceiver.png', uses: 1, held: 1, this: false },
]

function slip(name: string, rest: Partial<RewardCard> = {}): RewardCard {
  return { name, platinum: 10, ducats: 15, owned: 0, confidence: 1, ...rest }
}

/** Renders the overlay over exactly these slips and crowns no winner, so each test reads one card. */
async function renderOverlay(cards: RewardCard[]) {
  backend.getView.mockResolvedValue({ ...overlayView, reward: { cards, best_value_index: null, best_ducat_index: null } })
  render(<AppRoute pathname="/overlay" />)
  return screen.findByRole('main', { name: 'Reward overlay' })
}

describe('reward overlay route', () => {
  afterEach(cleanup)
  beforeEach(() => {
    vi.clearAllMocks()
    events.listener = undefined
    backend.getView.mockResolvedValue(overlayView)
    overlay.hideRewardOverlay.mockResolvedValue(undefined)
  })

  it('routes only the overlay pathname to the focused overlay', () => {
    expect(routeForPath('/overlay')).toBe('overlay')
    expect(routeForPath('/')).toBe('main')
    expect(routeForPath('/collection')).toBe('main')
  })

  it('renders reward decisions without interactive window chrome', async () => {
    render(<AppRoute pathname="/overlay" />)
    const advisor = await screen.findByRole('main', { name: 'Reward overlay' })
    expect(within(advisor).getAllByRole('article')).toHaveLength(2)
    expect(within(advisor).getByRole('article', { name: 'Uncertain' })).toHaveTextContent('Uncertain ·')
    // It leads on both metrics, but a read we do not trust must not be crowned on either.
    expect(within(advisor).getByRole('article', { name: 'Uncertain' })).not.toHaveTextContent('Top plat')
    expect(within(advisor).getByRole('article', { name: 'Uncertain' })).not.toHaveTextContent('Top ducats')
    expect(within(advisor).getByRole('article', { name: 'Certain' })).toHaveTextContent('Mastered')
    expect(within(advisor).queryByRole('button')).not.toBeInTheDocument()
  })

  it('renders an honest empty overlay', async () => {
    backend.getView.mockResolvedValue({ ...overlayView, reward: { cards: [], best_value_index: null, best_ducat_index: null } })
    render(<AppRoute pathname="/overlay" />)
    expect(await screen.findByText('No reward choices detected')).toBeInTheDocument()
  })

  it('refreshes immediately when native reward data is published', async () => {
    render(<AppRoute pathname="/overlay" />)
    expect(await screen.findByText('Certain')).toBeInTheDocument()
    backend.getView.mockResolvedValue({
      ...overlayView,
      reward: {
        cards: [{ name: 'Fresh reward', platinum: 20, ducats: 45, owned: 0, confidence: 1 }],
        best_value_index: 0,
      },
    })

    events.listener?.()

    await waitFor(() => expect(screen.getByText('Fresh reward')).toBeInTheDocument())
    expect(backend.getView).toHaveBeenCalledTimes(2)
  })

  it('states mastery on each slip and rules the slips that fill a gap', async () => {
    const advisor = await renderOverlay([
      slip('Lex Prime Barrel', { mastery: { state: 'unmastered', subject: null, parts: set, missing: true, completes: true } }),
      slip('Paris Prime String', { mastery: { state: 'mastered' } }),
      slip('Braton Prime Stock', { mastery: { state: 'built', rank: 14, max_rank: 30 } }),
      slip('Forma Blueprint'),
    ])
    // Completes set earns both signals, the inverted word and the double rule: this reward is the
    // last part missing from a buildable item.
    const lex = within(advisor).getByRole('article', { name: 'Lex Prime Barrel' })
    expect(within(lex).getByText('Completes set')).toHaveClass('ledger-word', 'inverted')
    expect(lex.querySelector('.ledger')).toHaveAttribute('data-tone', 'loud')
    expect(lex).toHaveClass('ruled-double')
    expect(lex).not.toHaveClass('ruled')
    expect(within(lex).getByRole('list', { name: 'Set' })).toBeInTheDocument()
    expect(within(lex).queryByText('Not owned')).not.toBeInTheDocument()
    // A mark that answers nothing loud carries its word and no rule.
    const paris = within(advisor).getByRole('article', { name: 'Paris Prime String' })
    expect(within(paris).getByText('Mastered')).toBeInTheDocument()
    expect(paris).not.toHaveClass('ruled')
    expect(paris).not.toHaveClass('ruled-double')
    expect(within(paris).queryByRole('list', { name: 'Set' })).not.toBeInTheDocument()
    expect(within(advisor).getByRole('article', { name: 'Braton Prime Stock' })).toHaveTextContent('Built · R14/30')
    // An item mastery never touches keeps today's ownership line and says nothing about mastery.
    const forma = within(advisor).getByRole('article', { name: 'Forma Blueprint' })
    expect(within(forma).getByText('Not owned')).toBeInTheDocument()
    expect(within(forma).queryByText(/Mastered|Unmastered/)).not.toBeInTheDocument()
    expect(forma).not.toHaveClass('ruled')
    expect(forma).not.toHaveClass('ruled-double')
  })

  it('names the consumer a mastered part is still wanted by', async () => {
    const advisor = await renderOverlay([
      slip('Bronco Prime', { mastery: { state: 'unmastered', subject: 'Akbronco', parts: set, missing: true, completes: false } }),
    ])
    const bronco = within(advisor).getByRole('article', { name: 'Bronco Prime' })
    expect(within(bronco).getByText('Unmastered Akbronco')).toBeInTheDocument()
    expect(bronco).toHaveClass('ruled')
    expect(within(bronco).getByRole('list', { name: 'Set' })).toBeInTheDocument()
  })

  it('draws no rule for a part the player already holds, but still shows the set', async () => {
    const advisor = await renderOverlay([
      slip('Lex Prime Barrel', { mastery: { state: 'unmastered', subject: null, parts: set, missing: false, completes: false } }),
    ])
    const lex = within(advisor).getByRole('article', { name: 'Lex Prime Barrel' })
    expect(within(lex).getByText('Unmastered')).toBeInTheDocument()
    expect(lex.querySelector('.ledger')).toHaveAttribute('data-tone', 'quiet')
    expect(lex).not.toHaveClass('ruled')
    expect(lex).not.toHaveClass('ruled-double')
    expect(within(lex).getByRole('list', { name: 'Set' })).toBeInTheDocument()
  })

  it('states a build in the foundry without ruling the slip', async () => {
    const advisor = await renderOverlay([slip('Lex Prime Barrel', { mastery: { state: 'in_foundry' } })])
    const lex = within(advisor).getByRole('article', { name: 'Lex Prime Barrel' })
    expect(within(lex).getByText('In foundry')).toBeInTheDocument()
    expect(lex).not.toHaveClass('ruled')
    expect(within(lex).queryByRole('list', { name: 'Set' })).not.toBeInTheDocument()
  })

  it('never rules, inverts, or draws a set for a read it does not trust', async () => {
    const advisor = await renderOverlay([
      slip('Lex Prime Barrel', { confidence: 0.6, mastery: { state: 'unmastered', subject: null, parts: set, missing: true, completes: true } }),
      slip('Paris Prime String', { confidence: 0.6, mastery: { state: 'unmastered', subject: null, parts: set, missing: true, completes: false } }),
    ])
    // The mark was built on a name the reader is unsure of, so the slip keeps the doubt mark and
    // states the word without the contour, the inverted chip, or the shapes the game owns.
    const lex = within(advisor).getByRole('article', { name: 'Lex Prime Barrel' })
    expect(within(lex).getByText(/Uncertain/)).toBeInTheDocument()
    expect(within(lex).getByText('Completes set')).not.toHaveClass('inverted')
    expect(lex).not.toHaveClass('ruled')
    expect(lex).not.toHaveClass('ruled-double')
    expect(within(lex).queryByRole('list', { name: 'Set' })).not.toBeInTheDocument()
    const paris = within(advisor).getByRole('article', { name: 'Paris Prime String' })
    expect(within(paris).getByText('Unmastered')).toBeInTheDocument()
    expect(paris).not.toHaveClass('ruled')
    expect(paris).not.toHaveClass('ruled-double')
    expect(within(paris).queryByRole('list', { name: 'Set' })).not.toBeInTheDocument()
  })

  it('reads an unknown mark as a dash with a spoken name', async () => {
    const advisor = await renderOverlay([slip('Lex Prime Barrel', { mastery: { state: 'unknown' } })])
    expect(screen.getByText('Mastery unknown')).toHaveClass('sr-only')
    // The dash is for the eye; a screen reader hears the spoken name rather than the glyph.
    const ledger = advisor.querySelector('.ledger') as HTMLElement
    expect(within(ledger).getByText('—')).toHaveAttribute('aria-hidden', 'true')
  })
})
