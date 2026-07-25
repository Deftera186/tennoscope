import { cleanup, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const backend = vi.hoisted(() => ({ getView: vi.fn() }))
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

const overlayView = {
  collection: { items: [], total_entries: 0 },
  reward: {
    cards: [
      { name: 'Certain', platinum: 10, ducats: 15, owned: 0, mastery_relevant: true, confidence: 1 },
      { name: 'Uncertain', platinum: 100, ducats: 100, owned: 0, mastery_relevant: false, confidence: 0.7 },
    ],
    best_value_index: 1,
  },
  health: {
    game_reader: { state: 'ready', message: 'ready', last_success: null },
    log_monitor: { state: 'ready', message: 'ready', last_success: null },
    capture: { state: 'degraded', message: 'waiting', last_success: null },
    catalog: { state: 'ready', message: 'ready', last_success: null },
    market: { state: 'degraded', message: 'waiting', last_success: null },
    database: { state: 'ready', message: 'ready', last_success: null },
    acquisition_stages: [],
  },
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
    expect(within(advisor).getByRole('article', { name: 'Uncertain' })).toHaveTextContent('Uncertain recognition')
    expect(within(advisor).getByRole('article', { name: 'Uncertain' })).not.toHaveTextContent('Best value')
    expect(within(advisor).getByRole('article', { name: 'Certain' })).toHaveTextContent('Mastery needed')
    expect(within(advisor).queryByRole('button')).not.toBeInTheDocument()
  })

  it('renders an honest empty overlay', async () => {
    backend.getView.mockResolvedValue({ ...overlayView, reward: { cards: [], best_value_index: null } })
    render(<AppRoute pathname="/overlay" />)
    expect(await screen.findByText('No reward choices detected')).toBeInTheDocument()
  })

  it('refreshes immediately when native reward data is published', async () => {
    render(<AppRoute pathname="/overlay" />)
    expect(await screen.findByText('Certain')).toBeInTheDocument()
    backend.getView.mockResolvedValue({
      ...overlayView,
      reward: {
        cards: [{ name: 'Fresh reward', platinum: 20, ducats: 45, owned: 0, mastery_relevant: false, confidence: 1 }],
        best_value_index: 0,
      },
    })

    events.listener?.()

    await waitFor(() => expect(screen.getByText('Fresh reward')).toBeInTheDocument())
    expect(backend.getView).toHaveBeenCalledTimes(2)
  })
})
