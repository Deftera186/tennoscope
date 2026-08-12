import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from 'react'
import './App.css'
import {
  acceptRiskDisclosure,
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
  setMarketPresence,
  setOrderQuantity,
  type AppView,
  type BackendHealth,
  type CollectionItem,
  type HealthState,
  type ItemCategory,
  type Presence,
} from './backend'
import { hideRewardOverlay, showRewardOverlay } from './overlay'
import { copyReport, openIssue, saveReport } from './report'
import { RewardCards } from './RewardCards'
import { MetalMark } from './MetalMark'
import { OrdersView } from './OrdersView'
import { isListable, listedOrderFor } from './orders'
import { SellForm, type SellHandler } from './SellForm'
import { atMaxRank, clampPage, COLLECTION_PAGE_SIZE, pageCount, pageItems, pageNumbers, rankLabel, sellableValue, stackValue } from './collection'
import { MAX_PRICE_FLOOR, readPriceFloor, writePriceFloor } from './settings'
import { snapshotFreshness } from './freshness'

type Page = 'collection' | 'rewards' | 'orders' | 'diagnostics' | 'settings' | 'about'
type Ownership = 'all' | 'owned' | 'mastered' | 'missing' | 'tradeable'
type Sort = 'name-asc' | 'quantity-desc' | 'category-asc' | 'value-desc'

const categories: Array<{ value: ItemCategory | 'all'; label: string; tally: string }> = [
  { value: 'all', label: 'All categories', tally: '✳' },
  { value: 'frame', label: 'Frame', tally: 'F' },
  { value: 'weapon', label: 'Weapon', tally: 'W' },
  { value: 'companion', label: 'Companion', tally: 'C' },
  { value: 'prime_part', label: 'Prime Part', tally: 'P' },
  { value: 'relic', label: 'Relic', tally: 'R' },
  { value: 'resource', label: 'Resource', tally: 'S' },
  { value: 'blueprint', label: 'Blueprint', tally: 'B' },
  { value: 'vehicle', label: 'Vehicle', tally: 'V' },
  { value: 'mod', label: 'Mod', tally: 'M' },
  { value: 'arcane', label: 'Arcane', tally: 'A' },
]

const sortOptions: Array<{ value: Sort; label: string }> = [
  { value: 'name-asc', label: 'Name A–Z' },
  { value: 'quantity-desc', label: 'Quantity' },
  { value: 'category-asc', label: 'Category' },
  { value: 'value-desc', label: 'Value' },
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

function App() {
  const [accepted, setAccepted] = useState<boolean | null>(null)
  const [view, setView] = useState<AppView | null>(null)
  const [page, setPage] = useState<Page>('collection')
  const [busy, setBusy] = useState(false)
  const [pricing, setPricing] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [clock, setClock] = useState(() => new Date())
  const [priceFloor, setPriceFloor] = useState(readPriceFloor)
  const [ordersBusy, setOrdersBusy] = useState(false)
  const [ordersError, setOrdersError] = useState<string | null>(null)
  const viewGeneration = useRef(0)
  const foregroundInFlight = useRef(0)

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
        setAccepted(status.risk_accepted)
        if (status.risk_accepted) {
          await requestView(getView, 'The local application backend is unavailable.')
          // Populates the collection badges and nav count as soon as the app is usable, rather
          // than leaving them blank until the player opens Orders. On an unlinked install this
          // command makes no network request, so it costs nothing.
          await requestMarketStatus()
        }
      })
      .catch(() => setError('The local application backend is unavailable.'))
  }, [requestView, requestMarketStatus])

  useEffect(() => {
    if (!accepted) return
    let active = true
    let timer: ReturnType<typeof setTimeout> | undefined
    const schedule = () => { if (active) timer = setTimeout(poll, 2500) }
    const poll = async () => {
      if (document.hidden || foregroundInFlight.current > 0) { schedule(); return }
      await requestView(getView, 'The live backend view could not be updated.')
      schedule()
    }
    schedule()
    return () => {
      active = false
      viewGeneration.current += 1
      if (timer) clearTimeout(timer)
    }
  }, [accepted, requestView])

  useEffect(() => {
    const timer = setInterval(() => setClock(new Date()), 30_000)
    return () => clearInterval(timer)
  }, [])

  async function accept() {
    setBusy(true)
    setError(null)
    try {
      await runForeground(async () => {
        await acceptRiskDisclosure()
        setAccepted(true)
        await requestView(getView, 'The local application backend is unavailable.')
      })
    } catch {
      setError('Setup could not be saved.')
    } finally {
      setBusy(false)
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
   * it. Ordering is still safe -- `requestView` applies a response only while its request is the
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

  async function ordersOperation(operation: () => Promise<AppView>, failure: string) {
    setOrdersBusy(true)
    setOrdersError(null)
    try {
      const next = await operation()
      setView(next)
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
  const ordersSell = (catalogPath: string, platinum: number, quantity: number, visible: boolean) =>
    ordersOperation(() => createOrder(catalogPath, platinum, quantity, visible), 'Could not publish that listing.')

  function openPage(next: Page) {
    setPage(next)
    if (next === 'orders') void ordersRefresh()
  }

  if (accepted === null && !error) return <main className="holding"><div className="streak" aria-hidden="true"/><p className="register-line">Starting TennoScope…</p></main>
  if (!accepted) return <SetupScreen busy={busy} error={error} onAccept={accept}/>

  const liveState = view?.health.game_reader.state ?? 'degraded'
  const freshness = snapshotFreshness(view?.collection.snapshot, clock)
  return <div className="assay">
    <header className="masthead">
      <div className="masthead-top">
        <div className="office">
          <span className="office-name">TennoScope</span>
          <span className="office-role">Local assay register</span>
        </div>
        <div className="masthead-state">
          <div className={`assay-state ${liveState}`}>
            <span className="state-mark" aria-hidden="true"/>
            <span className="assay-state-text">
              <strong role="status">{liveState === 'ready' ? 'Watching Warframe' : liveState === 'idle' ? 'Idle' : liveState === 'failed' ? 'Attention — reader failed' : 'Attention needed'}</strong>
              <small>{view?.health.game_reader.message ?? 'Connecting to local backend'}</small>
            </span>
          </div>
          {view && <span className="date-letter" title={freshness.detail}>{freshness.label}<span className="sr-only"> — {freshness.detail}</span></span>}
          <button type="button" className="stamp" onClick={refresh} disabled={busy}>
            <Mark name="refresh" className="punch-glyph"/><span>{busy ? 'Refreshing…' : 'Refresh inventory'}</span>
          </button>
        </div>
      </div>
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
    </header>

    <main className="sheet">
      {error && <p className="error-banner" role="alert">{error}</p>}
      {/* A sell can be started from a collection card, and its failure has to be readable where it
          was started. The orders screen renders this itself, in the block that owns the recovery. */}
      {ordersError && page !== 'orders' && <p className="error-banner" role="alert">{ordersError}</p>}
      {!view ? <LoadingView/> : <>
        {page === 'collection' && <CollectionPage view={view} pricing={pricing} onPriceLive={priceLive} priceFloor={priceFloor} onSell={ordersSell} ordersBusy={ordersBusy}/>}
        {page === 'rewards' && <RewardPage view={view}/>}
        {page === 'orders' && <OrdersView
          account={view.market_account}
          onSignIn={ordersSignIn}
          onLinkToken={ordersLinkToken}
          onSignOut={ordersSignOut}
          onRefresh={ordersRefresh}
          onRemove={ordersRemove}
          onLowerTo={ordersLowerTo}
          onSell={ordersSell}
          onPresence={ordersPresence}
          items={view.collection.items}
          busy={ordersBusy}
          error={ordersError}
        />}
        {page === 'diagnostics' && <DiagnosticsPage view={view}/>}
        {page === 'settings' && <SettingsPage view={view} priceFloor={priceFloor} onPriceFloor={floor => {
          setPriceFloor(floor)
          writePriceFloor(floor)
        }}/>}
        {page === 'about' && <AboutPage/>}
      </>}
    </main>
  </div>
}

/** The certificate of assay: the one-time disclosure, read before anything is inspected. */
function SetupScreen({ busy, error, onAccept }: { busy: boolean; error: string | null; onAccept: () => void }) {
  return <main className="certificate">
    <section className="certificate-sheet" aria-labelledby="setup-title">
      <div className="office">
        <span className="office-name">TennoScope</span>
        <span className="office-role">One-time setup · Read carefully</span>
      </div>
      <h1 id="setup-title" className="mark">Read-only game access</h1>
      <p className="prose">Automatic inventory sync needs permission to inspect the running Warframe process and make a direct inventory request.</p>
      <div className="clause-pair">
        <article>
          <span className="verdict-mark" aria-hidden="true"/>
          <h2>Private by design</h2>
          <p>The app never logs or uploads credentials or raw player payloads. Collection data stays on this device.</p>
        </article>
        <article className="caution">
          <span className="verdict-mark caution" aria-hidden="true"/>
          <h2>Know the risk</h2>
          <p>Third-party software and process inspection may carry account-policy or anti-cheat risk, even when access is read-only.</p>
        </article>
      </div>
      <p className="footnote">After acceptance, automatic read-only acquisition is enabled by default. You can revisit this disclosure in About.</p>
      {error && <p className="error-banner" role="alert">{error}</p>}
      <button type="button" className="seal" onClick={onAccept} disabled={busy}>
        {busy ? 'Saving locally…' : 'Accept risk and continue'}<span aria-hidden="true">→</span>
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

function CollectionPage({ view, pricing, onPriceLive, priceFloor, onSell, ordersBusy }: { view: AppView; pricing: boolean; onPriceLive: (ids: string[]) => void; priceFloor: number; onSell: SellHandler; ordersBusy: boolean }) {
  const [search, setSearch] = useState('')
  const [category, setCategory] = useState<ItemCategory | 'all'>('all')
  const [ownership, setOwnership] = useState<Ownership>('all')
  const [sort, setSort] = useState<Sort>('name-asc')
  const [page, setPage] = useState(1)
  const masteryEligible = view.collection.items.filter(item => ['frame', 'weapon', 'companion', 'vehicle'].includes(item.category))
  const mastered = masteryEligible.filter(item => item.mastered).length
  const owned = view.collection.items.filter(item => item.quantity > 0).length
  const missing = view.collection.items.filter(item => item.quantity === 0).length
  const priced = view.collection.items.filter(item => item.platinum !== undefined)
  const worth = priced.reduce((total, item) => total + (stackValue(item) ?? 0), 0)
  // The second, smaller figure. A unit price is only half of what a stack is worth: this collection's
  // largest single holding is 182 Quickdraw at a true 2p, and the whole game trades two Quickdraw a
  // month. Market rate leads because it is the plain reading of what is owned; what the market would
  // actually take sits under it, at the size of a qualification, which is what it is.
  const sellable = priced.reduce((total, item) => total + (sellableValue(item, priceFloor) ?? 0), 0)
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
          : sort === 'value-desc'
            ? (right.platinum ?? -1) - (left.platinum ?? -1) || left.name.localeCompare(right.name)
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
  useEffect(() => setPage(1), [search, category, ownership, sort])
  useEffect(() => setPage(value => clampPage(value, filtered.length)), [filtered.length])

  return <section className="page" aria-labelledby="collection-title">
    <div className="mark-head">
      <h1 id="collection-title" className="mark">Your collection</h1>
      <p className="prose">Canonical equipment, parts and relics observed on this account. Read only, held locally.</p>
    </div>

    <div className="assay-band">
      <BandCell kind="items" value={view.collection.total_entries} label="Items tracked" note={`${owned} currently owned`}/>
      <BandCell kind="mastered" value={mastered} label="Mastered" note={masteryEligible.length ? `${Math.round(mastered / masteryEligible.length * 100)}% of mastery-eligible items` : 'No mastery-eligible items'}/>
      <BandCell kind="missing" value={missing} label="Missing" note="From known collection data"/>
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
        label="Collection worth"
        note={priceFloor
          ? `Sellable counts only the copies the market buys in a month, at ${priceFloor} platinum and over`
          : 'Sellable counts only the copies the market buys in a month'}
      />
    </div>

    <div className="register">
      <div className="register-controls">
        <label className="search-slot">
          <Mark name="search" className="punch-glyph"/>
          <span className="sr-only">Search collection</span>
          <input type="search" aria-label="Search collection" placeholder="Search canonical item names…" value={search} onChange={event => setSearch(event.target.value)}/>
        </label>
        <div className="sort-slot" role="group" aria-label="Sort collection">
          <span>Sort</span>
          <div className="tally">
            {sortOptions.map(option => <button
              type="button"
              key={option.value}
              aria-pressed={sort === option.value}
              onClick={() => setSort(option.value)}
            >{option.label}</button>)}
          </div>
        </div>
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
        <div className="tally" role="group" aria-label="Ownership filters">
          {(['all', 'owned', 'mastered', 'missing', 'tradeable'] as const).map(filter => <button
            type="button"
            key={filter}
            aria-pressed={ownership === filter}
            onClick={() => setOwnership(filter)}
          >{filter[0].toUpperCase() + filter.slice(1)}</button>)}
        </div>
        <div className="provenance-row">
          <div className="register-status">
            <span className="register-line">{dumpDate ? `Prices from the ${shortDumpDate(dumpDate)} market summary` : 'No price summary loaded yet'}</span>
            <span className="register-line">{firstResult}–{lastResult} of {filtered.length}</span>
            {inProgress && <span className="register-line pricing" role="status">Checking live prices · {inProgress.done} of {inProgress.total}</span>}
          </div>
          {/* No count on the control while a pass runs: the register line beside it carries the one
              readout, and a second copy of the same numbers on the thing that is disabled reads as
              a different pass. The label only has to say the control is spoken for. */}
          <button
            type="button"
            className="stamp"
            disabled={pricing || inProgress !== null || pricableVisibleIds.length === 0}
            onClick={() => onPriceLive(pricableVisibleIds)}
          ><span>{pricing || inProgress ? 'Pricing…' : `Price these ${pricableVisibleIds.length}`}</span></button>
        </div>
      </div>

      {filtered.length
        ? <>
          <ul className="collection-grid" aria-label="Collection items">{visibleItems.map(item => <li key={item.id}><CollectionEntry item={item} listedOrder={listedOrderFor(view.market_account.orders, item.id)} sellable={view.market_account.link === 'linked' && isListable(item, view.market_account.listable)} onSell={onSell} busy={ordersBusy}/></li>)}</ul>
          <Pagination current={currentPage} total={totalPages} onChange={setPage}/>
        </>
        : <EmptyState
          title={view.collection.items.length ? 'No matching items' : 'No inventory items yet'}
          detail={view.collection.items.length ? 'Try another search or clear a filter.' : 'Start Warframe and refresh to create your first local snapshot.'}
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

function CollectionEntry({ item, listedOrder, sellable, onSell, busy }: { item: CollectionItem; listedOrder: ReturnType<typeof listedOrderFor>; sellable: boolean; onSell: SellHandler; busy: boolean }) {
  const missing = item.quantity === 0
  const [artFailed, setArtFailed] = useState(false)
  const [selling, setSelling] = useState(false)
  // The rank belongs in the accessible name, not only in the marks: a mod held at two ranks is two
  // cards headed the same word, and without it they are indistinguishable to anyone not reading
  // the cartouches.
  const label = rankLabel(item) ? `${item.name}, ${rankLabel(item)}` : item.name
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
        {missing
          ? <span className="hallmark absent">Missing</span>
          : <span className="hallmark owned">Owned ×{item.quantity}</span>}
        {rankLabel(item) && <span className={`hallmark rank${atMaxRank(item) ? ' maxed' : ''}`}>{rankLabel(item)}</span>}
        {item.mastered && <span className="hallmark mastered">Mastered</span>}
        {listedOrder && <span className="hallmark">listed {listedOrder.platinum}p</span>}
        {item.platinum !== undefined && <span className={`price${item.live ? ' live' : ''}`}>
          <MetalMark metal="plat" alt="platinum "/>
          {item.platinum_ceiling === undefined
            ? <>
              <b>{item.platinum}</b>
              {item.quantity > 1 && <em>{stackValue(item)} total</em>}
            </>
            // Nobody sells a half-ranked card, so the market brackets it without ever quoting it.
            // The two ends are what is known; a single number here would be invented.
            : <b title="Sellers list unranked and fully ranked copies only, so this rank sits between the two">
              {item.platinum}–{item.platinum_ceiling}
            </b>}
        </span>}
      </div>
      {item.live && <p className="freshness">checked live</p>}
      {/* Nothing offered on a card that is already listed: the badge above says so, and a second
          listing for the same item is not what "sell" meant. */}
      {sellable && !listedOrder && (selling
        ? <SellForm item={item} busy={busy} onSell={onSell} onDone={() => setSelling(false)}/>
        : <button type="button" className="stamp sell-open" disabled={busy} onClick={() => setSelling(true)}><span>Sell</span></button>)}
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

function RewardPage({ view }: { view: AppView }) {
  return <div className="page">
    <div className="mark-head">
      <h1 id="reward-title" className="mark">Reward advisor</h1>
      <p className="prose">TennoScope watches EE.log for a Void Fissure reward, reads the four cards off the screen with OCR, and places advice below the reward row.</p>
    </div>
    <section aria-label="Reward advisor">
      {view.reward.cards.length
        ? <RewardCards cards={view.reward.cards} bestValueIndex={view.reward.best_value_index} bestDucatIndex={view.reward.best_ducat_index}/>
        : <EmptyState title="No reward choices detected" detail="The observer is waiting for an English Void Fissure reward screen."/>}
    </section>
  </div>
}

function AssayRow({ label, health }: { label: string; health: BackendHealth | { state: HealthState; message: string; last_success?: string | null } }) {
  return <article className={`assay-row ${health.state}`}>
    <span className="state-mark" aria-hidden="true"/>
    <div>
      <h3>{label}</h3>
      <p>{health.message}</p>
      {/* Rows record their success time in whatever form their own source keeps: the market
          account writes Unix seconds, the price table an ISO date. Printed raw, one row reads
          "Last success: 1785492000". `snapshotFreshness` already resolves both. */}
      {health.last_success && <small>Last success: {healthSuccessLabel(health.last_success)}</small>}
    </div>
    <span className="assay-verdict">{health.state}</span>
  </article>
}

function healthSuccessLabel(value: string) {
  const { label, detail } = snapshotFreshness({ observed_at: value, game_build: '', source: '' })
  // The relative reading is the useful one on a health row; the exact stamp leads the detail, and
  // is worth keeping for anyone comparing rows against a log.
  return label === 'Sync time unavailable' ? value : `${label.replace(/^Synced /, '')} · ${detail.split(' · ')[0]}`
}

type ReportStatus =
  | { kind: 'idle' }
  | { kind: 'busy' }
  | { kind: 'done'; message: string }

function stageBroken(value: unknown): value is { state: HealthState } {
  return typeof value === 'object' && value !== null && 'state' in value
}

function ReportBlock({ health }: { health: AppView['health'] }) {
  const [status, setStatus] = useState<ReportStatus>({ kind: 'idle' })
  const broken = Object.values(health).some(value =>
    Array.isArray(value)
      ? value.some(stage => stage.state === 'degraded' || stage.state === 'failed')
      : stageBroken(value) && (value.state === 'degraded' || value.state === 'failed'),
  )
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
    <section className="report-plate" role="group" aria-label="Report a problem">
      <div className="report-head">
        <span className={`state-mark ${broken ? 'failed' : 'ready'}`} aria-hidden="true"/>
        <h2 className="report-title">Report a problem</h2>
        <p className="prose">Strike a record of what failed. Review it before it leaves this machine — nothing is sent anywhere.</p>
      </div>
      <div className="report-actions">
        <button type="button" className="stamp" disabled={status.kind === 'busy'} onClick={() => void run(openIssue, () => 'OPENED THE ISSUE FORM IN YOUR BROWSER.')}>Open an issue</button>
        <button type="button" className="stamp" disabled={status.kind === 'busy'} onClick={() => void run(copyReport, () => 'COPIED — PASTE IT INTO THE ISSUE FORM.')}>Copy report</button>
        <button type="button" className="stamp" disabled={status.kind === 'busy'} onClick={() => void run(saveReport, result =>
          `SAVED TO ${result?.folder_path ?? '…'}${result?.ee_log_included ? ' — EE.LOG INCLUDED (SENSITIVE) — SEND IT TO THE MAINTAINER ON DISCORD, NOT TO THE ISSUE.' : ''}`,
        )}>Save logs</button>
      </div>
      {status.kind === 'done' && <p className="report-status" role="status">{status.message}</p>}
    </section>
  )
}

function DiagnosticsPage({ view }: { view: AppView }) {
  const systems = [
    ['Game reader', view.health.game_reader],
    ['EE.log', view.health.log_monitor],
    ['Reward observer', view.health.capture],
    ['Catalog', view.health.catalog],
    ['Market data', view.health.market],
    ['Collection prices', view.health.collection_prices],
    ['Database', view.health.database],
    ['Market account', view.health.market_account],
  ] as const
  return <div className="page">
    <div className="mark-head">
      <h1 id="diagnostics-title" className="mark">Diagnostics</h1>
      <p className="prose">Status messages are deliberately scrubbed of temporary access values.</p>
    </div>
    <ReportBlock health={view.health}/>
    <section aria-label="Diagnostics">
      <div className="procedure-head">
        <h2 className="column-head">Core services</h2>
      </div>
      <div className="assay-list">{systems.map(([label, health]) => <AssayRow key={label} label={label} health={health}/>)}</div>

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
    </section>
  </div>
}

function SettingsPage({ view, priceFloor, onPriceFloor }: { view: AppView; priceFloor: number; onPriceFloor: (floor: number) => void }) {
  // The slider's own readout. A floor that only moves a figure on another page is a knob with no
  // dial: this says, at the moment it is dragged, exactly which holding it just wrote off.
  const counted = view.collection.items.filter(item => (sellableValue(item, priceFloor) ?? 0) > 0)
  const total = counted.reduce((sum, item) => sum + (sellableValue(item, priceFloor) ?? 0), 0)
  return <section className="page" aria-labelledby="settings-title">
    <div className="mark-head">
      <h1 id="settings-title" className="mark">Settings</h1>
      <p className="prose">Preferences are held on this device, and take effect as they are set.</p>
    </div>

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
        <p className="band-note">{figure(counted.length)} stacks counted · {figure(total)} platinum sellable</p>
      </div>

      <div className="setting">
        <div>
          <h3>Reward overlay placement</h3>
          <p className="prose">The strip is drawn against the game's own window, so where it lands is compositor-specific and there is no way to see it without a fissure running. This puts it on screen with nothing to read, and takes it down again.</p>
        </div>
        <OverlayPreviewToggle/>
      </div>
    </section>
  </section>
}

/** What the office says about itself: what it is, and what it does to your machine to say it. */
function AboutPage() {
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
      <article className="clause caution">
        <span className="clause-index" aria-hidden="true">Caution</span>
        <div>
          <h3>Read-only access disclosure</h3>
          <p className="prose">TennoScope inspects the running game process. Third-party software and process inspection may carry account-policy or anti-cheat risk even when no game memory is modified.</p>
        </div>
      </article>
      <article className="clause">
        <span className="clause-index" aria-hidden="true">II</span>
        <div>
          <h3>Automatic synchronization</h3>
          <p className="prose">The local EE.log monitor watches for inventory synchronization and refreshes automatically. Manual refresh remains available in the masthead.</p>
        </div>
      </article>
      <article className="clause">
        <span className="clause-index" aria-hidden="true">III</span>
        <div>
          <h3>Reward overlay</h3>
          <p className="prose">Reward names are read from the screen with OCR and matched against the squad's own relic pool. The strip is non-focusable and click-through, so it never takes input from the game. Settings can preview where it lands.</p>
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
function OverlayPreviewToggle() {
  const [shown, setShown] = useState(false)
  return <button
    type="button"
    className="stamp"
    aria-pressed={shown}
    onClick={() => {
      void (shown ? hideRewardOverlay() : showRewardOverlay())
      setShown(!shown)
    }}
  ><span>{shown ? 'Hide reward overlay' : 'Preview reward overlay'}</span></button>
}

function EmptyState({ title, detail }: { title: string; detail: string }) {
  return <div className="empty-state">
    <span className="void-mark" aria-hidden="true"/>
    <h2>{title}</h2>
    <p>{detail}</p>
  </div>
}

export default App
