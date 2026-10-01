import { invoke } from '@tauri-apps/api/core'

export type HealthState = 'ready' | 'idle' | 'degraded' | 'failed'
export interface BackendHealth { state: HealthState; message: string; last_success: string | null }
export interface AcquisitionStageHealth { stage: string; state: HealthState; message: string }
export type ItemCategory = 'frame' | 'weapon' | 'companion' | 'prime_part' | 'relic' | 'resource' | 'blueprint' | 'vehicle' | 'mod' | 'arcane'
export interface CollectionItem { id: string; name: string; category: ItemCategory; quantity: number; mastered: boolean; image_url?: string; platinum?: number; platinum_ceiling?: number; ducats?: number; rank?: number; max_rank?: number; live: boolean; priceable: boolean; monthly_trades?: number }
/** How far the live pricing pass the player asked for has got. */
export interface PricingProgress { done: number; total: number }
export interface RewardCard { name: string; platinum: number; ducats: number; owned: number; mastery?: MasteryMark; confidence: number }
/** One slot of the set a Prime item is built from: the copies the recipe takes, the copies held, and whether this reward is the one that fills it. */
export interface SetPart { name: string; image: string | null; uses: number; held: number; this: boolean }
/** Absent for items mastery never touches or while marks are off; `unknown` still replaces the
 * ownership line, as a dash. `subject` names the consumer a mastered part is still wanted by. */
export type MasteryMark =
  | { state: 'mastered' }
  | { state: 'built'; rank: number; max_rank: number }
  | { state: 'in_foundry' }
  | { state: 'unmastered'; subject: string | null; parts: SetPart[]; missing: boolean; completes: boolean }
  | { state: 'unknown' }
export type LinkState = 'unlinked' | 'linked' | 'needs_relink'
export type CredentialBacking = 'keyring' | 'database'
export type Presence = 'online' | 'ingame' | 'invisible'
/** `status: null` is offline: no socket held. */
export interface PresenceView { status: Presence | null; wanted: Presence | null; auto: boolean }
export type OrderStatus =
  | { state: 'ok' }
  | { state: 'missing' }
  | { state: 'overshoot'; owned: number }
  | { state: 'unverifiable' }
export interface MarketOrder { id: string; item_id: string; kind: 'sell' | 'buy'; platinum: number; quantity: number; per_trade: number; rank?: number; subtype?: string; visible: boolean; updated_at?: number }
export interface ReconciledOrder { order: MarketOrder; name?: string; row_id?: string; status: OrderStatus }
export interface MarketAccount { link: LinkState; backing?: CredentialBacking; orders: ReconciledOrder[]; fetched_at?: string; listed_platinum: number; flagged: number; listable: string[]; presence: PresenceView }

export interface AppView {
  collection: {
    items: CollectionItem[]
    total_entries: number
    snapshot?: { observed_at: number; game_build: string; source: string } | null
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
export type AccessMode = 'companion' | 'overlay' | 'full'
export interface SetupStatus {
  setup_complete: boolean
  access_mode: AccessMode | null
  desktop_capture_action_available: boolean
}

/**
 * Copies of a part the recipe takes (`uses`) and copies held. The strip's fraction starts at two
 * uses: for a part needed once, the game's own owned badge beside the strip already shows the count.
 */
export interface KioskMastery { held: number; uses: number }
/** One grid tile's corner chip, kept when its price, its mastery or both resolve. */
export interface CellChip { col: number; row: number; name: string; platinum: number | null; mastery: KioskMastery | null }
/** One basket row's platinum value; the game already draws the row's ducats. */
export interface BasketChip { index: number; name: string; platinum: number | null }
/** One poller epoch's whole overlay payload. */
export interface KioskView {
  session: number
  epoch: number
  cells: CellChip[]
  basket: BasketChip[]
  total_plat: number
  scroll_dy: number
  /** `unavailable` is the preference on with no live inventory behind it; the overlay says so once. */
  mastery_status: 'off' | 'live' | 'unavailable'
}

export const getView = () => invoke<AppView>('get_view')
export const getKioskView = () => invoke<KioskView | null>('get_kiosk_view')
export const refreshInventory = () => invoke<AppView>('refresh_inventory')
export const refreshPrices = (ids: string[]) => invoke<AppView>('refresh_prices', { itemIds: ids })
export const loadFakeSession = () => invoke<AppView>('load_fake_session')
export interface CollectedReport { folder_path: string; report_text: string; ee_log_included: boolean }

export const collectReport = () => invoke<CollectedReport>('collect_report')
export const collectReportText = () => invoke<string>('collect_report_text')
/**
 * Setup status, waiting out a backend that has not finished starting.
 *
 * Tauri builds the windows before it runs the setup hook, so the webview can reach `invoke`
 * before the runtime is managed, and the first thing setup does is open SQLite, which on a cold
 * first run is slow enough to lose that race. One failure here is not "the backend is
 * unavailable", it is "the backend is still starting"; only a persistent one is worth telling the
 * player about. The retry belongs to the startup call site, which is the only call that races
 * Tauri's setup hook; periodic status polls pass one attempt.
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
export const setAccessMode = (accessMode: AccessMode) => invoke<SetupStatus>('set_access_mode', { accessMode })
export const authorizeScreenCapture = () => invoke<SetupStatus>('authorize_screen_capture')

/** Overlay preferences the backend owns; the overlays never read browser storage for them. */
export interface Preferences { mastery_marks: boolean }
export const getPreferences = () => invoke<Preferences>('get_preferences')
export const setMasteryMarks = (enabled: boolean) => invoke<Preferences>('set_mastery_marks', { enabled })

export const marketStatus = () => invoke<AppView>('market_status')
export const marketSignIn = (email: string, password: string) => invoke<AppView>('market_sign_in', { email, password })
export const marketLinkToken = (token: string) => invoke<AppView>('market_link_token', { token })
export const marketSignOut = () => invoke<AppView>('market_sign_out')
export const refreshOrders = () => invoke<AppView>('refresh_orders')
export const removeOrder = (orderId: string) => invoke<AppView>('remove_order', { orderId })
export const setOrderQuantity = (orderId: string) => invoke<AppView>('set_order_quantity', { orderId })
export const setMarketPresence = (status: Presence | null, auto: boolean) =>
  invoke<AppView>('set_market_presence', { status, auto })
export const createOrder = (collectionId: string, platinum: number, quantity: number, visible: boolean) =>
  invoke<AppView>('create_order', { collectionId, platinum, quantity, visible })
export const updateOrder = (orderId: string, platinum: number, quantity: number) =>
  invoke<AppView>('update_order', { orderId, platinum, quantity })

export type InstallKind = 'appimage' | 'system_linux' | 'portable_win' | 'system_win' | 'unknown'
export interface VersionInfo { version: string; channel: string; kind: InstallKind; writable: boolean; updatable: boolean; manager: string | null; manager_command: string | null }
export interface UpdateSummary { version: string; current_version: string; notes: string | null; date: string | null; feed: string }
export interface CheckResult { kind: InstallKind; updatable: boolean; update: UpdateSummary | null }
export const getVersionInfo = () => invoke<VersionInfo>('get_version_info')
export const updateCheck = (feed: string) => invoke<CheckResult>('update_check', { feed })
export const updateDownloadAndInstall = (feed: string, expectedVersion: string) =>
  invoke<UpdateSummary>('update_download_and_install', { feed, expectedVersion })
