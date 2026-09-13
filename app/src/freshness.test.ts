import { describe, expect, it } from 'vitest'
import { snapshotFreshness, stampReading } from './freshness'

const now = new Date('2026-07-25T12:00:00Z')

describe('snapshot freshness', () => {
  it('formats relative freshness and exact source details', () => {
    const result = snapshotFreshness({
      observed_at: 1_784_980_560, game_build: 'build-42', source: 'warframe-memory',
    }, now)
    expect(result.label).toBe('Synced 4 minutes ago')
    expect(result.detail).toContain('warframe-memory')
    expect(result.detail).toContain('build-42')
  })

  it('keeps the source on record when the stamp is unreadable', () => {
    const result = snapshotFreshness({
      observed_at: Number.NaN, game_build: 'build-42', source: 'warframe-memory',
    }, now)
    expect(result.label).toBe('Sync time unavailable')
    expect(result.detail).toBe('Source: warframe-memory · Build: build-42')
  })

  it('handles a missing snapshot honestly', () => {
    expect(snapshotFreshness(null, now)).toEqual({
      label: 'No successful sync yet',
      detail: 'TennoScope has not saved a coherent inventory snapshot.',
    })
  })
})

describe('backend stamp readings', () => {
  it('reads Unix-second stamps as instants, not as year-1970 milliseconds', () => {
    // Health rows and the market fetch time arrive as seconds. `new Date('1784980560')` is not a
    // 2026 date but `Invalid Date`, and a shorter stamp like '1' silently lands in 2001 -- which is
    // how a fixture once hid this.
    expect(stampReading('1784980560', now)?.relative).toBe('4 minutes ago')
  })

  it('still reads the price dump\'s calendar-date stamp', () => {
    expect(stampReading('2026-07-25', now)?.relative).toBe('12 hours ago')
  })

  it('scales the reading to the elapsed time', () => {
    expect(stampReading('1784980770', now)?.relative).toBe('just now')
    expect(stampReading('1784980740', now)?.relative).toBe('1 minute ago')
    expect(stampReading('1784977200', now)?.relative).toBe('1 hour ago')
    expect(stampReading('1784970000', now)?.relative).toBe('3 hours ago')
    expect(stampReading('1784894400', now)?.relative).toBe('1 day ago')
    expect(stampReading('1784721600', now)?.relative).toBe('3 days ago')
  })

  it('reports the exact instant alongside the relative one', () => {
    expect(stampReading('1784980770', now)?.exact).toMatch(/2026/)
  })

  it('refuses to invent a time for an unusable stamp', () => {
    expect(stampReading('not-a-time', now)).toBeNull()
    expect(stampReading('', now)).toBeNull()
  })
})
