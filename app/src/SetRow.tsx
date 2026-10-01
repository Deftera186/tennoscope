import type { SetPart } from './backend'

/** The game's part icons as 64px white silhouettes, masked rather than painted: `currentColor`
 * gives the metal and a copy not held is the same shape faint, so one file serves every state. */
const PART_ART = import.meta.glob('./assets/parts/*.png', { eager: true, import: 'default' }) as Record<string, string>

/** A dog-eared sheet with a set-square cut out: reads as "blueprint" at 28px without words. */
const BLUEPRINT = 'M4 1h11l5 5v17H4zM15 1v5h5zM7 19V10l8 9z'
/** For parts whose art is not bundled (Prime weapons consumed whole): a plain lozenge marks a part the game gives no shape for, rather than inventing one. */
const FALLBACK = 'M12 2l10 10-10 10L2 12z'

/** Solid is held and faint is not, and each slot's label says the same in words. At most two
 * copies are drawn, since a third overlapping shape stops reading as a quantity; ×n says more. */
export function SetRow({ parts }: { parts: SetPart[] }) {
  return <ul className="set-row" aria-label="Set">
    {parts.map((part, index) => {
      const art = part.image ? PART_ART[`./assets/parts/${part.image}`] : undefined
      const copies = Math.min(part.uses, 2)
      // "3 of 1 held" reads as a contradiction, so a slot held to its need or past it says so in words.
      const holding = part.held >= part.uses ? 'held in full' : `${part.held} of ${part.uses} held`
      return <li
        key={`${part.name}-${index}`}
        className="set-slot"
        data-this={part.this || undefined}
        aria-label={`${part.name}, ${holding}${part.this ? ', this reward' : ''}`}
      >
        {Array.from({ length: copies }, (_, copy) => art
          ? <i
            key={copy}
            className="set-glyph"
            aria-hidden="true"
            data-held={copy < part.held}
            style={{ WebkitMaskImage: `url(${art})`, maskImage: `url(${art})` }}
          />
          : <svg key={copy} className="set-glyph" viewBox="0 0 24 24" aria-hidden="true" data-held={copy < part.held}>
            <path d={part.image === 'blueprint.png' ? BLUEPRINT : FALLBACK} fillRule="evenodd" />
          </svg>)}
        {part.uses > 2 && <span className="set-count">×{part.uses}</span>}
      </li>
    })}
  </ul>
}
