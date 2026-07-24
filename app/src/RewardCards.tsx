import type { RewardCard } from './backend'

export function RewardCards({ cards, bestValueIndex, className = 'reward-grid' }: { cards: RewardCard[]; bestValueIndex: number | null; className?: string }) {
  return <div className={className}>{cards.slice(0, 4).map((card, index) => {
    const uncertain = card.confidence < 0.8
    const best = !uncertain && bestValueIndex === index
    return <article key={`${card.name}-${index}`} className={best ? 'reward-card best' : 'reward-card'} aria-label={card.name}>
      {best && <span className="best-ribbon">Best value</span>}
      <span className="reward-index">Choice {index + 1}</span>
      <h2>{card.name}</h2>
      <div className="value-row"><strong>{card.platinum}<small> plat</small></strong><span>{card.ducats} ducats</span></div>
      <div className="badges">
        {card.owned > 0 ? <span className="badge quantity">Owned ×{card.owned}</span> : <span className="badge missing">Not owned</span>}
        {card.mastery_relevant && <span className="badge mastered">✦ Mastery needed</span>}
        {uncertain && <span className="badge confidence">Uncertain recognition · {Math.round(card.confidence * 100)}%</span>}
      </div>
    </article>
  })}</div>
}
