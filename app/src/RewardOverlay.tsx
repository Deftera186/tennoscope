import { useEffect, useRef, useState } from 'react'
import { getView, type AppView } from './backend'
import { hideRewardOverlay } from './overlay'
import { RewardCards } from './RewardCards'

export default function RewardOverlay() {
  const [view, setView] = useState<AppView | null>(null)
  const [error, setError] = useState(false)
  const generation = useRef(0)

  useEffect(() => {
    document.documentElement.classList.add('overlay-mode')
    let active = true
    let timer: ReturnType<typeof setTimeout> | undefined
    const update = async () => {
      const request = ++generation.current
      try {
        const next = await getView()
        if (active && request === generation.current) { setView(next); setError(false) }
      } catch {
        if (active && request === generation.current) setError(true)
      }
      if (active) timer = setTimeout(update, 1500)
    }
    void update()
    return () => { active = false; generation.current += 1; document.documentElement.classList.remove('overlay-mode'); if (timer) clearTimeout(timer) }
  }, [])

  return <main className="overlay-shell" aria-label="Reward overlay">
    <div className="overlay-title"><span>Reward advisor</span><button type="button" aria-label="Hide reward overlay" onClick={() => void hideRewardOverlay()}>×</button></div>
    {error ? <div className="overlay-empty">Reward data is unavailable.</div>
      : !view ? <div className="overlay-empty">Loading reward choices…</div>
        : view.reward.cards.length
          ? <RewardCards cards={view.reward.cards} bestValueIndex={view.reward.best_value_index} className="overlay-rewards"/>
          : <div className="overlay-empty"><strong>No reward choices detected</strong><span>Waiting for a reward source. OCR is not connected yet.</span></div>}
  </main>
}
