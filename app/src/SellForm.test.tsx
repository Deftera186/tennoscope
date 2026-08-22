import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { CollectionItem, MarketOrder } from './backend'
import { SellForm } from './SellForm'

// The handlers are shared across every test in this file; without clearing, a call one test made
// reads as made by the next.
afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

const braton: CollectionItem = {
  id: '/Lotus/Types/Recipes/Weapons/BratonPrimeBlueprint',
  name: 'Braton Prime Blueprint',
  category: 'prime_part',
  quantity: 5,
  mastered: false,
  platinum: 14,
  live: false,
  priceable: true,
}

function listing(overrides: Partial<MarketOrder> = {}): MarketOrder {
  return {
    id: 'order-one',
    item_id: '54a73e65e779893a797fff33',
    kind: 'sell',
    platinum: 12,
    quantity: 3,
    per_trade: 1,
    visible: true,
    updated_at: '2026-07-30T10:00:00Z',
    ...overrides,
  }
}

const handlers = {
  onSell: vi.fn().mockResolvedValue(undefined),
  onUpdate: vi.fn().mockResolvedValue(undefined),
  onDone: vi.fn(),
}

describe('editing a listing', () => {
  it('prefills from the listing, not from the card’s quote', () => {
    render(<SellForm item={braton} listing={listing()} busy={false} onSell={handlers.onSell} onUpdate={handlers.onUpdate} onDone={handlers.onDone} />)

    expect(screen.getByLabelText('Platinum')).toHaveValue(12)
    expect(screen.getByLabelText('Quantity')).toHaveValue(3)
  })

  it('saves the price and the count through the order, never a second listing', async () => {
    const user = userEvent.setup()
    render(<SellForm item={braton} listing={listing()} busy={false} onSell={handlers.onSell} onUpdate={handlers.onUpdate} onDone={handlers.onDone} />)

    await user.clear(screen.getByLabelText('Quantity'))
    await user.type(screen.getByLabelText('Quantity'), '5')
    await user.click(screen.getByRole('button', { name: /save listing/i }))

    expect(handlers.onUpdate).toHaveBeenCalledWith('order-one', 12, 5)
    expect(handlers.onSell).not.toHaveBeenCalled()
    expect(handlers.onDone).toHaveBeenCalled()
  })

  it('refuses a count above what the device holds', async () => {
    const user = userEvent.setup()
    render(<SellForm item={braton} listing={listing()} busy={false} onSell={handlers.onSell} onUpdate={handlers.onUpdate} onDone={handlers.onDone} />)

    await user.clear(screen.getByLabelText('Quantity'))
    await user.type(screen.getByLabelText('Quantity'), '9')

    expect(screen.getByRole('button', { name: /save listing/i })).toBeDisabled()
  })

  // The save patches price and count only. A checkbox that appeared to change visibility but sent
  // nothing would be a control that lies about what it does.
  it('offers no visibility choice, because the save does not send one', () => {
    render(<SellForm item={braton} listing={listing()} busy={false} onSell={handlers.onSell} onUpdate={handlers.onUpdate} onDone={handlers.onDone} />)

    expect(screen.queryByLabelText(/visible to buyers/i)).toBeNull()
  })
})

describe('publishing a new listing', () => {
  it('sends the price and count typed, visibly, as a new listing for the row', async () => {
    const user = userEvent.setup()
    render(<SellForm item={braton} busy={false} onSell={handlers.onSell} onUpdate={handlers.onUpdate} onDone={handlers.onDone} />)

    await user.clear(screen.getByLabelText('Quantity'))
    await user.type(screen.getByLabelText('Quantity'), '2')
    await user.click(screen.getByRole('button', { name: /list for sale/i }))

    expect(handlers.onSell).toHaveBeenCalledWith(braton.id, 14, 2, true)
    expect(handlers.onUpdate).not.toHaveBeenCalled()
  })
})
