import type { RewardCard } from './backend'
import { MetalMark } from './MetalMark'

/**
 * Platinum and ducats are two different answers to "which one do I take", and the player picks
 * between them for reasons this program cannot see -- saving for Baro, or just not wanting to sit
 * in trade chat. So both are shown at the same weight, in their own metal, with the leader marked
 * inside the column it won, rather than one headline number and a footnote.
 *
 * A reading we do not trust is never crowned on either metal. A price we do not have is struck as
 * a dash: untradeable items (Forma among them) have no listing and never will.
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
    return <article
      key={`${card.name}-${index}`}
      className={['slip', topPlat ? 'top-plat' : '', topDucat ? 'top-ducat' : ''].filter(Boolean).join(' ')}
      aria-label={card.name}
    >
      <span className="slip-lot">Choice {index + 1}</span>
      <h2 className="slip-name">{card.name}</h2>

      <div className="metals">
        <span className="metal plat">
          {card.platinum > 0
            ? <span className="metal-figure">{card.platinum}</span>
            : <span className="metal-figure market-pending"><span aria-hidden="true">—</span><span className="sr-only">No platinum price</span></span>}
          <span className="metal-label"><MetalMark metal="plat"/>plat</span>
          {topPlat && <span className="metal-hallmark">Top plat</span>}
        </span>
        <span className="metal ducat">
          <span className="metal-figure">{card.ducats}</span>
          <span className="metal-label"><MetalMark metal="ducat"/>ducats</span>
          {topDucat && <span className="metal-hallmark">Top ducats</span>}
        </span>
      </div>

      <div className="marks">
        {card.owned > 0
          ? <span className="hallmark owned">Owned ×{card.owned}</span>
          : <span className="hallmark absent">Not owned</span>}
        {card.mastery_relevant && <span className="hallmark mastered">Mastery needed</span>}
        {uncertain && <span className="hallmark doubt">Uncertain · {Math.round(card.confidence * 100)}%</span>}
      </div>
    </article>
  })}</div>
}
