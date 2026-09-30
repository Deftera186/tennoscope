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
    updated_at: 1_785_405_600,
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

  // A greyed button that says nothing is the defect; a reason that names the wrong field is the
  // same defect wearing a fix. The description has to hang off the input, because a disabled
  // button is not focusable and a description on it can never be reached.
  it('names the bound that was crossed, on the field that crossed it', async () => {
    const user = userEvent.setup()
    render(<SellForm item={braton} listing={listing()} busy={false} onSell={handlers.onSell} onUpdate={handlers.onUpdate} onDone={handlers.onDone} />)

    const price = screen.getByLabelText('Platinum')
    const count = screen.getByLabelText('Quantity')
    // The association is what matters, so it is read back through aria-describedby rather than
    // against a hard-coded id: a literal id would pass even if the two were wired to each other.
    const reasonFor = (input: HTMLElement) => {
      const id = input.getAttribute('aria-describedby')
      return id === null ? null : document.getElementById(id)
    }
    expect(reasonFor(price)).toBeNull()
    expect(reasonFor(count)).toBeNull()

    await user.clear(count)
    await user.type(count, '9')

    expect(count.getAttribute('aria-invalid')).toBe('true')
    expect(reasonFor(count)?.textContent).toMatch(/whole number of 1 to 5/)
    expect(reasonFor(count)?.closest('label')).toBe(count.closest('label'))
    expect(reasonFor(price)).toBeNull()
    expect(price.getAttribute('aria-invalid')).toBe('false')

    await user.clear(count)
    await user.type(count, '1')
    expect(reasonFor(count)).toBeNull()
    expect(screen.getByRole('button', { name: /save listing/i })).toBeEnabled()
  })

  // Two collection cards, or two docket rows, can hold a form open at once, and their bounds
  // differ. A shared id would make the second form's aria-describedby resolve to the first form's
  // sentence, so a player would be told their limit by a row that is not theirs.
  it('keeps each reason with its own form when two are open', async () => {
    const user = userEvent.setup()
    const rare: CollectionItem = { ...braton, id: 'rare', name: 'Rare Item', quantity: 3 }
    render(<>
      <SellForm item={braton} listing={listing()} busy={false} onSell={handlers.onSell} onUpdate={handlers.onUpdate} onDone={handlers.onDone} />
      <SellForm item={rare} listing={listing({ item_id: 'rare' })} busy={false} onSell={handlers.onSell} onUpdate={handlers.onUpdate} onDone={handlers.onDone} />
    </>)

    const quantities = screen.getAllByLabelText('Quantity')
    for (const q of quantities) {
      await user.clear(q)
      await user.type(q, '9')
    }

    const sentences = quantities.map(q => {
      const node = document.getElementById(q.getAttribute('aria-describedby') ?? '')
      return { node, owner: node?.closest('form'), form: q.closest('form') }
    })
    expect(sentences[0].node?.textContent).toMatch(/1 to 5/)
    expect(sentences[1].node?.textContent).toMatch(/1 to 3/)
    for (const s of sentences) expect(s.owner).toBe(s.form)
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
