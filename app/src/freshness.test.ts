import { describe, expect, it } from 'vitest'
import { snapshotFreshness } from './freshness'

describe('snapshot freshness', () => {
  const now = new Date('2026-07-25T12:00:00Z')

  it('formats relative freshness and exact source details', () => {
    const result = snapshotFreshness({
      observed_at: '2026-07-25T11:56:00Z', game_build: 'build-42', source: 'warframe-memory',
    }, now)
    expect(result.label).toBe('Synced 4 minutes ago')
    expect(result.detail).toContain('warframe-memory')
    expect(result.detail).toContain('build-42')
  })

  it('accepts persisted Unix seconds and handles a missing snapshot honestly', () => {
    expect(snapshotFreshness({ observed_at: '1784980740', game_build: 'unknown', source: 'warframe-memory' }, now).label)
      .toMatch(/^Synced /)
    expect(snapshotFreshness(null, now)).toEqual({ label: 'No successful sync yet', detail: 'TennoScope has not saved a coherent inventory snapshot.' })
  })
})
