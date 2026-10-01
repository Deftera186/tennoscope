import type { MasteryMark, RewardCard, SetPart } from './backend'
import { MetalMark } from './MetalMark'
import { SetRow } from './SetRow'

/**
 * The words the ledger states, written as the player reads them and uppercased by the stylesheet.
 * The space before a `·` never breaks, so a narrow slip wraps after the separator, not before it.
 */
function ledgerWord(mark: Exclude<MasteryMark, { state: 'unknown' }>): string {
  switch (mark.state) {
    case 'mastered': return 'Mastered'
    case 'built': return `Built\u00a0· R${mark.rank}/${mark.max_rank}`
    case 'in_foundry': return 'In foundry'
    // A chain mark names the Ak that still wants this part's item, read as one phrase: "Unmastered
    // Aklex" on a Lex part, which is why the part still matters when the Lex is mastered or built.
    case 'unmastered': return mark.completes
      ? 'Completes set'
      : mark.subject ? `Unmastered ${mark.subject}` : 'Unmastered'
  }
}

/** What a mark asks of the slip around it: the rule the article carries, the tone its word is drawn in, and the set the line may show. */
type LedgerState = { rule: '' | 'ruled' | 'ruled-double'; tone: 'loud' | 'quiet'; inverted: boolean; parts: SetPart[] | null }

/** Resolved once per card so the slip's rule and its ledger never disagree. A read we are unsure
 * of may name the wrong item, so it earns neither rule nor set row and keeps its doubt mark. */
function ledgerState(mark: MasteryMark, uncertain: boolean): LedgerState {
  const unmastered = mark.state === 'unmastered' ? mark : null
  const trusted = !uncertain
  const missing = trusted && !!unmastered?.missing
  const completes = missing && !!unmastered?.completes
  return {
    rule: completes ? 'ruled-double' : missing ? 'ruled' : '',
    tone: missing ? 'loud' : 'quiet',
    inverted: completes,
    parts: trusted && unmastered ? unmastered.parts : null,
  }
}

/** The doubt mark, the word that states the mark, and the set row at the far end of the line. An
 * unanswerable mark is the app's struck dash, and a screen reader hears what it means. */
function Ledger({ mark, state, uncertain, confidence }: { mark: MasteryMark; state: LedgerState; uncertain: boolean; confidence: number }) {
  return <div className="ledger" data-tone={state.tone}>
    {uncertain && <span className="hallmark doubt">Uncertain · {Math.round(confidence * 100)}%</span>}
    {mark.state === 'unknown'
      ? <span className="ledger-word"><span aria-hidden="true">—</span><span className="sr-only">Mastery unknown</span></span>
      : <span className={`ledger-word${state.inverted ? ' inverted' : ''}`}>{ledgerWord(mark)}</span>}
    {state.parts && <SetRow parts={state.parts}/>}
  </div>
}

/**
 * Platinum and ducats are two different answers to "which one do I take", and the player picks
 * between them for reasons this program cannot see: saving for Baro, or just not wanting to sit
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
  ownershipVerified = true,
  className = 'reward-grid',
}: {
  cards: RewardCard[]
  bestValueIndex: number | null
  bestDucatIndex: number | null
  ownershipVerified?: boolean
  className?: string
}) {
  return <div className={className}>{cards.slice(0, 4).map((card, index) => {
    const uncertain = card.confidence < 0.8
    const topPlat = !uncertain && bestValueIndex === index
    const topDucat = !uncertain && bestDucatIndex === index
    const marked = card.mastery ? { mark: card.mastery, state: ledgerState(card.mastery, uncertain) } : null
    return <article
      key={`${card.name}-${index}`}
      className={['slip', topPlat ? 'top-plat' : '', topDucat ? 'top-ducat' : '', marked?.state.rule].filter(Boolean).join(' ')}
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

      {marked
        // A mark replaces the ownership line: the set row already says how much is held, and two
        // lines do not fit the overlay's width. A card with no mark keeps the line.
        ? <Ledger mark={marked.mark} state={marked.state} uncertain={uncertain} confidence={card.confidence}/>
        : <div className="marks">
            {ownershipVerified
              ? card.owned > 0
                ? <span className="hallmark owned">Owned ×{card.owned}</span>
                : <span className="hallmark absent">Not owned</span>
              : <span className="hallmark doubt">Unverifiable</span>}
            {uncertain && <span className="hallmark doubt">Uncertain · {Math.round(card.confidence * 100)}%</span>}
          </div>}
    </article>
  })}</div>
}
