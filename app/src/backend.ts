import { invoke } from '@tauri-apps/api/core'

export type HealthState = 'ready' | 'idle' | 'degraded' | 'failed'
export interface BackendHealth { state: HealthState; message: string; last_success: string | null }
export interface AcquisitionStageHealth { stage: string; state: HealthState; message: string }
export type ItemCategory = 'frame' | 'weapon' | 'companion' | 'prime_part' | 'relic' | 'resource' | 'blueprint' | 'vehicle'
export interface CollectionItem { id: string; name: string; category: ItemCategory; quantity: number; mastered: boolean; image_url?: string }
export interface RewardCard { name: string; platinum: number; ducats: number; owned: number; mastery_relevant: boolean; confidence: number }
export interface AppView {
  collection: {
    items: CollectionItem[]
    total_entries: number
    snapshot?: { observed_at: string; game_build: string; source: string } | null
  }
  reward: { cards: RewardCard[]; best_value_index: number | null; best_ducat_index: number | null }
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
