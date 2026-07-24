import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import './App.css'
import {
  acceptRiskDisclosure,
  getSetupStatus,
  getView,
  refreshInventory,
  type AppView,
  type BackendHealth,
  type CollectionItem,
  type HealthState,
  type ItemCategory,
} from './backend'
import { showRewardOverlay } from './overlay'
import { RewardCards } from './RewardCards'

type Page = 'collection' | 'rewards' | 'diagnostics' | 'settings'
type Ownership = 'all' | 'owned' | 'mastered' | 'missing'
type Sort = 'name-asc' | 'quantity-desc' | 'category-asc'

const categories: Array<{ value: ItemCategory | 'all'; label: string; glyph: string }> = [
  { value: 'all', label: 'All categories', glyph: '◇' },
  { value: 'frame', label: 'Frame', glyph: 'F' },
  { value: 'weapon', label: 'Weapon', glyph: 'W' },
  { value: 'companion', label: 'Companion', glyph: 'C' },
  { value: 'prime_part', label: 'Prime Part', glyph: 'P' },
  { value: 'relic', label: 'Relic', glyph: 'R' },
  { value: 'resource', label: 'Resource', glyph: '•' },
  { value: 'blueprint', label: 'Blueprint', glyph: 'B' },
  { value: 'vehicle', label: 'Vehicle', glyph: 'V' },
]

const categoryName = Object.fromEntries(categories.map(category => [category.value, category.label])) as Record<ItemCategory | 'all', string>

function Icon({ name }: { name: Page | 'refresh' }) {
  const paths = {
    collection: <><path d="M4 5.5 12 2l8 3.5v13L12 22l-8-3.5z"/><path d="M4 5.5 12 9l8-3.5M12 9v13"/></>,
    rewards: <><path d="M12 3 9.5 8 4 9l4 4-.9 5.7L12 16l4.9 2.7L16 13l4-4-5.5-1z"/></>,
    diagnostics: <><path d="M4 18V9m5 9V5m6 13v-7m5 7V3"/></>,
    settings: <><circle cx="12" cy="12" r="3"/><path d="M19 13.5v-3l-2.1-.6a7 7 0 0 0-.7-1.7l1.1-1.9-2.1-2.1-1.9 1.1a7 7 0 0 0-1.7-.7L11 2H8l-.6 2.1a7 7 0 0 0-1.7.7L3.8 3.7 1.7 5.8l1.1 1.9a7 7 0 0 0-.7 1.7L0 10v3l2.1.6c.2.6.4 1.2.7 1.7l-1.1 1.9 2.1 2.1 1.9-1.1c.5.3 1.1.5 1.7.7L8 21h3l.6-2.1c.6-.2 1.2-.4 1.7-.7l1.9 1.1 2.1-2.1-1.1-1.9c.3-.5.5-1.1.7-1.7z" transform="translate(2.5 .5) scale(.8)"/></>,
    refresh: <><path d="M20 7v5h-5"/><path d="M18.5 16a8 8 0 1 1 1.1-8.2L20 12"/></>,
  }
  return <svg className="icon" viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">{paths[name]}</svg>
}

function App() {
  const [accepted, setAccepted] = useState<boolean | null>(null)
  const [view, setView] = useState<AppView | null>(null)
  const [page, setPage] = useState<Page>('collection')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
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

  if (accepted === null && !error) return <main className="startup"><div className="spinner" aria-hidden="true"/><p>Starting Warframe Helper…</p></main>
  if (!accepted) return <SetupScreen busy={busy} error={error} onAccept={accept}/>

  const liveState = view?.health.game_reader.state ?? 'degraded'
  return <div className="app-frame">
    <aside className="sidebar">
      <div className="brand" aria-label="Warframe Helper">
        <span className="brand-mark">WH</span>
        <span><strong>Warframe</strong><small>Helper</small></span>
      </div>
      <nav aria-label="Primary">
        {(['collection', 'rewards', 'diagnostics', 'settings'] as const).map(item => <button
          key={item}
          type="button"
          aria-label={item === 'rewards' ? 'Rewards' : item[0].toUpperCase() + item.slice(1)}
          className={page === item ? 'nav-item active' : 'nav-item'}
          aria-current={page === item ? 'page' : undefined}
          onClick={() => setPage(item)}
        ><Icon name={item}/><span>{item === 'rewards' ? 'Rewards' : item[0].toUpperCase() + item.slice(1)}</span>{item === 'rewards' && view?.reward.cards.length ? <em>{view.reward.cards.length}</em> : null}</button>)}
      </nav>
      <div className="privacy-note"><span aria-hidden="true">⌂</span><p><strong>Local only</strong><br/>Your collection stays here.</p></div>
    </aside>

    <main className="workspace">
      <header className="topbar">
        <div className={`live-state ${liveState}`} role="status"><span/><div><strong>{liveState === 'ready' ? 'Watching Warframe' : 'Attention needed'}</strong><small>{view?.health.game_reader.message ?? 'Connecting to local backend'}</small></div></div>
        <button type="button" className="refresh-button" onClick={refresh} disabled={busy}><Icon name="refresh"/>{busy ? 'Refreshing…' : 'Refresh inventory'}</button>
      </header>
      {error && <p className="error-banner" role="alert">{error}</p>}
      <div className="content">
        {!view ? <LoadingView/> : <>
          {page === 'collection' && <CollectionPage view={view}/>}
          {page === 'rewards' && <RewardPage view={view}/>}
          {page === 'diagnostics' && <DiagnosticsPage view={view}/>}
          {page === 'settings' && <SettingsPage/>}
        </>}
      </div>
    </main>
  </div>
}

function SetupScreen({ busy, error, onAccept }: { busy: boolean; error: string | null; onAccept: () => void }) {
  return <main className="setup-shell">
    <section className="setup-card" aria-labelledby="setup-title">
      <div className="setup-brand"><span className="brand-mark">WH</span><span>WARFRAME HELPER</span></div>
      <p className="eyebrow">One-time setup · Read carefully</p>
      <h1 id="setup-title">Read-only game access</h1>
      <p className="setup-lead">Automatic inventory sync needs permission to inspect the running Warframe process and make a direct inventory request.</p>
      <div className="disclosure-grid">
        <article><span className="disclosure-icon safe">✓</span><div><h2>Private by design</h2><p>The app never logs or uploads credentials or raw player payloads. Collection data stays on this device.</p></div></article>
        <article><span className="disclosure-icon risk">!</span><div><h2>Know the risk</h2><p>Third-party software and process inspection may carry account-policy or anti-cheat risk, even when access is read-only.</p></div></article>
      </div>
      <p className="setup-footnote">After acceptance, automatic read-only acquisition is enabled by default. You can revisit this disclosure in Settings.</p>
      {error && <p className="error-banner" role="alert">{error}</p>}
      <button type="button" className="primary-action" onClick={onAccept} disabled={busy}>{busy ? 'Saving locally…' : 'Accept risk and continue'}<span aria-hidden="true">→</span></button>
    </section>
  </main>
}

function LoadingView() {
  return <section className="state-card" aria-live="polite"><div className="spinner" aria-hidden="true"/><h1>Loading your local collection…</h1><p>Reading the latest saved snapshot.</p></section>
}

function CollectionPage({ view }: { view: AppView }) {
  const [search, setSearch] = useState('')
  const [category, setCategory] = useState<ItemCategory | 'all'>('all')
  const [ownership, setOwnership] = useState<Ownership>('all')
  const [sort, setSort] = useState<Sort>('name-asc')
  const masteryEligible = view.collection.items.filter(item => ['frame', 'weapon', 'companion', 'vehicle'].includes(item.category))
  const mastered = masteryEligible.filter(item => item.mastered).length
  const owned = view.collection.items.filter(item => item.quantity > 0).length
  const missing = view.collection.items.filter(item => item.quantity === 0).length
  const filtered = useMemo(() => {
    const query = search.trim().toLocaleLowerCase()
    return view.collection.items
      .filter(item => !query || item.name.toLocaleLowerCase().includes(query))
      .filter(item => category === 'all' || item.category === category)
      .filter(item => ownership === 'all' || (ownership === 'owned' && item.quantity > 0) || (ownership === 'mastered' && item.mastered) || (ownership === 'missing' && item.quantity === 0))
      .toSorted((left, right) => sort === 'quantity-desc'
        ? right.quantity - left.quantity || left.name.localeCompare(right.name)
        : sort === 'category-asc'
          ? left.category.localeCompare(right.category) || left.name.localeCompare(right.name)
          : left.name.localeCompare(right.name))
  }, [view.collection.items, search, category, ownership, sort])

  return <section className="page" aria-labelledby="collection-title">
    <div className="page-heading"><div><p className="eyebrow">Inventory snapshot</p><h1 id="collection-title">Your collection</h1><p>Browse the canonical items synchronized from your account.</p></div><span className="snapshot-mark">Local snapshot</span></div>
    <div className="summary-grid">
      <SummaryCard label="Items tracked" value={view.collection.total_entries} detail={`${owned} currently owned`} kind="items"/>
      <SummaryCard label="Mastered" value={mastered} detail={masteryEligible.length ? `${Math.round(mastered / masteryEligible.length * 100)}% of mastery-eligible items` : 'No mastery-eligible items'} kind="mastered"/>
      <SummaryCard label="Missing" value={missing} detail="From known collection data" kind="missing"/>
    </div>
    <section className="collection-panel">
      <div className="collection-tools">
        <label className="search-field"><span aria-hidden="true">⌕</span><span className="sr-only">Search collection</span><input type="search" aria-label="Search collection" placeholder="Search canonical item names…" value={search} onChange={event => setSearch(event.target.value)}/></label>
        <label className="sort-field"><span>Sort</span><select aria-label="Sort collection" value={sort} onChange={event => setSort(event.target.value as Sort)}><option value="name-asc">Name A–Z</option><option value="quantity-desc">Quantity</option><option value="category-asc">Category</option></select></label>
      </div>
      <div className="category-strip" aria-label="Item categories">{categories.map(item => <button type="button" key={item.value} className={category === item.value ? 'chip selected' : 'chip'} aria-label={item.label} aria-pressed={category === item.value} onClick={() => setCategory(item.value)}><span aria-hidden="true">{item.glyph}</span>{item.label}</button>)}</div>
      <div className="result-bar"><div className="segmented" aria-label="Ownership filters">{(['all', 'owned', 'mastered', 'missing'] as const).map(filter => <button type="button" key={filter} aria-pressed={ownership === filter} onClick={() => setOwnership(filter)}>{filter[0].toUpperCase() + filter.slice(1)}</button>)}</div><span>{filtered.length} result{filtered.length === 1 ? '' : 's'}</span></div>
      {filtered.length
        ? <ul className="collection-grid" aria-label="Collection items">{filtered.map(item => <li key={item.id}><CollectionCard item={item}/></li>)}</ul>
        : <EmptyState title={view.collection.items.length ? 'No matching items' : 'No inventory items yet'} detail={view.collection.items.length ? 'Try another search or clear a filter.' : 'Start Warframe and refresh to create your first local snapshot.'}/>}
    </section>
  </section>
}

function SummaryCard({ label, value, detail, kind }: { label: string; value: number; detail: string; kind: string }) {
  return <article className={`summary-card ${kind}`} data-summary={kind}><span className="summary-symbol" aria-hidden="true">{kind === 'mastered' ? '✦' : kind === 'missing' ? '○' : '◈'}</span><div><small>{label}</small><strong>{value}</strong><p>{detail}</p></div></article>
}

function CollectionCard({ item }: { item: CollectionItem }) {
  const missing = item.quantity === 0
  return <article className={`item-card category-${item.category}`} aria-label={item.name}>
    <div className="item-art" aria-hidden="true"><span>{categoryName[item.category].slice(0, 2).toUpperCase()}</span><i/></div>
    <div className="item-body"><span className="category-label">{categoryName[item.category]}</span><h2>{item.name}</h2><div className="badges">{missing ? <span className="badge missing">Missing</span> : <span className="badge quantity">Owned ×{item.quantity}</span>}{item.mastered && <span className="badge mastered">✦ Mastered</span>}</div></div>
  </article>
}

function RewardPage({ view }: { view: AppView }) {
  return <div className="page"><div className="page-heading"><div><p className="eyebrow">Decision support</p><h1 id="reward-title">Reward advisor</h1><p>Current reward candidates only. Screen capture and OCR are not connected yet.</p></div><span className="snapshot-mark">Foundation preview</span></div>
    <section className="reward-panel" aria-label="Reward advisor">{view.reward.cards.length ? <RewardCards cards={view.reward.cards} bestValueIndex={view.reward.best_value_index}/> : <EmptyState title="No reward choices detected" detail="The advisor will show up to four choices when a reward source is connected. No OCR is performed in this MVP."/>}</section>
  </div>
}

function HealthCard({ label, health }: { label: string; health: BackendHealth | { state: HealthState; message: string; last_success?: string | null } }) {
  return <article className={`diagnostic-card ${health.state}`}><span className="health-dot" aria-hidden="true"/><div><h2>{label}</h2><p>{health.message}</p>{health.last_success && <small>Last success: {health.last_success}</small>}</div><strong>{health.state}</strong></article>
}

function DiagnosticsPage({ view }: { view: AppView }) {
  const systems = [
    ['Game reader', view.health.game_reader],
    ['EE.log', view.health.log_monitor],
    ['Screen capture', view.health.capture],
    ['Catalog', view.health.catalog],
    ['Market data', view.health.market],
    ['Database', view.health.database],
  ] as const
  return <div className="page"><div className="page-heading"><div><p className="eyebrow">Local system health</p><h1 id="diagnostics-title">Diagnostics</h1><p>Status messages are deliberately scrubbed of temporary access values.</p></div></div>
    <section className="diagnostics-panel" aria-label="Diagnostics"><div className="section-heading"><h2 className="section-title">Core services</h2><button type="button" className="secondary-action" onClick={() => void showRewardOverlay()}>Preview reward overlay</button></div><div className="diagnostic-grid">{systems.map(([label, health]) => <HealthCard key={label} label={label} health={health}/>)}</div><h2 className="section-title">Acquisition pipeline</h2>{view.health.acquisition_stages.length ? <ol className="pipeline">{view.health.acquisition_stages.map((stage, index) => { const words = stage.stage.replaceAll('_', ' '); const label = words[0].toUpperCase() + words.slice(1); return <li key={stage.stage}><span className={`stage-number ${stage.state}`}>{index + 1}</span><div><strong>{label}</strong><p>{stage.message}</p></div><span className={`stage-state ${stage.state}`}>{stage.state}</span></li> })}</ol> : <EmptyState title="No acquisition attempt yet" detail="Start Warframe or request a refresh to populate the five pipeline stages."/>}</section>
  </div>
}

function SettingsPage() {
  return <section className="page" aria-labelledby="settings-title"><div className="page-heading"><div><p className="eyebrow">Application</p><h1 id="settings-title">Settings &amp; about</h1><p>Warframe Helper is a free, open-source, local-first companion.</p></div><span className="snapshot-mark">MVP · GPLv3</span></div>
    <div className="settings-grid"><article className="settings-card"><span className="settings-icon">⌂</span><div><h2>Local-first storage</h2><p>Your inventory snapshot and preferences are stored on this device in the application data directory. The UI has no telemetry or cloud account.</p></div></article><article className="settings-card warning"><span className="settings-icon">!</span><div><h2>Read-only access disclosure</h2><p>Warframe Helper inspects the running game process. Third-party software and process inspection may carry account-policy or anti-cheat risk even when no game memory is modified.</p></div></article><article className="settings-card"><span className="settings-icon">↻</span><div><h2>Automatic synchronization</h2><p>The local EE.log monitor watches for inventory synchronization and refreshes automatically. Manual refresh remains available in the top bar.</p></div></article><article className="settings-card"><span className="settings-icon">▱</span><div><h2>Reward overlay</h2><p>Preview the focused always-on-top reward window. It remains honest when no reward source is connected.</p><button type="button" className="secondary-action" onClick={() => void showRewardOverlay()}>Preview reward overlay</button></div></article></div>
  </section>
}

function EmptyState({ title, detail }: { title: string; detail: string }) {
  return <div className="empty-state"><span aria-hidden="true">◇</span><h2>{title}</h2><p>{detail}</p></div>
}

export default App
