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
  it('is false when every row is ready or idle', () => {
    const h = health({ market_account: { state: 'idle', message: 'not linked', last_success: null } })
    expect(reportBlockVisible(h)).toBe(false)
  })

  it('ignores the whole fresh baseline: no game, no log, catalog still loading', () => {
    const h = health({
      game_reader: { state: 'degraded', message: 'Warframe is not running', last_success: null },
      log_monitor: { state: 'degraded', message: 'EE.log not found; retrying', last_success: null },
      catalog: { state: 'degraded', message: 'Item catalog has not loaded yet', last_success: null },
      collection_prices: { state: 'degraded', message: 'Collection price dump has not loaded yet', last_success: null },
    })
    expect(reportBlockVisible(h)).toBe(false)
  })

  it('reports a degraded row that has worked this session', () => {
    expect(reportBlockVisible(health({ market: { state: 'degraded', message: 'Market offline', last_success: '2026-07-27' } }))).toBe(true)
  })

  it('does not report a degraded row that never worked, even with the game running', () => {
    const h = health({
      game_reader: { state: 'ready', message: 'ok', last_success: null },
      log_monitor: { state: 'degraded', message: 'EE.log not found', last_success: null },
    })
    expect(reportBlockVisible(h)).toBe(false)
  })

  it('reports a game row that worked then degraded mid-session', () => {
    expect(reportBlockVisible(health({ game_reader: { state: 'degraded', message: 'Warframe is not running', last_success: '123' } }))).toBe(true)
  })

  it('reports any row that failed', () => {
    expect(reportBlockVisible(health({ catalog: { state: 'failed', message: 'no catalog', last_success: null } }))).toBe(true)
    expect(reportBlockVisible(health({ game_reader: { state: 'failed', message: 'reader crashed', last_success: null } }))).toBe(true)
  })

  it('reports a failed acquisition stage even without a game session', () => {
    expect(reportBlockVisible(health({ acquisition_stages: [{ stage: 'schema_validation', state: 'failed', message: 'bad' }] }))).toBe(true)
  })

  it('hides an idle acquisition stage before the game has ever worked', () => {
    expect(reportBlockVisible(health({ acquisition_stages: [{ stage: 'game_discovery', state: 'idle', message: 'Warframe is not running' }] }))).toBe(false)
  })

  it('hides an idle acquisition stage even after the game has worked this session', () => {
    const h = health({
      game_reader: { state: 'ready', message: 'ok', last_success: '123' },
      acquisition_stages: [{ stage: 'game_discovery', state: 'idle', message: 'Warframe is not running' }],
    })
    expect(reportBlockVisible(h)).toBe(false)
  })
})