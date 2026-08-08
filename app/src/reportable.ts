import type { AppView, HealthState } from './backend'

const GAME_REQUIRED_ROWS = new Set<keyof AppView['health']>(['game_reader', 'log_monitor'])

function worries(state: HealthState): boolean {
  return state === 'degraded' || state === 'failed'
}

/**
 * Whether the Report block should appear for the current health view.
 *
 * The game-gate rows (game_reader, log_monitor) are degraded by design
 * whenever the game is closed; while the game is closed, degradation alone
 * is not a fault to report. A failed game row is always a fault: the failure
 * can persist after the game exits (for example a failed acquisition leaves
 * its row failed). Independent rows and acquisition stages carry their own
 * verdicts.
 */
export function reportBlockVisible(health: AppView['health']): boolean {
  const gameRunning = health.game_reader.state === 'ready'
  const rows = (Object.keys(health) as Array<keyof AppView['health']>).filter(
    (row) => !GAME_REQUIRED_ROWS.has(row),
  )
  const independentBroken = rows.some((row) => {
    const value = health[row]
    return !Array.isArray(value) && worries(value.state)
  })
  const gameBroken =
    (health.game_reader.state === 'failed' || health.log_monitor.state === 'failed') ||
    ((worries(health.game_reader.state) || worries(health.log_monitor.state)) && gameRunning)
  const stagesBroken = health.acquisition_stages.some((stage) => worries(stage.state))
  return independentBroken || gameBroken || stagesBroken
}