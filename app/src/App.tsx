import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import './App.css'
import {
  acceptRiskDisclosure,
  getSetupStatus,
  getView,
  refreshInventory,
  refreshPrices,
  type AppView,
  type BackendHealth,
  type CollectionItem,
  type HealthState,
  type ItemCategory,
} from './backend'
import { hideRewardOverlay, showRewardOverlay } from './overlay'
import { RewardCards } from './RewardCards'
import { clampPage, COLLECTION_PAGE_SIZE, pageCount, pageItems, pageNumbers, stackValue } from './collection'
import { snapshotFreshness } from './freshness'

type Page = 'collection' | 'rewards' | 'diagnostics' | 'settings'
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
]

const sortOptions: Array<{ value: Sort; label: string }> = [
  { value: 'name-asc', label: 'Name A–Z' },
  { value: 'quantity-desc', label: 'Quantity' },
  { value: 'category-asc', label: 'Category' },
  { value: 'value-desc', label: 'Value' },
]

const categoryName = Object.fromEntries(categories.map(category => [category.value, category.label])) as Record<ItemCategory | 'all', string>

const pageLabel: Record<Page, string> = {
  collection: 'Collection',
  rewards: 'Rewards',
  diagnostics: 'Diagnostics',
  settings: 'Settings',
}

/**
 * Assay marks, drawn in the world's own grammar: hard geometry, square caps,
 * no rounded joins. The rewards glyph is the orb of the platinum standard mark.
 */
function Mark({ name, className = 'punch-glyph' }: { name: Page | 'refresh' | 'search'; className?: string }) {
  const paths = {
    collection: <><path d="M3 4h18M3 10h13M3 16h18M3 22h9"/></>,
    rewards: <><circle cx="12" cy="14.5" r="7.5"/><path d="M12 7V1.5M9 4h6"/></>,
    diagnostics: <><path d="M2 21h20M6 21 14 3M11 21 19 3"/></>,
    settings: <><path d="M7 2h10l-2 9H9z"/><path d="M10 11h4v11h-4z"/></>,
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

  useEffect(() => {
    getSetupStatus()
      .then(async status => {
        setAccepted(status.risk_accepted)
        if (status.risk_accepted) await requestView(getView, 'The local application backend is unavailable.')
      })
      .catch(() => setError('The local application backend is unavailable.'))
  }, [requestView])

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
   */
  async function priceLive(ids: string[]) {
    setPricing(true)
    await requestView(() => refreshPrices(ids), 'Live prices could not be fetched.')
    setPricing(false)
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
        {(['collection', 'rewards', 'diagnostics', 'settings'] as const).map(item => <button
          key={item}
          type="button"
          aria-label={pageLabel[item]}
          className={page === item ? 'punch struck' : 'punch'}
          aria-current={page === item ? 'page' : undefined}
          onClick={() => setPage(item)}
        >
          <span className="punch-face">
            <Mark name={item}/>
            <span className="punch-name">{pageLabel[item]}</span>
            {item === 'rewards' && view?.reward.cards.length ? <em className="punch-count">{view.reward.cards.length}</em> : null}
          </span>
        </button>)}
      </nav>
    </header>

    <main className="sheet">
      {error && <p className="error-banner" role="alert">{error}</p>}
      {!view ? <LoadingView/> : <>
        {page === 'collection' && <CollectionPage view={view} pricing={pricing} onPriceLive={priceLive}/>}
        {page === 'rewards' && <RewardPage view={view}/>}
        {page === 'diagnostics' && <DiagnosticsPage view={view}/>}
        {page === 'settings' && <SettingsPage/>}
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
      <p className="footnote">After acceptance, automatic read-only acquisition is enabled by default. You can revisit this disclosure in Settings.</p>
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

function CollectionPage({ view, pricing, onPriceLive }: { view: AppView; pricing: boolean; onPriceLive: (ids: string[]) => void }) {
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
            ? (stackValue(right) ?? -1) - (stackValue(left) ?? -1) || left.name.localeCompare(right.name)
            : left.name.localeCompare(right.name))
  }, [view.collection.items, search, category, ownership, sort])
  const totalPages = pageCount(filtered.length)
  const currentPage = clampPage(page, filtered.length)
  const visibleItems = pageItems(filtered, currentPage)
  const firstResult = filtered.length ? (currentPage - 1) * COLLECTION_PAGE_SIZE + 1 : 0
  const lastResult = Math.min(currentPage * COLLECTION_PAGE_SIZE, filtered.length)
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
      <BandCell kind="worth" value={worth} unit="p" label="Collection worth" note={`${priced.length} of ${view.collection.items.length} items priced`}/>
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
        <div className="refresh-slot">
          <button
            type="button"
            className="stamp"
            disabled={pricing}
            aria-label="Refresh prices on this page"
            onClick={() => onPriceLive(visibleItems.map(item => item.id))}
          ><span>{pricing ? 'Pricing…' : 'Refresh prices'}</span></button>
          {pricing && <span className="streak" aria-hidden="true"/>}
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

      <div className="register-bar">
        <div className="tally" role="group" aria-label="Ownership filters">
          {(['all', 'owned', 'mastered', 'missing', 'tradeable'] as const).map(filter => <button
            type="button"
            key={filter}
            aria-pressed={ownership === filter}
            onClick={() => setOwnership(filter)}
          >{filter[0].toUpperCase() + filter.slice(1)}</button>)}
        </div>
        <span>{firstResult}–{lastResult} of {filtered.length}</span>
      </div>

      {filtered.length
        ? <>
          <ul className="collection-grid" aria-label="Collection items">{visibleItems.map(item => <li key={item.id}><CollectionEntry item={item}/></li>)}</ul>
          <Pagination current={currentPage} total={totalPages} onChange={setPage}/>
        </>
        : <EmptyState
          title={view.collection.items.length ? 'No matching items' : 'No inventory items yet'}
          detail={view.collection.items.length ? 'Try another search or clear a filter.' : 'Start Warframe and refresh to create your first local snapshot.'}
        />}
    </div>
  </section>
}

// The worth cell sits in a row of plain counts, where a bare number reads as one more count.
function BandCell({ kind, value, unit = '', label, note }: { kind: string; value: number; unit?: string; label: string; note: string }) {
  return <div className={`band-cell ${kind}`} data-summary={kind} data-testid={`band-${kind}`}>
    <span className="band-figure">{value}{unit}</span>
    <span className="band-label">{label}</span>
    <p className="band-note">{note}</p>
  </div>
}

function CollectionEntry({ item }: { item: CollectionItem }) {
  const missing = item.quantity === 0
  const [artFailed, setArtFailed] = useState(false)
  return <article className={`entry cat-${item.category}`} aria-label={item.name}>
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
        {item.mastered && <span className="hallmark mastered">Mastered</span>}
        {item.platinum !== undefined && <span className={`price${item.live ? ' live' : ''}`}>
          <b>{item.platinum}p</b>
          {item.quantity > 1 && <em>{stackValue(item)}p total</em>}
          {item.live && <span className="hallmark live">Live</span>}
        </span>}
      </div>
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
      {health.last_success && <small>Last success: {health.last_success}</small>}
    </div>
    <span className="assay-verdict">{health.state}</span>
  </article>
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
  ] as const
  return <div className="page">
    <div className="mark-head">
      <h1 id="diagnostics-title" className="mark">Diagnostics</h1>
      <p className="prose">Status messages are deliberately scrubbed of temporary access values.</p>
    </div>
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

function SettingsPage() {
  return <section className="page" aria-labelledby="settings-title">
    <div className="mark-head">
      <h1 id="settings-title" className="mark">Settings &amp; about</h1>
      <p className="prose">TennoScope is a free, open-source, local-first companion. GPLv3 · MVP.</p>
    </div>
    <div className="clauses">
      <article className="clause">
        <span className="clause-index" aria-hidden="true">I</span>
        <div>
          <h2>Local-first storage</h2>
          <p className="prose">Your inventory snapshot and preferences are stored on this device in the application data directory. The UI has no telemetry or cloud account.</p>
        </div>
      </article>
      <article className="clause caution">
        <span className="clause-index" aria-hidden="true">Caution</span>
        <div>
          <h2>Read-only access disclosure</h2>
          <p className="prose">TennoScope inspects the running game process. Third-party software and process inspection may carry account-policy or anti-cheat risk even when no game memory is modified.</p>
        </div>
      </article>
      <article className="clause">
        <span className="clause-index" aria-hidden="true">II</span>
        <div>
          <h2>Automatic synchronization</h2>
          <p className="prose">The local EE.log monitor watches for inventory synchronization and refreshes automatically. Manual refresh remains available in the masthead.</p>
        </div>
      </article>
      <article className="clause">
        <span className="clause-index" aria-hidden="true">III</span>
        <div>
          <h2>Reward overlay</h2>
          <p className="prose">Reward names are read from the screen with OCR and matched against the squad's own relic pool. The strip is non-focusable and click-through, so it never takes input from the game. Placement is compositor-specific — preview it here to check where it lands.</p>
          <OverlayPreviewToggle/>
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
