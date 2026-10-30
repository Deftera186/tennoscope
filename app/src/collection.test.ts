import { describe, expect, it } from 'vitest'
import { atMaxRank, collectionTotals, hasEquipData, hasUnequippedCopy, isUnrankedUpgrade, masteryOrder, meetsMinPrice, pageCount, pageItems, pageNumbers, rankLabel, sellableValue, stackValue } from './collection'

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

describe('collectionTotals', () => {
  it('partitions only mastery-eligible holdings by mastery and ownership', () => {
    const items = [
      { category: 'frame', quantity: 1, mastered: true },
      { category: 'weapon', quantity: 1, mastered: false, rank: 7, max_rank: 10 },
      { category: 'companion', quantity: 0, mastered: false },
      { category: 'mod', quantity: 3, mastered: true },
    ]

    expect(collectionTotals(items, 0)).toMatchObject({
      masteryEligible: 3,
      mastered: 1,
      owned: 3,
      missing: 1,
    })
  })

  it('totals only priced value and applies the per-copy sellable floor inclusively', () => {
    const items = [
      { category: 'prime_part', quantity: 2, mastered: false, platinum: 4, monthly_trades: 2 },
      { category: 'prime_part', quantity: 3, mastered: false, platinum: 5, monthly_trades: 2 },
      { category: 'prime_part', quantity: 1, mastered: false, platinum: 7, monthly_trades: 0 },
      { category: 'prime_part', quantity: 4, mastered: false },
    ]

    expect(collectionTotals(items, 5)).toMatchObject({
      priced: 3,
      worth: 30,
      sellable: 10,
      sellableCount: 1,
    })
  })

  it('counts ducats only for copies actually held', () => {
    const items = [
      { category: 'prime_part', quantity: 2, mastered: false, ducats: 45 },
      { category: 'prime_part', quantity: 0, mastered: false, ducats: 100 },
      { category: 'prime_part', quantity: 3, mastered: false },
    ]

    expect(collectionTotals(items, 0).ducatsAtStake).toBe(90)
  })

  it('returns zeroes for an empty collection', () => {
    expect(collectionTotals([], 5)).toEqual({
      masteryEligible: 0,
      mastered: 0,
      owned: 0,
      missing: 0,
      priced: 0,
      worth: 0,
      sellable: 0,
      sellableCount: 0,
      ducatsAtStake: 0,
    })
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

describe('masteryOrder', () => {
  it('puts unmastered equipment first, mastered equipment next, and everything else last', () => {
    expect(masteryOrder({ category: 'weapon', mastered: false })).toBe(0)
    expect(masteryOrder({ category: 'frame', mastered: false })).toBe(0)
    expect(masteryOrder({ category: 'companion', mastered: true })).toBe(1)
    expect(masteryOrder({ category: 'vehicle', mastered: true })).toBe(1)
  })

  it('ranks what earns no mastery last whatever its flag says', () => {
    expect(masteryOrder({ category: 'mod', mastered: false })).toBe(2)
    expect(masteryOrder({ category: 'prime_part', mastered: false })).toBe(2)
    expect(masteryOrder({ category: 'relic', mastered: true })).toBe(2)
  })
})

describe('copy filters', () => {
  // A copy installed in a build is one the player relies on. Unknown is not "none in use": a row
  // read before the app counted must never be offered as spare.
  it('finds a row with a copy installed nowhere, and never guesses one', () => {
    expect(hasUnequippedCopy({ quantity: 3, equipped: 1 })).toBe(true)
    expect(hasUnequippedCopy({ quantity: 2, equipped: 0 })).toBe(true)
    expect(hasUnequippedCopy({ quantity: 1, equipped: 1 })).toBe(false)
    expect(hasUnequippedCopy({ quantity: 0, equipped: 0 })).toBe(false)
    expect(hasUnequippedCopy({ quantity: 2 })).toBe(false)
  })

  it('knows equip state once any row carries it', () => {
    expect(hasEquipData([])).toBe(false)
    expect(hasEquipData([{}])).toBe(false)
    expect(hasEquipData([{}, { equipped: 0 }])).toBe(true)
  })

  it('takes the unranked stack of mods and arcanes, and nothing without ranks', () => {
    expect(isUnrankedUpgrade({ category: 'mod' })).toBe(true)
    expect(isUnrankedUpgrade({ category: 'arcane' })).toBe(true)
    expect(isUnrankedUpgrade({ category: 'mod', rank: 5 })).toBe(false)
    expect(isUnrankedUpgrade({ category: 'prime_part' })).toBe(false)
  })

  // Above Any an unpriced row fails, as Tradeable does: hundreds of resources and blueprints have
  // no market, and keeping them would bury the copies the floor is set to find.
  it('keeps a copy priced at or over the floor, and nothing unpriced above Any', () => {
    expect(meetsMinPrice({}, 0)).toBe(true)
    expect(meetsMinPrice({ platinum: 0 }, 0)).toBe(true)
    expect(meetsMinPrice({}, 3)).toBe(false)
    expect(meetsMinPrice({ platinum: 2 }, 3)).toBe(false)
    expect(meetsMinPrice({ platinum: 3 }, 3)).toBe(true)
  })
})
