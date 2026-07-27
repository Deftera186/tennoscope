import { useEffect, useRef, useState } from 'react'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getView, type AppView } from './backend'
import { RewardCards } from './RewardCards'

export default function RewardOverlay() {
  const [view, setView] = useState<AppView | null>(null)
  const [error, setError] = useState(false)
  const generation = useRef(0)

  useEffect(() => {
    document.documentElement.classList.add('overlay-mode')
    let active = true
    let timer: ReturnType<typeof setTimeout> | undefined
    let unlisten: UnlistenFn | undefined
    const refresh = async () => {
      const request = ++generation.current
      try {
        const next = await getView()
        if (active && request === generation.current) { setView(next); setError(false) }
      } catch {
        if (active && request === generation.current) setError(true)
      }
    }
    const poll = async () => {
      await refresh()
      if (active) timer = setTimeout(poll, 1500)
    }
    void listen('reward-updated', () => { void refresh() }).then(stop => {
      if (active) unlisten = stop
      else stop()
    })
    void poll()
    return () => { active = false; generation.current += 1; unlisten?.(); document.documentElement.classList.remove('overlay-mode'); if (timer) clearTimeout(timer) }
  }, [])

  return <main className="overlay-shell" aria-label="Reward overlay">
    {error ? <div className="overlay-empty">Reward data is unavailable.</div>
      : !view ? <div className="overlay-empty">Loading reward choices…</div>
        : view.reward.cards.length
          ? <RewardCards cards={view.reward.cards} bestValueIndex={view.reward.best_value_index} bestDucatIndex={view.reward.best_ducat_index} className="overlay-rewards"/>
          : <div className="overlay-empty"><strong>No reward choices detected</strong><span>Watching the active Warframe display.</span></div>}
  </main>
}
