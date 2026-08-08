import { describe, expect, it } from 'vitest'
import type { AppView } from './backend'
import { reportBlockVisible } from './reportable'

function health(overrides: Partial<AppView['health']> = {}): AppView['health'] {
  const ready = { state: 'ready', message: 'ok', last_success: null } as const
  return {
    game_reader: ready, log_monitor: ready, capture: ready, catalog: ready,
    market: ready, collection_prices: ready, database: ready, market_account: { ...ready },
    acquisition_stages: [],
    ...overrides,
  }
}

describe('reportBlockVisible', () => {
  it('is false when everything is ready or idle', () => {
    const h = health({ market_account: { state: 'idle', message: 'not linked', last_success: null } })
    expect(reportBlockVisible(h)).toBe(false)
  })

  it('is true when any game-independent row is degraded', () => {
    expect(reportBlockVisible(health({ market: { state: 'degraded', message: 'offline', last_success: null } }))).toBe(true)
  })

  it('is true when any row has failed', () => {
    expect(reportBlockVisible(health({ catalog: { state: 'failed', message: 'no catalog', last_success: null } }))).toBe(true)
  })

  it('ignores a degraded game row when the game is not running', () => {
    const h = health({ game_reader: { state: 'degraded', message: 'waiting', last_success: null } })
    expect(reportBlockVisible(h)).toBe(false)
  })

  it('reports a failed game row even when the game is not running', () => {
    const h = health({ game_reader: { state: 'failed', message: 'reader crashed', last_success: null } })
    expect(reportBlockVisible(h)).toBe(true)
  })

  it('reports a degraded game row while the game is running', () => {
    const h = health({
      game_reader: { state: 'ready', message: 'ok', last_success: null },
      log_monitor: { state: 'degraded', message: 'EE.log not found', last_success: null },
    })
    expect(reportBlockVisible(h)).toBe(true)
  })

  it('reports degraded or failed acquisition stages', () => {
    expect(reportBlockVisible(health({ acquisition_stages: [{ stage: 'schema_validation', state: 'failed', message: 'bad' }] }))).toBe(true)
    expect(reportBlockVisible(health({ acquisition_stages: [{ stage: 'memory_permission', state: 'degraded', message: 'locked' }] }))).toBe(true)
  })
})