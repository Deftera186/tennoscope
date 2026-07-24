import { invoke } from '@tauri-apps/api/core'

export type HealthState = 'ready' | 'degraded' | 'failed'
export interface BackendHealth { state: HealthState; message: string; last_success: string | null }
export interface AcquisitionStageHealth { stage: string; state: HealthState; message: string }
export interface AppView {
  collection: {
    items: Array<{ id: string; name: string; category: string; quantity: number; mastered: boolean }>
    total_entries: number
  }
  reward: { cards: unknown[]; best_value_index: number | null }
  health: {
    game_reader: BackendHealth
    log_monitor: BackendHealth
    capture: BackendHealth
    catalog: BackendHealth
    market: BackendHealth
    database: BackendHealth
    acquisition_stages: AcquisitionStageHealth[]
  }
}
export interface SetupStatus { risk_accepted: boolean }

export const getView = () => invoke<AppView>('get_view')
export const refreshInventory = () => invoke<AppView>('refresh_inventory')
export const loadFakeSession = () => invoke<AppView>('load_fake_session')
export const getSetupStatus = () => invoke<SetupStatus>('get_setup_status')
export const acceptRiskDisclosure = () => invoke<SetupStatus>('accept_risk_disclosure')
