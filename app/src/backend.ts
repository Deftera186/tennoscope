import { invoke } from '@tauri-apps/api/core'

export type HealthState = 'ready' | 'idle' | 'degraded' | 'failed'
export interface BackendHealth { state: HealthState; message: string; last_success: string | null }
export interface AcquisitionStageHealth { stage: string; state: HealthState; message: string }
export type ItemCategory = 'frame' | 'weapon' | 'companion' | 'prime_part' | 'relic' | 'resource' | 'blueprint' | 'vehicle' | 'mod' | 'arcane'
export interface CollectionItem { id: string; name: string; category: ItemCategory; quantity: number; mastered: boolean; image_url?: string; platinum?: number; platinum_ceiling?: number; rank?: number; max_rank?: number; live: boolean; priceable: boolean; monthly_trades?: number }
/** How far the live pricing pass the player asked for has got. */
export interface PricingProgress { done: number; total: number }
export interface RewardCard { name: string; platinum: number; ducats: number; owned: number; mastery_relevant: boolean; confidence: number }
export type LinkState = 'unlinked' | 'linked' | 'needs_relink'
export type CredentialBacking = 'keyring' | 'database'
export type Presence = 'online' | 'ingame' | 'invisible'
/** `status: null` is offline — no socket held. */
export interface PresenceView { status: Presence | null; wanted: Presence | null; auto: boolean }
export type OrderStatus =
  | { state: 'ok' }
  | { state: 'missing' }
  | { state: 'overshoot'; owned: number }
  | { state: 'unverifiable' }
export interface MarketOrder { id: string; item_id: string; kind: 'sell' | 'buy'; platinum: number; quantity: number; per_trade: number; rank?: number; subtype?: string; visible: boolean; updated_at?: string }
export interface ReconciledOrder { order: MarketOrder; name?: string; status: OrderStatus }
export interface MarketAccount { link: LinkState; backing?: CredentialBacking; orders: ReconciledOrder[]; fetched_at?: string; listed_platinum: number; flagged: number; listable: string[]; presence: PresenceView }

export interface AppView {
  collection: {
    items: CollectionItem[]
    total_entries: number
    snapshot?: { observed_at: string; game_build: string; source: string } | null
    pricing?: PricingProgress | null
  }
  reward: { cards: RewardCard[]; best_value_index: number | null; best_ducat_index: number | null }
  market_account: MarketAccount
  health: {
    game_reader: BackendHealth
    log_monitor: BackendHealth
    capture: BackendHealth
    catalog: BackendHealth
    market: BackendHealth
    collection_prices: BackendHealth
    database: BackendHealth
    market_account: BackendHealth
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

export const marketStatus = () => invoke<AppView>('market_status')
export const marketSignIn = (email: string, password: string) => invoke<AppView>('market_sign_in', { email, password })
export const marketLinkToken = (token: string) => invoke<AppView>('market_link_token', { token })
export const marketSignOut = () => invoke<AppView>('market_sign_out')
export const refreshOrders = () => invoke<AppView>('refresh_orders')
export const removeOrder = (orderId: string) => invoke<AppView>('remove_order', { orderId })
export const setOrderQuantity = (orderId: string) => invoke<AppView>('set_order_quantity', { orderId })
export const setMarketPresence = (status: Presence | null, auto: boolean) =>
  invoke<AppView>('set_market_presence', { status, auto })
export const createOrder = (catalogPath: string, platinum: number, quantity: number, visible: boolean) =>
  invoke<AppView>('create_order', { catalogPath, platinum, quantity, visible })
