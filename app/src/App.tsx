import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from 'react'
import './App.css'
import {
  authorizeScreenCapture,
  getSetupStatus,
  getView,
  marketLinkToken,
  marketSignIn,
  marketSignOut,
  marketStatus,
  refreshInventory,
  refreshOrders,
  refreshPrices,
  removeOrder,
  createOrder,
  updateOrder,
  setMarketPresence,
  setAccessMode,
  setOrderQuantity,
  type AppView,
  type BackendHealth,
  type AccessMode,
  type CollectionItem,
  type HealthState,
  type ItemCategory,
  type SetupStatus,
  type MarketOrder,
  type Presence,
} from './backend'
import { hideRewardOverlay, showRewardOverlay } from './overlay'
import { copyReport, openIssue, saveReport } from './report'
import { closeWindow, minimizeWindow, readWindowMaximized, toggleMaximizeWindow, watchWindowResized } from './window'
import { RewardCards } from './RewardCards'
import { MetalMark } from './MetalMark'
import { OrdersView } from './OrdersView'
import { isListable, listedLabel, listedOrderFor } from './orders'
import { SellForm, type SellHandler, type UpdateHandler } from './SellForm'
import { atMaxRank, clampPage, collectionTotals, COLLECTION_PAGE_SIZE, pageCount, pageItems, pageNumbers, rankLabel, stackValue } from './collection'
import { MAX_PRICE_FLOOR, readPriceFloor, readShowDucats, writePriceFloor, writeShowDucats } from './settings'
import { snapshotFreshness, stampReading } from './freshness'
import { reportBlockVisible } from './reportable'
import { AccessSelector } from './AccessSelector'
import { accessMode } from './access'

type Page = 'collection' | 'rewards' | 'orders' | 'diagnostics' | 'settings' | 'about'
type Ownership = 'all' | 'owned' | 'mastered' | 'missing' | 'tradeable'
type Sort = 'name-asc' | 'quantity-desc' | 'category-asc' | 'platinum-desc' | 'ducats-desc'

const categories: Array<{ value: ItemCategory | 'all'; label: string; tally: string }> = [
  { value: 'all', label: 'All categories', tally: '✳' },
  { value: 'frame', label: 'Frame', tally: 'F' },
  { value: 'weapon', label: 'Weapon', tally: 'W' },
  { value: 'companion', label: 'Companion', tally: 'C' },
  { value: 'prime_part', label: 'Prime Parts', tally: 'P' },
  { value: 'relic', label: 'Relic', tally: 'R' },
  { value: 'resource', label: 'Resource', tally: 'S' },
  { value: 'blueprint', label: 'Blueprint', tally: 'B' },
  { value: 'vehicle', label: 'Vehicle', tally: 'V' },
  { value: 'mod', label: 'Mod', tally: 'M' },
  { value: 'arcane', label: 'Arcane', tally: 'A' },
]

// The two value sorts are named and marked by their own metal: "Value" stopped answering once a
// card could carry two prices, and a currency's own icon is the disambiguator this screen already
// teaches. Ducats belongs to the ducat layer -- offered only while the values are on screen.
const sortOptions: Array<{ value: Sort; label: string; metal?: 'plat' | 'ducat' }> = [
  { value: 'name-asc', label: 'Name A–Z' },
  { value: 'quantity-desc', label: 'Quantity' },
  { value: 'category-asc', label: 'Category' },
  { value: 'platinum-desc', label: 'Platinum', metal: 'plat' },
  { value: 'ducats-desc', label: 'Ducats', metal: 'ducat' },
]

const categoryName = Object.fromEntries(categories.map(category => [category.value, category.label])) as Record<ItemCategory | 'all', string>

const monthAbbr = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec']

/** `2026-07-27` -> `27 Jul`. Formatted by hand rather than `Intl`, whose month/day order follows
 * the runtime locale -- this reading has to look the same on every machine it runs on. */
function shortDumpDate(isoDate: string): string {
  const [, month, day] = isoDate.split('-').map(Number)
  return `${day} ${monthAbbr[month - 1]}`
}

const pageLabel: Record<Page, string> = {
  collection: 'Collection',
  rewards: 'Rewards',
  orders: 'Orders',
  diagnostics: 'Diagnostics',
  settings: 'Settings',
  about: 'About',
}

/**
 * Assay marks, drawn in the world's own grammar: hard geometry, square caps,
 * no rounded joins. The rewards glyph is the orb of the platinum standard mark.
 */
function Mark({ name, className = 'punch-glyph' }: { name: Page | 'refresh' | 'search'; className?: string }) {
  const paths = {
    collection: <><path d="M3 4h18M3 10h13M3 16h18M3 22h9"/></>,
    rewards: <><circle cx="12" cy="14.5" r="7.5"/><path d="M12 7V1.5M9 4h6"/></>,
    orders: <><path d="M3 3h18v6H3z"/><path d="M6 9v12h12V9M10 13h4"/></>,
    diagnostics: <><path d="M2 21h20M6 21 14 3M11 21 19 3"/></>,
    settings: <><path d="M7 2h10l-2 9H9z"/><path d="M10 11h4v11h-4z"/></>,
    // The office's own hallmark cartouche, which is what this page is: the register's statement
    // about itself.
    about: <><path d="M4 3h16v11.5L12 21 4 14.5z"/><path d="M12 8v6"/></>,
    refresh: <><path d="M21 5v6h-6"/><path d="M20 11a8 8 0 1 0-1.5 6"/></>,
    search: <><circle cx="10.5" cy="10.5" r="7"/><path d="M15.5 15.5 22 22"/></>,
  }
  return <svg className={className} viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="square" strokeLinejoin="miter">{paths[name]}</svg>
}

/**
 * Minimize, maximize and close, drawn in the same square-stroke grammar as the page marks. The
 * maximize control is named by what it does next -- restore while maximized -- because a button
 * whose name never changes cannot say which press undoes the other.
 */
function WindowControls() {
  const [maximized, setMaximized] = useState(false)
  useEffect(() => {
    let active = true
    let unlisten: (() => void) | undefined
    void readWindowMaximized().then(value => { if (active) setMaximized(value) })
    void watchWindowResized(() => {
      void readWindowMaximized().then(value => { if (active) setMaximized(value) })
    }).then(fn => {
      if (active) unlisten = fn
      else fn()
    })
    return () => {
      active = false
      unlisten?.()
    }
  }, [])
  return <div className="window-controls" role="group" aria-label="Window">
    <button type="button" className="window-control" aria-label="Minimize window" onClick={() => { void minimizeWindow() }}>
      <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M1 5h8"/></svg>
    </button>
    <button type="button" className="window-control" aria-label={maximized ? 'Restore window' : 'Maximize window'} onClick={() => { void toggleMaximizeWindow() }}>
      {maximized
        ? <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M3.5 3.5h5v5h-5zM6.5 3.5v-2h-5v5h2"/></svg>
        : <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M1.5 1.5h7v7h-7z"/></svg>}
    </button>
    <button type="button" className="window-control close" aria-label="Close window" onClick={() => { void closeWindow() }}>
      <svg viewBox="0 0 10 10" aria-hidden="true"><path d="M1 1l8 8M9 1L1 9"/></svg>
    </button>
  </div>
}

function App() {
  const [setupStatus, setSetupStatus] = useState<SetupStatus | null>(null)
  const [selectedMode, setSelectedMode] = useState<AccessMode>('full')
  const [modeBusy, setModeBusy] = useState(false)
  const [modeError, setModeError] = useState<string | null>(null)
  const [view, setView] = useState<AppView | null>(null)
  const [page, setPage] = useState<Page>('collection')
  const [busy, setBusy] = useState(false)
  const [pricing, setPricing] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [clock, setClock] = useState(() => new Date())
  const [priceFloor, setPriceFloor] = useState(readPriceFloor)
  const [showDucats, setShowDucats] = useState(readShowDucats)
  const [ordersBusy, setOrdersBusy] = useState(false)
  const [ordersError, setOrdersError] = useState<string | null>(null)
  const [ordersNote, setOrdersNote] = useState<string | null>(null)
  const [captureNote, setCaptureNote] = useState<string | null>(null)
  const viewGeneration = useRef(0)
  const foregroundInFlight = useRef(0)
  const setupGeneration = useRef(0)
  const captureAuthorizationInFlight = useRef(false)
  const modeTransitionInFlight = useRef(false)
  const effectiveModeRef = useRef<AccessMode | null>(null)
  const [captureAuthorizationBusy, setCaptureAuthorizationBusy] = useState(false)

  const requestView = useCallback(async (request: () => Promise<AppView>, failure: string) => {
    const generation = ++viewGeneration.current
    try {
      const next = await request()
      if (generation === viewGeneration.current) {
        setView(next)
        setError(null)
      }
    } catch {
      if (generation === viewGeneration.current) setError(failure)
    }
  }, [])

  const runForeground = useCallback(async (operation: () => Promise<void>) => {
    foregroundInFlight.current += 1
    // A write owns the setup state it returns. Retire any poll that started before the write so
    // its older snapshot cannot land after the operation has completed.
    setupGeneration.current += 1
    try { await operation() }
    finally { foregroundInFlight.current -= 1 }
  }, [])

  const requestMarketStatus = useCallback(async () => {
    try {
      const next = await marketStatus()
      // Merge rather than replace: `getView` at startup is the source of truth for everything
      // else, and adopting only the fields this command owns keeps the two calls independent
      // races instead of one clobbering the other's result.
      setView(current => current ? { ...current, market_account: next.market_account, health: { ...current.health, market_account: next.health.market_account } } : next)
    } catch {
      // Startup already surfaces a failure via `getView` if the backend is down; a market-status
      // miss on top of that would just repeat the same alert.
    }
  }, [])

  useEffect(() => {
    getSetupStatus()
      .then(async status => {
        effectiveModeRef.current = status.access_mode
        setSetupStatus(status)
        if (status.access_mode) setSelectedMode(status.access_mode)
        if (status.setup_complete) {
          await requestView(getView, 'The local application backend is unavailable.')
          await requestMarketStatus()
        }
      })
      .catch(() => setError('The local application backend is unavailable.'))
  }, [requestView, requestMarketStatus])

  useEffect(() => {
    if (!setupStatus?.setup_complete) return
    let active = true
    let timer: ReturnType<typeof setTimeout> | undefined
    const schedule = () => { if (active) timer = setTimeout(poll, 2500) }
    const poll = async () => {
      if (document.hidden || foregroundInFlight.current > 0) { schedule(); return }
      const generation = ++setupGeneration.current
      await Promise.all([
        requestView(getView, 'The live backend view could not be updated.'),
        getSetupStatus(1).then(status => {
          if (active && generation === setupGeneration.current) {
            const modeChanged = status.access_mode !== effectiveModeRef.current
            effectiveModeRef.current = status.access_mode
            setSetupStatus(status)
            if (modeChanged && status.access_mode && !modeTransitionInFlight.current) {
              setSelectedMode(status.access_mode)
            }
          }
        }).catch(() => {
          // `getView` owns the shared backend failure banner. Keep the last known capture action
          // rather than hiding it because one capability poll failed.
        }),
      ])
      schedule()
    }
    schedule()
    return () => {
      active = false
      setupGeneration.current += 1
      viewGeneration.current += 1
      clearTimeout(timer)
    }
  }, [setupStatus?.setup_complete, requestView])

  useEffect(() => {
    const timer = setInterval(() => setClock(new Date()), 30_000)
    return () => clearInterval(timer)
  }, [])

  async function changeAccessMode(next: AccessMode) {
    if (modeTransitionInFlight.current) return
    modeTransitionInFlight.current = true
    const previous = setupStatus?.access_mode ?? null
    setModeBusy(true)
    setModeError(null)
    try {
      await runForeground(async () => {
        const status = await setAccessMode(next)
        effectiveModeRef.current = status.access_mode
        setSetupStatus(status)
        if (status.access_mode) setSelectedMode(status.access_mode)
        await requestView(getView, 'The local application backend is unavailable.')
        // Reconciliation depends on the effective mode: without this a Full-to-Companion
        // downgrade keeps Full-era verified ownership and sellable flags until the player
        // happens to open Orders. The helper merges only what it owns and stays silent on
        // failure, so it is safe on every successful transition, not just first run.
        await requestMarketStatus()
      })
    } catch {
      const active = previous ? accessMode(previous).name : 'No mode'
      // Keep the pending selection on every failure direction so retry is one Confirm click;
      // the effective mode beside it still names what is actually active.
      setModeError(`Could not change Warframe access. ${active} remains active. Review the mode and try again.`)
    } finally {
      modeTransitionInFlight.current = false
      setModeBusy(false)
    }
  }

  async function authorizeCapture() {
    if (captureAuthorizationInFlight.current) return
    captureAuthorizationInFlight.current = true
    setCaptureAuthorizationBusy(true)
    // The desktop chooser is human-paced and may remain open indefinitely. Retire setup polls
    // already in flight, but leave view polling alive while the chooser owns the user's attention.
    const generation = ++setupGeneration.current
    try {
      const status = await authorizeScreenCapture()
      if (generation === setupGeneration.current) {
        effectiveModeRef.current = status.access_mode
        setSetupStatus(status)
      }
    } finally {
      captureAuthorizationInFlight.current = false
      setCaptureAuthorizationBusy(false)
    }
  }
  async function refresh() {
    setBusy(true)
    setError(null)
    await runForeground(() => requestView(refreshInventory, 'Inventory refresh failed. Check diagnostics for acquisition health.'))
    setBusy(false)
  }

  /**
   * Deliberately outside `runForeground`: a page refresh prices up to forty-eight items at three
   * requests a second, so it is on the wire for about sixteen seconds, and the whole promise of it
   * is that prices appear as they land. That only happens if the 2.5s poll keeps running through
   * it. Ordering remains coherent -- `requestView` applies a response only while its request is
   * newest one started, so an older view can never land on top of a newer one.
   *
   * The local flag exists only to hold the control down for the up-to-2.5s gap before the poll
   * carries the backend's own progress. The counting is the backend's: it is the only party that
   * knows the total, and it publishes the count the same way for every pass.
   */
  async function priceLive(ids: string[]) {
    setPricing(true)
    await requestView(() => refreshPrices(ids), 'Live prices could not be fetched.')
    setPricing(false)
  }

  /**
   * Every market write goes through here: fresh view on success, a banner on failure. The optional
   * note is spoken once, on success only -- the badge appearing is the sighted player's
   * confirmation, and the note is the same confirmation for anyone not looking at it.
   */
  async function ordersOperation(
    operation: () => Promise<AppView>,
    failure: string,
    note?: (next: AppView) => string,
  ) {
    setOrdersBusy(true)
    setOrdersError(null)
    try {
      const next = await operation()
      setView(next)
      setOrdersNote(note ? note(next) : null)
    } catch {
      setOrdersError(failure)
    } finally {
      setOrdersBusy(false)
    }
  }

  const ordersSignIn = (email: string, password: string) =>
    ordersOperation(() => marketSignIn(email, password), 'Could not sign in to warframe.market.')
  const ordersLinkToken = (token: string) =>
    ordersOperation(() => marketLinkToken(token), 'Could not link with that token.')
  const ordersSignOut = () =>
    ordersOperation(marketSignOut, 'Could not unlink the account.')
  const ordersRefresh = () =>
    ordersOperation(refreshOrders, 'warframe.market could not be reached.')
  const ordersRemove = (orderId: string) =>
    ordersOperation(() => removeOrder(orderId), 'Could not remove that listing.')
  const ordersLowerTo = (orderId: string, _quantity: number) =>
    ordersOperation(() => setOrderQuantity(orderId), 'Could not lower that listing.')
  const ordersPresence = (status: Presence | null, auto: boolean) =>
    ordersOperation(() => setMarketPresence(status, auto), 'Could not change your market status.')
  /** The card's own accessible name: the rank belongs in it because a mod held at two ranks is two
   * cards headed the same word. Shared with the spoken note, so the note says what the card is
   * named. */
  function cardLabel(item: CollectionItem): string {
    return rankLabel(item) ? `${item.name}, ${rankLabel(item)}` : item.name
  }

  const ordersSell = (collectionId: string, platinum: number, quantity: number, visible: boolean) =>
    ordersOperation(
      () => createOrder(collectionId, platinum, quantity, visible),
      'Could not publish that listing.',
      next => {
        const item = next.collection.items.find(entry => entry.id === collectionId)
        return `Listed ${item ? cardLabel(item) : 'the item'} at ${platinum} platinum × ${quantity}`
      },
    )
  const ordersUpdate = (orderId: string, platinum: number, quantity: number) =>
    ordersOperation(
      () => updateOrder(orderId, platinum, quantity),
      'Could not update that listing.',
      next => {
        const name = next.market_account.orders.find(entry => entry.order.id === orderId)?.name
        return `Listing updated: ${name ?? 'the item'} at ${platinum} platinum × ${quantity}`
      },
    )

  function openPage(next: Page) {
    setPage(next)
    if (next === 'orders') void ordersRefresh()
  }

  if (setupStatus === null && !error) return <main className="holding"><div className="streak" aria-hidden="true"/><p className="register-line">Starting TennoScope…</p></main>
  if (!setupStatus?.setup_complete) return <SetupScreen selected={selectedMode} busy={modeBusy} error={modeError ?? error} onSelect={setSelectedMode} onContinue={() => { void changeAccessMode(selectedMode) }}/>


  const effectiveMode = setupStatus.access_mode ?? 'companion'
  const liveState = view?.health.game_reader.state ?? 'degraded'
  const freshness = snapshotFreshness(view?.collection.snapshot, clock)
  return <div className="assay">
    <header className="masthead">
      {/* Window chrome, destination choice, and runtime operations each keep a dedicated row. This
          leaves every destination visible at the shipped 1180px width and prevents a long reader
          message or refresh action from clipping the active page. */}
      <div className="masthead-top" data-tauri-drag-region="deep">
        <div className="office">
          <span className="office-name">TennoScope</span>
          <span className="office-role">Local assay register</span>
        </div>
        <WindowControls/>
      </div>
      <div className="masthead-work">
        <nav className="hallmark-row" aria-label="Primary">
          {(['collection', 'rewards', 'orders', 'diagnostics', 'settings', 'about'] as const).map(item => <button
            key={item}
            type="button"
            aria-label={pageLabel[item]}
            className={page === item ? 'punch struck' : 'punch'}
            aria-current={page === item ? 'page' : undefined}
            onClick={() => openPage(item)}
          >
            <span className="punch-face">
              <Mark name={item}/>
              <span className="punch-name">{pageLabel[item]}</span>
              {item === 'rewards' && view?.reward.cards.length ? <em className="punch-count">{view.reward.cards.length}</em> : null}
              {item === 'orders' && view?.market_account.flagged ? <em className="punch-count">{view.market_account.flagged}</em> : null}
            </span>
          </button>)}
        </nav>
      </div>
      <section className="masthead-state" aria-label="Runtime operations">
          <div className={`assay-state ${effectiveMode === 'companion' ? 'idle' : liveState}`}>
            <span className="state-mark" aria-hidden="true"/>
            <span className="assay-state-text">
              <strong role="status">{effectiveMode === 'companion' ? 'Companion mode' : liveState === 'ready' ? 'Watching Warframe' : liveState === 'idle' ? 'Idle' : liveState === 'failed' ? 'Attention — reader failed' : 'Attention needed'}</strong>
              <small>{effectiveMode === 'companion' ? 'Using saved and reference data' : view?.health.game_reader.message ?? 'Connecting to local backend'}</small>
            </span>
          </div>
          {view && <span className="date-letter" title={freshness.detail}>{freshness.label}<span className="sr-only"> — {freshness.detail}</span></span>}
          <button type="button" className="stamp" onClick={refresh} disabled={busy || modeBusy || setupStatus?.access_mode !== 'full'} aria-describedby={setupStatus?.access_mode !== 'full' ? 'inventory-access-note' : undefined}>
            <Mark name="refresh" className="punch-glyph"/><span>{busy ? 'Refreshing…' : 'Refresh inventory'}</span>
          </button>
          {setupStatus?.access_mode !== 'full' && <span id="inventory-access-note" className="sr-only">Full access is required to acquire inventory. The saved snapshot remains available.</span>}
      </section>
    </header>

    <main className="sheet">
      {error && <p className="error-banner" role="alert">{error}</p>}
      {/* A sell can be started from a collection card, and its failure has to be readable where it
          was started. The orders screen renders this itself, in the block that owns the recovery. */}
      {ordersError && page !== 'orders' && <p className="error-banner" role="alert">{ordersError}</p>}
      {/* Always in the document, spoken only when a write leaves a word in it: a live region that
          appears and disappears with its content is not announced by every reader. */}
      <p className="sr-only" role="status">{ordersNote}</p>
      {!view ? <LoadingView/> : <>
        {page === 'collection' && <CollectionPage view={view} effectiveMode={effectiveMode} pricing={pricing} onPriceLive={priceLive} priceFloor={priceFloor} showDucats={showDucats} onToggleDucats={() => {
          setShowDucats(current => {
            writeShowDucats(!current)
            return !current
          })
        }} onSell={ordersSell} onUpdate={ordersUpdate} ordersBusy={ordersBusy}/>}
        {page === 'rewards' && <RewardPage view={view} effectiveMode={effectiveMode}/>}
        {page === 'orders' && <OrdersView
          account={view.market_account}
          onSignIn={ordersSignIn}
          onLinkToken={ordersLinkToken}
          onSignOut={ordersSignOut}
          onRefresh={ordersRefresh}
          onRemove={ordersRemove}
          onLowerTo={ordersLowerTo}
          onSell={ordersSell}
          onUpdate={ordersUpdate}
          onPresence={ordersPresence}
          items={view.collection.items}
          busy={ordersBusy}
          error={ordersError}
        />}
        {page === 'diagnostics' && <DiagnosticsPage view={view} effectiveMode={effectiveMode}/>}
        {page === 'settings' && <SettingsPage view={view} priceFloor={priceFloor} effectiveMode={effectiveMode} selectedMode={selectedMode} modeBusy={modeBusy} modeError={modeError} onSelectMode={mode => {
          setSelectedMode(mode)
          setModeError(null)
        }} onConfirmMode={() => { void changeAccessMode(selectedMode) }} desktopCaptureActionAvailable={setupStatus?.desktop_capture_action_available ?? false} captureAuthorizationBusy={captureAuthorizationBusy} captureNote={captureNote} onCaptureNote={setCaptureNote} onAuthorizeCapture={authorizeCapture} onPriceFloor={floor => {
          setPriceFloor(floor)
          writePriceFloor(floor)
        }}/>}
        {page === 'about' && <AboutPage effectiveMode={effectiveMode}/>}
      </>}
    </main>
  </div>
}

function SetupScreen({ selected, busy, error, onSelect, onContinue }: { selected: AccessMode; busy: boolean; error: string | null; onSelect: (mode: AccessMode) => void; onContinue: () => void }) {
  return <main className="certificate">
    <section className="certificate-sheet" aria-labelledby="setup-title">
      <div className="office">
        <span className="office-name">TennoScope</span>
        <span className="office-role">One-time setup</span>
      </div>
      <h1 id="setup-title" className="mark">Choose Warframe access</h1>
      <p className="prose">Each level includes the previous level. Nothing starts until you confirm.</p>
      <AccessSelector value={selected} onChange={onSelect} applyingMode={busy ? selected : null} disabled={busy}/>
      {error && <p className="error-banner" role="alert">{error}</p>}
      <button type="button" className="seal" onClick={onContinue} disabled={busy} aria-busy={busy}>
        {busy ? `Applying ${accessMode(selected).name}…` : `Confirm ${accessMode(selected).name}`}<span aria-hidden="true">→</span>
      </button>
    </section>
  </main>
}


function LoadingView() {
  return <section className="page" aria-live="polite">
    <div className="mark-head">
      <h1 className="mark">Loading your local collection…</h1>
      <p className="prose">Reading the latest saved snapshot.</p>
    </div>
    <div className="streak" aria-hidden="true"/>
  </section>
}

function CollectionPage({ view, effectiveMode, pricing, onPriceLive, priceFloor, showDucats, onToggleDucats, onSell, onUpdate, ordersBusy }: { view: AppView; effectiveMode: AccessMode; pricing: boolean; onPriceLive: (ids: string[]) => void; priceFloor: number; showDucats: boolean; onToggleDucats: () => void; onSell: SellHandler; onUpdate: UpdateHandler; ordersBusy: boolean }) {
  const ownershipVerified = effectiveMode === 'full'
  const snapshotOnly = !ownershipVerified
  const [search, setSearch] = useState('')
  const [category, setCategory] = useState<ItemCategory | 'all'>('all')
  const [ownership, setOwnership] = useState<Ownership>('all')
  const [sort, setSort] = useState<Sort>('name-asc')
  const [page, setPage] = useState(1)
  const {
    masteryEligible,
    mastered,
    owned,
    missing,
    worth,
    sellable,
    ducatsAtStake,
  } = collectionTotals(view.collection.items, priceFloor)
  const filtered = useMemo(() => {
    const query = search.trim().toLocaleLowerCase()
    return view.collection.items
      .filter(item => !query || item.name.toLocaleLowerCase().includes(query))
      .filter(item => category === 'all' || item.category === category)
      .filter(item => ownership === 'all'
        || (ownership === 'owned' && item.quantity > 0)
        || (ownership === 'mastered' && item.mastered)
        || (ownership === 'missing' && item.quantity === 0)
        || (ownership === 'tradeable' && item.platinum !== undefined))
      .toSorted((left, right) => sort === 'quantity-desc'
        ? right.quantity - left.quantity || left.name.localeCompare(right.name)
        : sort === 'category-asc'
          ? left.category.localeCompare(right.category) || left.name.localeCompare(right.name)
          : sort === 'platinum-desc'
            ? (right.platinum ?? -1) - (left.platinum ?? -1) || left.name.localeCompare(right.name)
            : sort === 'ducats-desc'
              ? (right.ducats ?? -1) - (left.ducats ?? -1) || left.name.localeCompare(right.name)
              : left.name.localeCompare(right.name))
  }, [view.collection.items, search, category, ownership, sort])
  const totalPages = pageCount(filtered.length)
  const currentPage = clampPage(page, filtered.length)
  const visibleItems = pageItems(filtered, currentPage)
  const firstResult = filtered.length ? (currentPage - 1) * COLLECTION_PAGE_SIZE + 1 : 0
  const lastResult = Math.min(currentPage * COLLECTION_PAGE_SIZE, filtered.length)
  // What the refresh can *attempt*, not what already has a number. A relic is priced only if some
  // dump in the last month saw it trade, so a thinly-traded one is unpriced, and excluding unpriced
  // items would close the manual path against exactly the items that need it. `priceable` is the backend's own answer to "can warframe.market be asked about
  // this": it drops every name the price table cannot resolve, so counting owned items instead
  // promised prices for items no request was ever going to be made for. Quantity 0 is not owned and
  // is never priceable.
  const pricableVisibleIds = visibleItems.filter(item => item.priceable).map(item => item.id)
  // One readout for the whole page, published by the backend rather than counted here: it is the
  // party that knows the total, and every pass comes out of one rate-limited budget, so a second
  // counter would be describing one queue twice.
  const inProgress = view.collection.pricing ?? null
  const dumpDate = view.health.collection_prices.last_success
  // You can only sort by what is on screen. Hiding ducat values retires their sort with them, and
  // the pressed chip moves to platinum in plain sight rather than leaving an invisible criterion
  // reorder the register.
  const sorts = showDucats ? sortOptions : sortOptions.filter(option => option.value !== 'ducats-desc')
  useEffect(() => {
    if (!showDucats && sort === 'ducats-desc') setSort('platinum-desc')
  }, [showDucats, sort])
  useEffect(() => {
    if (!ownershipVerified) setOwnership('all')
  }, [ownershipVerified])
  useEffect(() => setPage(1), [search, category, ownership, sort])
  useEffect(() => setPage(value => clampPage(value, filtered.length)), [filtered.length])

  return <section className="page" aria-labelledby="collection-title">
    <div className="mark-head">
      <h1 id="collection-title" className="mark">{snapshotOnly ? 'Saved snapshot' : 'Your collection'}</h1>
      <p className="prose">{snapshotOnly ? 'Your stored collection remains available for reference, valuation, and planning.' : 'Canonical equipment, parts and relics observed on this account. Read only, held locally.'}</p>
    </div>

    <div className={`assay-band${showDucats ? ' with-ducats' : ''}`}>
      <BandCell kind="items" value={view.collection.total_entries} label={snapshotOnly ? 'Saved entries' : 'Items tracked'} note={snapshotOnly ? 'Entries retained in the saved snapshot' : `${owned} currently owned`}/>
      <BandCell kind="mastered" value={mastered} label={snapshotOnly ? 'Mastery records' : 'Mastered'} note={snapshotOnly ? 'Historical snapshot; current mastery is unverifiable' : masteryEligible ? `${Math.round(mastered / masteryEligible * 100)}% of mastery-eligible items` : 'No mastery-eligible items'}/>
      <BandCell kind="missing" value={missing} label={snapshotOnly ? 'Zero-quantity records' : 'Missing'} note={snapshotOnly ? 'Historical snapshot; current ownership is unverifiable' : 'From known collection data'}/>
      {/* Two figures and the one clause that qualifies them. The cell had five numbers in it and
          read as an argument about the collection rather than a valuation of it; the live-pass count
          was a second copy of the register line below, and the priced-item count mostly measured how
          much of a collection is untradeable. The cap is stated here, on the figure it applies to,
          rather than down among the filters where it was answering a question nobody had asked yet. */}
      <BandCell
        kind="worth"
        value={worth}
        unit={<MetalMark metal="plat" alt=" platinum"/>}
        aside={<>{figure(sellable)}<MetalMark metal="plat" alt=" platinum"/> sellable</>}
        label={snapshotOnly ? 'Saved snapshot value' : 'Collection worth'}
        note={snapshotOnly
          ? 'Valuation uses saved quantities; current ownership is unverifiable'
          : priceFloor
            ? `Sellable counts only the copies the market buys in a month, at ${priceFloor} platinum and over`
            : 'Sellable counts only the copies the market buys in a month'}
      />
      {showDucats && <BandCell
        kind="ducats"
        value={ducatsAtStake}
        unit={<MetalMark metal="ducat" alt=" ducats"/>}
        label={snapshotOnly ? 'Saved snapshot ducats' : 'Ducats at stake'}
        note={snapshotOnly ? 'Total uses saved quantities; current ownership is unverifiable' : "Every owned prime part, at Baro Ki'Teer's posted prices"}
      />}
    </div>

    <div className="register">
      {view.collection.items.length ? <>
      <div className="register-controls">
        <label className="search-slot">
          <Mark name="search" className="punch-glyph"/>
          <span className="sr-only">Search collection</span>
          <input type="search" aria-label="Search collection" placeholder="Search canonical item names…" value={search} onChange={event => setSearch(event.target.value)}/>
        </label>
        <div className="sort-slot" role="group" aria-label="Sort collection">
          <span>Sort</span>
          <div className="tally">
            {sorts.map(option => <button
              type="button"
              key={option.value}
              aria-pressed={sort === option.value}
              onClick={() => setSort(option.value)}
            >{option.metal && <MetalMark metal={option.metal}/>}{option.label}</button>)}
          </div>
        </div>
        {/* The ducat layer's valve, drawn as a switch because it has two states rather than a
            place among the sort and filter modes beside it. The label names what it shows; on,
            its thumb crosses the track's midline in the gold every ducat reading on this sheet
            already wears. */}
        <button
          type="button"
          role="switch"
          aria-checked={showDucats}
          className="display-switch"
          onClick={onToggleDucats}
        >
          <span className="display-face"><MetalMark metal="ducat"/>Ducat values</span>
          <span className="display-track" aria-hidden="true"><span className="display-thumb"/></span>
        </button>
      </div>

      <div className="shield-strip" role="group" aria-label="Item categories">
        {categories.map(item => <button
          type="button"
          key={item.value}
          className="shield"
          aria-label={item.label}
          aria-pressed={category === item.value}
          onClick={() => setCategory(item.value)}
        ><span className="shield-face"><b aria-hidden="true">{item.tally}</b>{item.label}</span></button>)}
      </div>

      {/* The bar's own bottom rule doubles as the gauge: while a pass runs it fills with platinum
          from the left. An engraved hairline is what this system already uses to divide the sheet,
          so a reading struck into one needs no new component and nothing that spins. */}
      <div className="register-bar" style={inProgress ? { '--assay-progress': inProgress.done / Math.max(inProgress.total, 1) } as CSSProperties : undefined}>
        {ownershipVerified && <div className="tally" role="group" aria-label="Ownership filters">
          {(['all', 'owned', 'mastered', 'missing', 'tradeable'] as const).map(filter => <button
            type="button"
            key={filter}
            aria-pressed={ownership === filter}
            onClick={() => setOwnership(filter)}
          >{filter[0].toUpperCase() + filter.slice(1)}</button>)}
        </div>}
        <div className="provenance-row">
          <div className="register-status">
            <span className="register-line">{dumpDate ? `Prices from the ${shortDumpDate(dumpDate)} market summary` : 'No price summary loaded yet'}</span>
            <span className="register-line">{firstResult}–{lastResult} of {filtered.length}</span>
            {inProgress && <span className="register-line pricing" role="status">Checking live prices · {inProgress.done} of {inProgress.total}</span>}
          {/* Live prices come from the market dump and explicit price checks, neither of which
              reads the game or needs verified ownership: Companion and Overlay keep valuation.
              Selling still needs Full, and that gate lives on the sell controls, not here. */}
          <button
            type="button"
            className="stamp"
            disabled={pricing || inProgress !== null || pricableVisibleIds.length === 0}
            onClick={() => onPriceLive(pricableVisibleIds)}
          ><span>{pricing || inProgress ? 'Pricing…' : `Price these ${pricableVisibleIds.length}`}</span></button>
          </div>
        </div>
      </div>

        {filtered.length
          ? <>
            <ul className="collection-grid" aria-label="Collection items">{visibleItems.map(item => <li key={item.id}><CollectionEntry item={item} ownershipVerified={ownershipVerified} showDucats={showDucats} listedOrder={listedOrderFor(view.market_account.orders, item.id)} sellable={ownershipVerified && view.market_account.link === 'linked' && isListable(item, view.market_account.listable)} onSell={onSell} onUpdate={onUpdate} busy={ordersBusy}/></li>)}</ul>
            <Pagination current={currentPage} total={totalPages} onChange={setPage}/>
          </>
          : <EmptyState title="No matching items" detail="Try another search or clear a filter."/>}
      </> : <EmptyState
        title={snapshotOnly ? 'No saved inventory snapshot yet' : 'No inventory items yet'}
        detail={snapshotOnly ? 'Full access can acquire one; Companion remains useful for reference, prices, and planning.' : 'Start Warframe and refresh to create your first local snapshot.'}
      />}
    </div>
  </section>
}

/** Struck figures are grouped: a five-digit total is read, not counted. */
function figure(value: number): string {
  return value.toLocaleString('en-US')
}

// The worth cell sits in a row of plain counts, where a bare number reads as one more count.
// `aside` takes the slot the other cells put their note in, so four labels still strike one line
// across the band: a second figure wedged between mark and label would drop this label alone.
function BandCell({ kind, value, unit, aside, label, note }: { kind: string; value: number; unit?: ReactNode; aside?: ReactNode; label: string; note?: string }) {
  return <div className={`band-cell ${kind}`} data-summary={kind} data-testid={`band-${kind}`}>
    <span className="band-figure">{figure(value)}{unit}</span>
    <span className="band-label">{label}</span>
    {aside && <span className="band-aside">{aside}</span>}
    {note && <p className="band-note">{note}</p>}
  </div>
}

function CollectionEntry({ item, ownershipVerified, showDucats, listedOrder, sellable, onSell, onUpdate, busy }: { item: CollectionItem; ownershipVerified: boolean; showDucats: boolean; listedOrder: MarketOrder | null; sellable: boolean; onSell: SellHandler; onUpdate: UpdateHandler; busy: boolean }) {
  const missing = item.quantity === 0
  const [artFailed, setArtFailed] = useState(false)
  const [selling, setSelling] = useState(false)
  // The rank belongs in the accessible name, not only in the marks: a mod held at two ranks is two
  // cards headed the same word, and without it they are indistinguishable to anyone not reading
  // the cartouches.
  const label = rankLabel(item) ? `${item.name}, ${rankLabel(item)}` : item.name
  // Nothing offered on a card whose whole holding is already listed: the badge above says so, and
  // the market allows one sell order per item, so a second listing is not what "sell more" can
  // mean. A listing that covers part of the holding keeps the control, as an edit of the listing
  // that already stands -- raising the count is the only honest way to sell the remainder.
  const remaining = sellable && (!listedOrder || listedOrder.quantity < item.quantity)
  return <article className={`entry cat-${item.category}`} aria-label={label}>
    <div className="entry-well">
      {item.image_url && !artFailed
        ? <img src={item.image_url} alt={item.name} loading="lazy" decoding="async" onError={() => setArtFailed(true)}/>
        : <span className="well-mark" aria-hidden="true">{categoryName[item.category].slice(0, 2).toUpperCase()}</span>}
    </div>
    <div className="entry-body">
      <span className="entry-cat">{categoryName[item.category]}</span>
      <h2 className="entry-name">{item.name}</h2>
      <div className="marks">
        {ownershipVerified
          ? missing
            ? <span className="hallmark absent">Missing</span>
            : <span className="hallmark owned">Owned ×{item.quantity}</span>
          : null}
        {rankLabel(item) && <span className={`hallmark rank${atMaxRank(item) ? ' maxed' : ''}`}>{rankLabel(item)}</span>}
        {ownershipVerified && item.mastered && <span className="hallmark mastered">Mastered</span>}
        {listedOrder && <span className="hallmark">{listedLabel(listedOrder, item.quantity)}</span>}
        {item.platinum !== undefined && <span className={`price${item.live ? ' live' : ''}`}>
          <MetalMark metal="plat" alt="platinum "/>
          {item.platinum_ceiling === undefined
            ? <>
              <b>{item.platinum}</b>
              {ownershipVerified && item.quantity > 1 && <em>{stackValue(item)} total</em>}
            </>
            // Nobody sells a half-ranked card, so the market brackets it without ever quoting it.
            // The two ends are what is known; a single number here would be invented.
            : <b title="Sellers list unranked and fully ranked copies only, so this rank sits between the two">
              {item.platinum}–{item.platinum_ceiling}
            </b>}
        </span>}
        {/* Baro's price, beside the market's. It is a fact of the item rather than of a holding,
            so it reads on a missing part too, where the platinum span above stays silent -- and it
            totals like platinum does, because a stack of parts banks a stack of ducats. */}
        {showDucats && (item.ducats !== undefined
          ? <span className="price ducat-reading">
            <MetalMark metal="ducat" alt="ducat "/>
            <b>{item.ducats}</b>
            {ownershipVerified && item.quantity > 1 && <em>{item.ducats * item.quantity} total</em>}
          </span>
          : item.category === 'prime_part' && <span className="ducat-unavailable">Ducat value unavailable</span>)}
      </div>
      {item.live && <p className="freshness">checked live</p>}
      {remaining && (selling
        ? <SellForm item={item} listing={listedOrder ?? undefined} busy={busy} onSell={onSell} onUpdate={onUpdate} onDone={() => setSelling(false)}/>
        : <button type="button" className="stamp sell-open" disabled={busy} onClick={() => setSelling(true)}><span>{listedOrder ? 'Sell more' : 'Sell'}</span></button>)}
    </div>
  </article>
}

function Pagination({ current, total, onChange }: { current: number; total: number; onChange: (page: number) => void }) {
  if (total <= 1) return null
  const pages = pageNumbers(current, total)
  return <nav className="pagination" aria-label="Collection pages">
    <button type="button" disabled={current === 1} aria-label="Previous page" onClick={() => onChange(current - 1)}>←</button>
    {pages.map((page, index) => <span key={page} className="page-slot">
      {index > 0 && page - pages[index - 1] > 1 ? <i aria-hidden="true">…</i> : null}
      <button type="button" className={page === current ? 'current' : ''} aria-current={page === current ? 'page' : undefined} aria-label={`Go to page ${page}`} onClick={() => onChange(page)}>{page}</button>
    </span>)}
    <button type="button" disabled={current === total} aria-label="Next page" onClick={() => onChange(current + 1)}>→</button>
  </nav>
}

function RewardPage({ view, effectiveMode }: { view: AppView; effectiveMode: AccessMode }) {
  const ownershipVerified = effectiveMode === 'full'
  const companion = effectiveMode === 'companion'
  return <div className="page">
    <div className="mark-head">
      <h1 id="reward-title" className="mark">Reward advisor</h1>
      <p className="prose">{companion
        ? 'Live reward observation is inactive in Companion. Historical reward cards remain available with saved ownership marked unverifiable.'
        : 'TennoScope watches EE.log for a Void Fissure reward, reads the four cards off the screen with OCR, and places advice below the reward row.'}</p>
    </div>
    <section aria-label="Reward advisor">
      {view.reward.cards.length
        ? <RewardCards cards={view.reward.cards} bestValueIndex={view.reward.best_value_index} bestDucatIndex={view.reward.best_ducat_index} ownershipVerified={ownershipVerified}/>
        : <EmptyState title="No reward choices detected" detail={companion ? 'Live reward observation is inactive in Companion; reference data remains available.' : 'The observer is waiting for an English Void Fissure reward screen.'}/>}
    </section>
  </div>
}

function AssayRow({ label, health }: { label: string; health: BackendHealth | { state: HealthState; message: string; last_success?: string | null } }) {
  return <article className={`assay-row ${health.state}`}>
    <span className="state-mark" aria-hidden="true"/>
    <div>
      <h3>{label}</h3>
      <p>{health.message}</p>
      {/* Rows record their success time in whatever form their own source keeps: most write Unix
          seconds, the price table an ISO date. Printed raw, one row reads
          "Last success: 1785492000". The relative reading is the useful one here; the exact stamp
          rides along for anyone comparing a row against a log. */}
      {health.last_success && <small>Last success: {healthSuccessLabel(health.last_success)}</small>}
    </div>
    <span className="assay-verdict">{health.state}</span>
  </article>
}

function healthSuccessLabel(value: string) {
  const reading = stampReading(value)
  return reading ? `${reading.relative} · ${reading.exact}` : value
}

type ReportStatus =
  | { kind: 'idle' }
  | { kind: 'busy' }
  | { kind: 'done'; message: string }

function ReportBlock({ health, alwaysVisible }: { health: AppView['health']; alwaysVisible?: boolean }) {
  const [status, setStatus] = useState<ReportStatus>({ kind: 'idle' })
  const broken = reportBlockVisible(health)
  if (!alwaysVisible && !broken) return null
  const run = async (action: () => Promise<void | { folder_path: string; ee_log_included: boolean }>, done: (result: { folder_path: string; ee_log_included: boolean } | null) => string) => {
    setStatus({ kind: 'busy' })
    try {
      const result = await action()
      const resultOrNull = result && typeof result === 'object' ? result : null
      setStatus({ kind: 'done', message: done(resultOrNull) })
    } catch (error) {
      setStatus({ kind: 'done', message: String(error) })
    }
  }
  return (
    <section className={`report-plate${broken ? ' broken' : ''}`} role="group" aria-label="Report a problem">
      <div className="report-head">
        <span className={`state-mark ${broken ? 'failed' : 'ready'}`} aria-hidden="true"/>
        <h2 className="report-title">Report a problem</h2>
        <p className="prose">{broken
          ? 'Strike a record of what failed. Review it before it leaves this machine — nothing is sent anywhere.'
          : 'Something not working right? Bundle your diagnostics and open an issue — nothing leaves this machine without you sending it.'}</p>
      </div>
      <div className="report-actions">
        <button type="button" className="stamp" disabled={status.kind === 'busy'} onClick={() => void run(openIssue, () => 'OPENED THE ISSUE FORM IN YOUR BROWSER.')}>Open an issue</button>
        <button type="button" className="stamp" disabled={status.kind === 'busy'} onClick={() => void run(copyReport, () => 'COPIED — PASTE IT INTO THE DIAGNOSTICS FIELD OF THE ISSUE FORM.')}>Copy diagnostics</button>
        <button type="button" className="stamp" disabled={status.kind === 'busy'} onClick={() => void run(saveReport, result =>
          `SAVED TO ${result?.folder_path ?? '…'}${result?.ee_log_included ? ' — EE.LOG INCLUDED (SANITIZED) — SAFE TO ATTACH TO THE ISSUE.' : ''}`,
        )}>Save logs</button>
      </div>
      {status.kind === 'done' && <p className="report-status" role="status">{status.message}</p>}
    </section>
  )
}

function DiagnosticsPage({ view, effectiveMode }: { view: AppView; effectiveMode: AccessMode }) {
  const referenceSystems = [
    ['Catalog', view.health.catalog],
    ['Market data', view.health.market],
    ['Collection prices', view.health.collection_prices],
    ['Database', view.health.database],
    ['Market account', view.health.market_account],
  ] as const
  const liveSystems = [
    ['Game reader', view.health.game_reader],
    ['EE.log', view.health.log_monitor],
    ['Reward observer', view.health.capture],
  ] as const
  const systems = effectiveMode === 'companion' ? referenceSystems : [...liveSystems, ...referenceSystems]
  const showAcquisition = effectiveMode === 'full'
  return <div className="page">
    <div className="mark-head">
      <h1 id="diagnostics-title" className="mark">Diagnostics</h1>
      <p className="prose">Status messages are deliberately scrubbed of temporary access values.</p>
    </div>
    <ReportBlock health={view.health}/>
    <section aria-label="Diagnostics">
      {effectiveMode === 'companion' && <p className="prose">Live Warframe diagnostics are inactive in Companion. Local and reference services remain visible.</p>}
      <div className="procedure-head">
        <h2 className="column-head">Core services</h2>
      </div>
      <div className="assay-list">{systems.map(([label, health]) => <AssayRow key={label} label={label} health={health}/>)}</div>

      {showAcquisition && <>
        <div className="procedure-head second">
          <h2 className="column-head">Acquisition pipeline</h2>
        </div>
        {view.health.acquisition_stages.length
          ? <ol className="stages">{view.health.acquisition_stages.map((stage, index) => {
            const words = stage.stage.replaceAll('_', ' ')
            const label = words[0].toUpperCase() + words.slice(1)
            return <li key={stage.stage} className={stage.state}>
              <span className={`ordinal ${stage.state}`}>{index + 1}</span>
              <div><strong>{label}</strong><p>{stage.message}</p></div>
              <span className="assay-verdict">{stage.state}</span>
            </li>
          })}</ol>
          : <EmptyState title="No acquisition attempt yet" detail="Start Warframe or request a refresh to populate the five pipeline stages."/>}
      </>}
    </section>
  </div>
}

function SettingsPage({ view, priceFloor, effectiveMode, selectedMode, modeBusy, modeError, onSelectMode, onConfirmMode, desktopCaptureActionAvailable, captureAuthorizationBusy, captureNote, onCaptureNote, onAuthorizeCapture, onPriceFloor }: { view: AppView; priceFloor: number; effectiveMode: AccessMode; selectedMode: AccessMode; modeBusy: boolean; modeError: string | null; onSelectMode: (mode: AccessMode) => void; onConfirmMode: () => void; desktopCaptureActionAvailable: boolean; captureAuthorizationBusy: boolean; captureNote: string | null; onCaptureNote: (note: string | null) => void; onAuthorizeCapture: () => Promise<void>; onPriceFloor: (floor: number) => void }) {
  const { sellable: total, sellableCount: counted } = collectionTotals(view.collection.items, priceFloor)
  return <section className="page" aria-labelledby="settings-title">
    <div className="mark-head">
      <h1 id="settings-title" className="mark">Settings</h1>
      <p className="prose">Preferences are held on this device, and take effect as they are set.</p>
    </div>
    <section className="mode-setting" aria-labelledby="access-setting-title">
      <div className="procedure-head access-setting-head">
        <div>
          <h2 id="access-setting-title" className="column-head">Warframe access</h2>
          <p className="band-note">Every change needs confirmation. Nothing applies until you confirm.</p>
        </div>
      </div>
      <AccessSelector
        value={selectedMode}
        effectiveMode={effectiveMode}
        onChange={onSelectMode}
        onConfirmUpgrade={onConfirmMode}
        applyingMode={modeBusy ? selectedMode : null}
        disabled={modeBusy}
      />
      {modeError && <p className="error-banner" role="alert">{modeError}</p>}
    </section>


    <section aria-label="Preferences">
      <div className="procedure-head">
        <h2 className="column-head">Preferences</h2>
      </div>
      <div className="setting">
        <div>
          <h3>Collection price floor</h3>
          <p className="prose">Stacks worth less than this per copy are left out of the sellable figure, and out of it alone — the market-rate total always counts everything. What the market completes is measured; whether a 3&nbsp;platinum mod is worth an evening of arranging the trade by hand is yours to say.</p>
        </div>
        <div className="dial">
          <label className="dial-slot">
            <span className="sr-only">Minimum platinum a copy must be worth to count</span>
            <input
              type="range"
              min={0}
              max={MAX_PRICE_FLOOR}
              step={1}
              value={priceFloor}
              style={{ '--dial-fill': `${priceFloor / MAX_PRICE_FLOOR * 100}%` } as CSSProperties}
              aria-valuetext={priceFloor ? `${priceFloor} platinum and over` : 'Every price counts'}
              onChange={event => onPriceFloor(Number(event.target.value))}
            />
          </label>
          <output className="dial-figure">
            {priceFloor ? <>{priceFloor}<MetalMark metal="plat" alt=" platinum"/><span> and over</span></> : <span>Every price</span>}
          </output>
        </div>
        <p className="band-note">{figure(counted)} stacks counted · {figure(total)} platinum sellable</p>
      </div>

      <DesktopCaptureSetting actionAvailable={effectiveMode !== 'companion' && desktopCaptureActionAvailable} prohibited={effectiveMode === 'companion'} busy={captureAuthorizationBusy} note={captureNote} onNote={onCaptureNote} onAuthorize={onAuthorizeCapture}/>

      <div className="setting">
        <div>
          <h3>Reward overlay placement</h3>
          <p className="prose">The strip is drawn against the game's own window, so where it lands is compositor-specific and there is no way to see it without a fissure running. This puts it on screen with nothing to read, and takes it down again.</p>
        </div>
        <OverlayPreviewToggle prohibited={effectiveMode === 'companion'} busy={modeBusy}/>
      </div>
    </section>

    <section aria-label="Support">
      <div className="procedure-head">
        <h2 className="column-head">Support</h2>
      </div>
      <ReportBlock health={view.health} alwaysVisible/>
    </section>
  </section>
}

function DesktopCaptureSetting({ actionAvailable, prohibited, busy, note, onNote, onAuthorize }: { actionAvailable: boolean; prohibited: boolean; busy: boolean; note: string | null; onNote: (note: string | null) => void; onAuthorize: () => Promise<void> }) {
  useEffect(() => {
    if (actionAvailable && note === 'Desktop capture allowed.') {
      onNote('Desktop capture needs permission again.')
    }
  }, [actionAvailable, note, onNote])
  async function authorize() {
    if (busy) return
    onNote(null)
    try {
      await onAuthorize()
      onNote('Desktop capture allowed.')
    } catch {
      onNote('Desktop capture was denied or is unavailable. Check that desktop screen sharing works, then try again.')
    }
  }
  return <div className="setting">
    <div>
      <h3>Screen capture</h3>
      <p className="prose">Screen capture is automatic. Desktop sharing is only needed when Warframe is launched with PROTON_ENABLE_WAYLAND=1 and this compositor has no direct capture API.</p>
      {prohibited ? <p className="prohibition-note" id="capture-prohibited">Overlay or Full access is required because Companion does not use screen capture.</p> : actionAvailable && <p className="prose">The desktop opens its own screen chooser. Select every display where Warframe may run. KDE/GNOME may show an active screen-sharing indicator. TennoScope releases the session when the game exits.</p>}
    </div>
    {(actionAvailable || prohibited) && <button type="button" className="stamp" onClick={authorize} disabled={busy || prohibited} aria-busy={busy} aria-describedby={prohibited ? 'capture-prohibited' : undefined}>{busy ? 'Allowing desktop capture…' : 'Allow desktop capture'}</button>}
    <p className="band-note capture-status" role="status" aria-live="polite" aria-atomic="true">{note}</p>
  </div>
}

/** What the office says about itself: what it is, and what it does to your machine to say it. */
function AboutPage({ effectiveMode }: { effectiveMode: AccessMode }) {
  const observesGame = effectiveMode !== 'companion'
  const acquiresInventory = effectiveMode === 'full'
  return <section className="page" aria-labelledby="about-title">
    <div className="mark-head">
      <h1 id="about-title" className="mark">About</h1>
      <p className="prose">TennoScope is a free, open-source, local-first companion. GPLv3 · MVP.</p>
    </div>

    <div className="clauses">
      <article className="clause">
        <span className="clause-index" aria-hidden="true">I</span>
        <div>
          <h3>Local-first storage</h3>
          <p className="prose">Your inventory snapshot and preferences are stored on this device in the application data directory. The UI has no telemetry or cloud account.</p>
        </div>
      </article>
      <article className={observesGame ? 'clause caution' : 'clause'}>
        <span className="clause-index" aria-hidden="true">{observesGame ? 'Caution' : 'II'}</span>
        <div>
          <h3>{observesGame ? 'Warframe access disclosure' : 'Companion access'}</h3>
          <p className="prose">{effectiveMode === 'full'
            ? 'Full access inspects the running game process using read-only memory access. Third-party software and process inspection may carry account-policy or anti-cheat risk even when no game memory is modified.'
            : effectiveMode === 'overlay'
              ? 'Overlay observes process presence, EE.log, and visible pixels to provide live reward assistance. It does not read process memory or acquire inventory.'
              : 'Companion does not observe the running game process or EE.log, capture the screen, display overlays, read process memory, or acquire inventory.'}</p>
        </div>
      </article>
      <article className="clause">
        <span className="clause-index" aria-hidden="true">III</span>
        <div>
          <h3>Inventory synchronization</h3>
          <p className="prose">{acquiresInventory ? 'Full access can synchronize inventory automatically and through the manual refresh in the masthead.' : 'Inventory synchronization is available only in Full access. Your existing saved snapshot remains available here.'}</p>
        </div>
      </article>
      <article className="clause">
        <span className="clause-index" aria-hidden="true">IV</span>
        <div>
          <h3>Reward assistance</h3>
          <p className="prose">{observesGame ? 'Reward names can be read from visible screen pixels with OCR and matched against the squad’s relic pool. The overlay is non-focusable and click-through, so it never takes input from the game.' : 'The catalog and relic reference remain available for planning. Live screen recognition and the click-through overlay are available in Overlay and Full access.'}</p>
        </div>
      </article>
    </div>
  </section>
}

/**
 * The strip is placed against the game's own window, so a preview is the only way
 * to see it without a fissure running -- but a preview you cannot dismiss does not
 * earn its place, which is why this is a toggle and not a one-way button.
 */
function OverlayPreviewToggle({ prohibited, busy }: { prohibited: boolean; busy: boolean }) {
  const [shown, setShown] = useState(false)
  const [blocked, setBlocked] = useState(false)
  return <div className="prohibited-control">
    <button
      type="button"
      className="stamp"
      aria-pressed={shown}
      disabled={prohibited || busy}
      aria-describedby={prohibited ? 'overlay-prohibited' : blocked ? 'overlay-blocked' : undefined}
      onClick={() => {
        // The backend rejects while an access transition holds the gate. Flip only after the
        // command lands, so a click across a downgrade reports instead of desyncing the toggle.
        const next = !shown
        setBlocked(false)
        void (shown ? hideRewardOverlay() : showRewardOverlay())
          .then(() => setShown(next))
          .catch(() => setBlocked(true))
      }}
    ><span>{shown ? 'Hide reward overlay' : 'Preview reward overlay'}</span></button>
    {prohibited && <p id="overlay-prohibited" className="prohibition-note">Overlay or Full access is required because Companion does not create overlays.</p>}
    {!prohibited && blocked && <p id="overlay-blocked" className="band-note" role="status">The overlay is unavailable while access is changing. Try again once the new mode is effective.</p>}
  </div>
}

function EmptyState({ title, detail }: { title: string; detail: string }) {
  return <div className="empty-state">
    <span className="void-mark" aria-hidden="true"/>
    <h2>{title}</h2>
    <p>{detail}</p>
  </div>
}

export default App
