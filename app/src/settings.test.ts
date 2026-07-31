import { beforeEach, describe, expect, it, vi } from 'vitest'

import { clampPriceFloor, MAX_PRICE_FLOOR, readPriceFloor, writePriceFloor } from './settings'

describe('price floor', () => {
  beforeEach(() => localStorage.clear())

  it('survives the window it was set in', () => {
    writePriceFloor(5)
    expect(readPriceFloor()).toBe(5)
  })

  it('answers with no floor for anything it cannot read as one', () => {
    expect(readPriceFloor(), 'nothing stored yet').toBe(0)
    expect(clampPriceFloor('twelve')).toBe(0)
    expect(clampPriceFloor(null)).toBe(0)
  })

  it('keeps a stored value inside the slider it came from', () => {
    expect(clampPriceFloor(-3)).toBe(0)
    expect(clampPriceFloor(9000)).toBe(MAX_PRICE_FLOOR)
    expect(clampPriceFloor(4.6), 'the slider only has whole platinum on it').toBe(5)
  })

  // A webview with storage disabled throws on access. The preference is a display choice, so
  // losing it at the end of the session is the correct failure -- refusing to draw the app is not.
  it('still runs where storage is refused', () => {
    const denied = vi.spyOn(localStorage, 'getItem').mockImplementation(() => { throw new Error('denied') })
    const deniedWrite = vi.spyOn(localStorage, 'setItem').mockImplementation(() => { throw new Error('denied') })
    expect(readPriceFloor()).toBe(0)
    expect(() => writePriceFloor(7)).not.toThrow()
    denied.mockRestore()
    deniedWrite.mockRestore()
  })
})
