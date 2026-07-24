import { cleanup, render, screen } from '@testing-library/react'
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
    capture: { state: 'degraded', message: 'Not connected', last_success: null },
    catalog: { state: 'ready', message: 'Catalog ready', last_success: '1' },
    market: { state: 'degraded', message: 'Not connected', last_success: null },
    database: { state: 'ready', message: 'SQLite database available', last_success: null },
    acquisition_stages: [],
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
  })
})
