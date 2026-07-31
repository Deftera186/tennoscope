import { describe, expect, it } from 'vitest'
import { atMaxRank, pageCount, pageItems, pageNumbers, rankLabel, sellableValue, stackValue } from './collection'

describe('collection pagination', () => {
  it('renders at most 48 items and returns the expected range', () => {
    const items = Array.from({ length: 106 }, (_, index) => index)
    expect(pageCount(items.length)).toBe(3)
    expect(pageItems(items, 1)).toEqual(items.slice(0, 48))
    expect(pageItems(items, 2)).toEqual(items.slice(48, 96))
    expect(pageItems(items, 3)).toEqual(items.slice(96))
  })

  it('clamps invalid pages and produces bounded navigation labels', () => {
    const items = Array.from({ length: 1000 }, (_, index) => index)
    expect(pageItems(items, 999)).toEqual(items.slice(960, 1000))
    expect(pageNumbers(1, 21)).toEqual([1, 2, 3, 4, 5, 21])
    expect(pageNumbers(11, 21)).toEqual([1, 9, 10, 11, 12, 13, 21])
    expect(pageNumbers(21, 21)).toEqual([1, 17, 18, 19, 20, 21])
  })
})

describe('stackValue', () => {
  it('values a stack at its unit price times what is owned', () => {
    expect(stackValue({ quantity: 3, platinum: 19 })).toBe(57)
  })

  it('has no value for an item with no price', () => {
    expect(stackValue({ quantity: 3 })).toBeNull()
  })
})

describe('sellableValue', () => {
  it('stops at what the market takes, and at what is owned', () => {
    expect(sellableValue({ quantity: 182, platinum: 2, monthly_trades: 2 })).toBe(4)
    expect(sellableValue({ quantity: 1, platinum: 19, monthly_trades: 4 })).toBe(19)
  })

  // Scan Matter is a 240p mod with no completed trade in twenty-eight days. A correct unit price on
  // a copy nobody bought is worth nothing in bulk, so an absent count is zero rather than no limit.
  it('is worth nothing when nobody traded one', () => {
    expect(sellableValue({ quantity: 1, platinum: 240 })).toBe(0)
    expect(sellableValue({ quantity: 3 })).toBeNull()
  })

  // The floor is per copy, not per stack: 182 Quickdraw at 2p is a lot of platinum and still 182
  // copies of a 2p mod, which is exactly the holding somebody sets a floor to stop counting.
  it('drops a stack whose copies are cheaper than the floor', () => {
    expect(sellableValue({ quantity: 182, platinum: 2, monthly_trades: 2 }, 5)).toBe(0)
    expect(sellableValue({ quantity: 1, platinum: 19, monthly_trades: 4 }, 5)).toBe(19)
    expect(sellableValue({ quantity: 1, platinum: 5, monthly_trades: 4 }, 5), 'the floor counts itself').toBe(5)
    expect(sellableValue({ quantity: 3 }, 5), 'an unpriced item has no answer, floor or no floor').toBeNull()
  })
})

describe('rank marks', () => {
  it('says nothing for the unranked stack, which is most of the collection', () => {
    expect(rankLabel({})).toBeNull()
    expect(atMaxRank({})).toBeNull()
  })

  it('reads a rank against its ceiling', () => {
    expect(rankLabel({ rank: 7, max_rank: 10 })).toBe('Rank 7/10')
    expect(atMaxRank({ rank: 7, max_rank: 10 })).toBe(false)
    expect(atMaxRank({ rank: 10, max_rank: 10 })).toBe(true)
  })

  // A riven's ceiling is published as a sentinel, so it is dropped rather than believed. Unknown
  // is not "no": a card that might be maxed must not be drawn as one that certainly is not.
  it('shows a rank with no known ceiling without claiming one', () => {
    expect(rankLabel({ rank: 3 })).toBe('Rank 3')
    expect(atMaxRank({ rank: 3 })).toBeNull()
  })
})
