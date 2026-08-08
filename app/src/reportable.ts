import type { AppView, HealthState } from './backend'

/**
 * Whether the Report block should appear for the current health view.
 *
 * A row is reportable when it failed, or when it degraded after having
 * worked at some point this session (a `last_success` stamp exists). The
 * boot baseline — waiting for a game, no EE.log found, catalog and price
 * dump still loading — is degraded without a stamp and is not a fault.
 */
export function reportBlockVisible(health: AppView['health']): boolean {
  const rowsBroken = (Object.entries(health) as Array<[string, unknown]>).some(([, value]) => {
    if (Array.isArray(value)) return false
    const row = value as { state: HealthState; last_success: string | null }
    return row.state === 'failed' || (row.state === 'degraded' && row.last_success !== null)
  })
  const stagesBroken = health.acquisition_stages.some(
    (stage) => stage.state === 'degraded' || stage.state === 'failed',
  )
  return rowsBroken || stagesBroken
}