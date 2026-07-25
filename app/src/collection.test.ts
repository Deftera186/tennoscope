import { describe, expect, it } from 'vitest'
import { pageCount, pageItems, pageNumbers } from './collection'

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
