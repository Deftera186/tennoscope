import { describe, expect, it } from 'vitest'
import type { ReconciledOrder } from './backend'
import { backingLabel, fixLabel, isFlagged, listedOrderFor, orderValue, sortOrders, statusLabel, uncountedReason } from './orders'

function entry(
  id: string,
  status: ReconciledOrder['status'],
  overrides: Partial<ReconciledOrder['order']> = {},
): ReconciledOrder {
  return {
    order: {
      id,
      item_id: `item-${id}`,
      kind: 'sell',
      platinum: 12,
      quantity: 1,
      per_trade: 1,
      visible: true,
      updated_at: '2026-07-30T10:00:00Z',
      ...overrides,
    },
    name: 'Braton Prime Blueprint',
    status,
  }
}

describe('order flags', () => {
  it('treats only claims as flags', () => {
    expect(isFlagged({ state: 'missing' })).toBe(true)
    expect(isFlagged({ state: 'overshoot', owned: 1 })).toBe(true)
    expect(isFlagged({ state: 'ok' })).toBe(false)
    // The rule the whole feature rests on: not knowing is not a problem, and a row that says so
    // would be an accusation the app cannot support.
    expect(isFlagged({ state: 'unverifiable' })).toBe(false)
  })

  it('says what is wrong in the row, in the player’s terms', () => {
    expect(statusLabel(entry('a', { state: 'missing' }))).toBe('You no longer own this')
    expect(statusLabel(entry('b', { state: 'overshoot', owned: 1 }, { quantity: 3 }))).toBe(
      'You own 1 of 3 listed',
    )
    expect(statusLabel(entry('c', { state: 'ok' }))).toBeNull()
    expect(statusLabel(entry('d', { state: 'unverifiable' }))).toBeNull()
  })

  it('names the fix as the action it performs, and leaves removal to every row', () => {
    expect(fixLabel({ state: 'overshoot', owned: 1 })).toBe('Lower to 1')
    expect(fixLabel({ state: 'missing' })).toBeNull()
    expect(fixLabel({ state: 'ok' })).toBeNull()
    expect(fixLabel({ state: 'unverifiable' })).toBeNull()
  })
})

describe('order ordering', () => {
  it('puts the rows needing attention first', () => {
    const sorted = sortOrders([
      entry('fine', { state: 'ok' }),
      entry('unknown', { state: 'unverifiable' }),
      entry('gone', { state: 'missing' }),
      entry('over', { state: 'overshoot', owned: 1 }),
    ])

    expect(sorted.map(each => each.order.id)).toEqual(['gone', 'over', 'fine', 'unknown'])
  })

  it('is stable for rows of equal standing, so the list does not shuffle on refresh', () => {
    const first = entry('one', { state: 'ok' })
    const second = entry('two', { state: 'ok' })

    expect(sortOrders([first, second]).map(each => each.order.id)).toEqual(['one', 'two'])
    expect(sortOrders([second, first]).map(each => each.order.id)).toEqual(['two', 'one'])
  })
})

describe('the collection badge', () => {
  it('finds a visible sell order for an item', () => {
    const orders = [entry('a', { state: 'ok' }, { item_id: '/Lotus/Thing', platinum: 24 })]

    expect(listedOrderFor(orders, '/Lotus/Thing')?.platinum).toBe(24)
  })

  it('ignores hidden orders and buy orders, which are not listings anybody sees', () => {
    const hidden = entry('h', { state: 'ok' }, { item_id: '/Lotus/Thing', visible: false })
    const buying = entry('b', { state: 'ok' }, { item_id: '/Lotus/Thing', kind: 'buy' })

    expect(listedOrderFor([hidden, buying], '/Lotus/Thing')).toBeNull()
  })

  it('has nothing to say about an item with no order', () => {
    expect(listedOrderFor([], '/Lotus/Thing')).toBeNull()
  })
})

describe('credential backing', () => {
  it('names where the credential lives, because the two are not equally strong', () => {
    expect(backingLabel('keyring')).toBe('System keyring')
    expect(backingLabel('database')).toBe('Local database file')
    expect(backingLabel(undefined)).toBe('Not stored')
  })
})

describe('what a row contributes to the listed total', () => {
  it('prices the trade, not the unit', () => {
    // 12 units at 38p per six is asking 76p, not 456p.
    expect(orderValue(entry('a', { state: 'ok' }, { platinum: 38, quantity: 12, per_trade: 6 }).order)).toBe(76)
    expect(orderValue(entry('a', { state: 'ok' }, { quantity: 3 }).order)).toBe(36)
  })

  it('counts nothing from an order nobody can buy, and says which', () => {
    const hidden = entry('a', { state: 'ok' }, { visible: false }).order
    const buying = entry('a', { state: 'ok' }, { kind: 'buy' }).order
    expect(orderValue(hidden)).toBeNull()
    expect(uncountedReason(hidden)).toBe('Hidden')
    expect(orderValue(buying)).toBeNull()
    expect(uncountedReason(buying)).toBe('Buy order')
    expect(uncountedReason(entry('a', { state: 'ok' }).order)).toBeNull()
  })
})
