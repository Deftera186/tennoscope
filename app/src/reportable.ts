import type { AppView, HealthState } from './backend'

/**
 * Rows that report idle until something actually uses them, so a degraded
 * state always means a real failure rather than a boot baseline.
 *
 * The backend carries the previous `last_success` forward when these degrade,
 * so the first failure of a session has no stamp. Gating them on one hid
 * exactly the reports worth having: a reward screen that never read cleanly
 * stayed silent for the whole session.
 */
const REPORTABLE_WHEN_FIRST_DEGRADED = new Set(['capture', 'market', 'collection_prices'])

/**
 * Whether the Report block should appear for the current health view.
 *
 * A row is reportable when it failed, or when it degraded and that degradation
 * is a fault rather than a state the app reaches on its own.
 *
 * The rows in `REPORTABLE_WHEN_FIRST_DEGRADED` need no `last_success` stamp:
 * they sit idle until used, so degraded already means something failed. The
 * remaining rows keep the stamp requirement, because they can degrade while
 * still working — a catalog served from cache is the clear case — and a stamp
 * is what separates "worked, then broke" from a state nobody needs to report.
 */
export function reportBlockVisible(health: AppView['health']): boolean {
  const rowsBroken = (Object.entries(health) as Array<[string, unknown]>).some(([key, value]) => {
    if (Array.isArray(value)) return false
    const row = value as { state: HealthState; last_success: string | null }
    if (row.state === 'failed') return true
    if (row.state !== 'degraded') return false
    return REPORTABLE_WHEN_FIRST_DEGRADED.has(key) || row.last_success !== null
  })
  const stagesBroken = health.acquisition_stages.some((stage) => {
    if (stage.state === 'failed') return true
    return stage.state === 'degraded' && health.game_reader.last_success !== null
  })
  return rowsBroken || stagesBroken
}