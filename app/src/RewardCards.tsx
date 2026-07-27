import type { RewardCard } from './backend'

/**
 * Platinum and ducats are two different answers to "which one do I take", and the player picks
 * between them for reasons this program cannot see -- saving for Baro, or just not wanting to sit
 * in trade chat. So both are shown at the same weight, with the leader in each column marked,
 * rather than one headline number and a footnote.
 */
export function RewardCards({
  cards,
  bestValueIndex,
  bestDucatIndex,
  className = 'reward-grid',
}: {
  cards: RewardCard[]
  bestValueIndex: number | null
  bestDucatIndex: number | null
  className?: string
}) {
  return <div className={className}>{cards.slice(0, 4).map((card, index) => {
    const uncertain = card.confidence < 0.8
    const topPlat = !uncertain && bestValueIndex === index
    const topDucat = !uncertain && bestDucatIndex === index
    const pick = topPlat && topDucat ? 'Top plat & ducats' : topPlat ? 'Top plat' : topDucat ? 'Top ducats' : null
    return <article
      key={`${card.name}-${index}`}
      className={['reward-card', topPlat ? 'top-plat' : '', topDucat ? 'top-ducat' : ''].filter(Boolean).join(' ')}
      aria-label={card.name}
    >
      {pick && <span className="pick-flag">{pick}</span>}
      <span className="reward-index">Choice {index + 1}</span>
      <h2>{card.name}</h2>
      <div className="value-row">
        <span className={topPlat ? 'metric plat lead' : 'metric plat'}>
          {card.platinum > 0
            ? <strong>{card.platinum}</strong>
            : <strong className="market-pending" aria-label="platinum price unavailable">—</strong>}
          <small>plat</small>
        </span>
        <span className={topDucat ? 'metric ducat lead' : 'metric ducat'}>
          <strong>{card.ducats}</strong><small>ducats</small>
        </span>
      </div>
      <div className="badges">
        {card.owned > 0 ? <span className="badge quantity">Owned ×{card.owned}</span> : <span className="badge missing">Not owned</span>}
        {card.mastery_relevant && <span className="badge mastered">✦ Mastery needed</span>}
        {uncertain && <span className="badge confidence">Uncertain · {Math.round(card.confidence * 100)}%</span>}
      </div>
    </article>
  })}</div>
}
