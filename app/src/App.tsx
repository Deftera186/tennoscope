import { useCallback, useEffect, useRef, useState } from 'react'
import './App.css'
import { acceptRiskDisclosure, getSetupStatus, getView, refreshInventory, type AppView } from './backend'

function App() {
  const [accepted, setAccepted] = useState<boolean | null>(null)
  const [view, setView] = useState<AppView | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const viewGeneration = useRef(0)

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
      if (document.hidden) { schedule(); return }
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
      await acceptRiskDisclosure()
      setAccepted(true)
      await requestView(getView, 'The local application backend is unavailable.')
    } catch {
      setError('Setup could not be saved.')
    } finally {
      setBusy(false)
    }
  }

  async function refresh() {
    setBusy(true)
    setError(null)
    await requestView(refreshInventory, 'Inventory refresh failed. Check acquisition health below.')
    setBusy(false)
  }

  if (accepted === null && !error) return <main className="app-shell"><p>Starting Warframe Helper…</p></main>
  if (!accepted) {
    return <main className="app-shell">
      <section className="welcome-card setup-card">
        <p className="eyebrow">Initial setup</p>
        <h1>Read-only game access</h1>
        <p>Warframe Helper reads the running Warframe process to discover temporary inventory authorization, then requests your inventory directly from Warframe’s API.</p>
        <p>No credentials or raw player payloads are logged or uploaded by this app. However, third-party software and process inspection may carry account-policy or anti-cheat risk, even when access is read-only.</p>
        <p>After acceptance, automatic read-only acquisition is enabled by default and this disclosure is not shown again.</p>
        {error && <p role="alert">{error}</p>}
        <button type="button" onClick={accept} disabled={busy}>{busy ? 'Saving…' : 'Accept and continue'}</button>
      </section>
    </main>
  }

  return <main className="app-shell dashboard">
    <header className="app-header"><div><p className="eyebrow">Local-first companion</p><h1>Warframe Helper</h1></div><button type="button" onClick={refresh} disabled={busy}>{busy ? 'Refreshing…' : 'Refresh inventory'}</button></header>
    {error && <p role="alert">{error}</p>}
    <section className="panel"><div className="panel-heading"><h2>Collection</h2><span>{view?.collection.total_entries ?? 0} items</span></div>
      {view?.collection.items.length ? <ul>{view.collection.items.map(item => <li key={item.id}>{item.name} × {item.quantity}</li>)}</ul> : <p>Your synchronized inventory will appear here when Warframe is running.</p>}
    </section>
    <section className="panel"><h2>Acquisition health</h2><div className="health-grid">
      {view && Object.entries({ Game: view.health.game_reader, 'EE.log monitor': view.health.log_monitor, Catalog: view.health.catalog, Database: view.health.database }).map(([name, health]) => <article key={name} className={`health ${health.state}`} aria-label={`${name} health: ${health.state}`}><strong>{name}</strong><span>{health.message}</span></article>)}
    </div>
    {view?.health.acquisition_stages.length ? <div className="stage-list" aria-label="Acquisition stages">
      {view.health.acquisition_stages.map(stage => {
        const words = stage.stage.replaceAll('_', ' ')
        const name = words[0]?.toUpperCase() + words.slice(1)
        return <article key={stage.stage} className={`health ${stage.state}`} aria-label={`${name} health: ${stage.state}`}><strong>{name}</strong><span>{stage.message}</span></article>
      })}
    </div> : null}</section>
  </main>
}

export default App
