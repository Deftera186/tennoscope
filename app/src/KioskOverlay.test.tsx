import { cleanup, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const backend = vi.hoisted(() => ({ getKioskView: vi.fn() }))
const events = vi.hoisted(() => ({
  listeners: {} as Record<string, ((event: { payload: unknown }) => void) | undefined>,
  listen: vi.fn(),
}))
vi.mock('./backend', () => backend)
vi.mock('./MetalMark', () => ({ MetalMark: () => <img alt="" data-testid="plat-mark"/> }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: events.listen.mockImplementation((event: string, listener: () => void) => {
    events.listeners[event] = listener
    return Promise.resolve(() => { events.listeners[event] = undefined })
  }),
}))

import { AppRoute } from './Root'
import { routeForPath } from './routing'
import type { KioskView } from './backend'

const sampleView: KioskView = {
  epoch: 3,
  cells: [
    { col: 0, row: 0, name: 'Titania Prime Systems Blueprint', platinum: 30 },
    { col: 5, row: 2, name: 'Tiberon Prime Barrel', platinum: 12 },
  ],
  basket: [
    { index: 0, name: 'Afentis Prime Blade', platinum: 6 },
    { index: 1, name: 'Fulmin Prime Receiver', platinum: null },
  ],
  total_plat: 6,
  scroll_dy: 0,
}

describe('kiosk overlay route', () => {
  afterEach(cleanup)
  beforeEach(() => {
    vi.clearAllMocks()
    events.listen.mockClear()
    events.listeners = {}
    backend.getKioskView.mockResolvedValue(sampleView)
  })

  it('routes the kiosk pathname away from the main app', () => {
    expect(routeForPath('/kiosk')).toBe('kiosk')
    expect(routeForPath('/overlay')).toBe('overlay')
    expect(routeForPath('/')).toBe('main')
  })

  it('renders the kiosk overlay, not the full app, at /kiosk', async () => {
    render(<AppRoute pathname="/kiosk" />)
    const shell = await screen.findByRole('main', { name: 'Kiosk overlay' })
    expect(within(shell).getAllByTestId('kiosk-grid-chip')).toHaveLength(2)
    expect(within(shell).getAllByTestId('kiosk-basket-chip')).toHaveLength(2)
    expect(within(shell).getByTestId('kiosk-total')).toHaveTextContent('6p')
  })

  it('anchors grid chips at their calibrated tile corners', async () => {
    render(<AppRoute pathname="/kiosk" />)
    const [first, last] = await screen.findAllByTestId('kiosk-grid-chip')
    // The 50% already carries the design centre (960), so the offset is the raw edge minus it:
    // col0,row0 right edge = 76 + 0 + 190 - 6 = 260 -> 50% - 700; top = 199 - 2.
    expect(first).toHaveStyle({ left: 'calc(50% + -700 * var(--h))', top: 'calc(197 * var(--h))' })
    // col5,row2: right = 76 + 207.5*5 + 190 - 6 = 1297.5 -> 50% + 338; top = 643 - 2.
    expect(last).toHaveStyle({ left: 'calc(50% + 338 * var(--h))', top: 'calc(641 * var(--h))' })
  })

  it('right-aligns basket pairs onto the game ducat column', async () => {
    render(<AppRoute pathname="/kiosk" />)
    const [first] = await screen.findAllByTestId('kiosk-basket-chip')
    // Pair's bottom-right corner lands on (1750, baseline+descent): inside the pane edge,
    // clear of the game's own digits which start at x>=1759.
    expect(first).toHaveStyle({
      left: 'calc(50% + 790 * var(--h))',
      top: 'calc(246 * var(--h))',
      transform: 'translate(-100%, -100%)',
    })
  })

  it('adds only the platinum the game does not already show', async () => {
    render(<AppRoute pathname="/kiosk" />)
    const [first] = await screen.findAllByTestId('kiosk-basket-chip')
    expect(first).toHaveTextContent('6p')
    expect(first).not.toHaveTextContent('d')
    expect(screen.queryByText(/\d+d/)).not.toBeInTheDocument()
  })

  it('hides the total while the basket is empty', async () => {
    backend.getKioskView.mockResolvedValue({ ...sampleView, basket: [], total_plat: 0 })
    render(<AppRoute pathname="/kiosk" />)
    await screen.findByRole('main', { name: 'Kiosk overlay' })
    expect(screen.queryByTestId('kiosk-total')).not.toBeInTheDocument()
  })

  it('renders nothing when no kiosk session is live', async () => {
    backend.getKioskView.mockResolvedValue(null)
    render(<AppRoute pathname="/kiosk" />)
    await screen.findByRole('main', { name: 'Kiosk overlay' })
    expect(screen.queryByTestId('kiosk-grid-chip')).not.toBeInTheDocument()
  })

  it('follows streamed scroll offsets on the grid layer, basket pinned', async () => {
    render(<AppRoute pathname="/kiosk" />)
    const grid = await screen.findByTestId('kiosk-grid')
    const basket = (await screen.findAllByTestId('kiosk-basket-chip'))[0]
    await waitFor(() => expect(grid).toHaveStyle({ transform: 'translateY(calc(0 * var(--h)))' }))

    events.listeners['kiosk-scroll']?.({ payload: 5 })
    await waitFor(() => expect(grid).toHaveStyle({ transform: 'translateY(calc(5 * var(--h)))' }))
    // The backend streams movement, not position: each verdict is how far the grid went since
    // the last look, so the chips ride a scroll of any length by adding them up. (Assigning
    // them absolutely left the chips 17px from home on a 300px scroll.)
    events.listeners['kiosk-scroll']?.({ payload: 9 })
    await waitFor(() => expect(grid).toHaveStyle({ transform: 'translateY(calc(14 * var(--h)))' }))
    expect(basket).not.toHaveStyle({ transform: 'translateY(calc(14 * var(--h)))' })
    expect(grid).not.toHaveClass('kiosk-faded')
  })

  it('fades on an unreadable verdict and re-anchors with the view scroll offset', async () => {
    render(<AppRoute pathname="/kiosk" />)
    const grid = await screen.findByTestId('kiosk-grid')
    const strip = await screen.findByTestId('kiosk-strip')
    await waitFor(() => expect(strip).not.toHaveClass('kiosk-faded'))

    events.listeners['kiosk-scroll']?.({ payload: 12 })
    await waitFor(() => expect(grid).toHaveStyle({ transform: 'translateY(calc(12 * var(--h)))' }))
    events.listeners['kiosk-scroll']?.({ payload: null })
    await waitFor(() => expect(strip).toHaveClass('kiosk-faded'))

    // The settled read ran with bands shifted by the scroll, so the view says where the grid
    // now sits: the offset re-anchors to it instead of snapping back to zero.
    backend.getKioskView.mockResolvedValue({ ...sampleView, epoch: 4, scroll_dy: -142 })
    events.listeners['kiosk-updated']?.()
    await waitFor(() => expect(strip).not.toHaveClass('kiosk-faded'))
    expect(grid).toHaveStyle({ transform: 'translateY(calc(-142 * var(--h)))' })
    expect(await screen.findByTitle('Titania Prime Systems Blueprint')).toBeInTheDocument()
  })
})
