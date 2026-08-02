import { invoke } from '@tauri-apps/api/core'

export type HealthState = 'ready' | 'idle' | 'degraded' | 'failed'
export interface BackendHealth { state: HealthState; message: string; last_success: string | null }
export interface AcquisitionStageHealth { stage: string; state: HealthState; message: string }
export type ItemCategory = 'frame' | 'weapon' | 'companion' | 'prime_part' | 'relic' | 'resource' | 'blueprint' | 'vehicle' | 'mod' | 'arcane'
export interface CollectionItem { id: string; name: string; category: ItemCategory; quantity: number; mastered: boolean; image_url?: string; platinum?: number; platinum_ceiling?: number; rank?: number; max_rank?: number; live: boolean; priceable: boolean; monthly_trades?: number }
/** How far the live pricing pass the player asked for has got. */
export interface PricingProgress { done: number; total: number }
export interface RewardCard { name: string; platinum: number; ducats: number; owned: number; mastery_relevant: boolean; confidence: number }
export interface AppView {
  collection: {
    items: CollectionItem[]
    total_entries: number
    snapshot?: { observed_at: string; game_build: string; source: string } | null
    pricing?: PricingProgress | null
  }
  reward: { cards: RewardCard[]; best_value_index: number | null; best_ducat_index: number | null }
  health: {
    game_reader: BackendHealth
    log_monitor: BackendHealth
    capture: BackendHealth
    catalog: BackendHealth
    market: BackendHealth
    collection_prices: BackendHealth
    database: BackendHealth
    acquisition_stages: AcquisitionStageHealth[]
  }
}
export interface SetupStatus { risk_accepted: boolean }

export const getView = () => invoke<AppView>('get_view')
export const refreshInventory = () => invoke<AppView>('refresh_inventory')
export const refreshPrices = (ids: string[]) => invoke<AppView>('refresh_prices', { itemIds: ids })
export const loadFakeSession = () => invoke<AppView>('load_fake_session')
/**
 * Setup status, waiting out a backend that has not finished starting.
 *
 * Tauri builds the windows before it runs the setup hook, so the webview can reach `invoke`
 * before the runtime is managed -- and the first thing setup does is open SQLite, which on a cold
 * first run is slow enough to lose that race. One failure here is not "the backend is
 * unavailable", it is "the backend is still starting"; only a persistent one is worth telling the
 * player about. This is the first call the app makes, so the retry belongs here rather than at
 * the one call site.
 */
export async function getSetupStatus(attempts = 12, delayMs = 250): Promise<SetupStatus> {
  for (let attempt = 1; ; attempt++) {
    try {
      return await invoke<SetupStatus>('get_setup_status')
    } catch (error) {
      if (attempt >= attempts) throw error
      await new Promise(resolve => setTimeout(resolve, delayMs))
    }
  }
}
export const acceptRiskDisclosure = () => invoke<SetupStatus>('accept_risk_disclosure')
