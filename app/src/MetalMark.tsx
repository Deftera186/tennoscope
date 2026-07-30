import platinumMark from './assets/platinum.png'
import ducatMark from './assets/ducats.png'

/**
 * The two currencies, as the game itself draws them.
 *
 * Until now the difference between a platinum reading and a ducat reading was carried by hue and by
 * a tracked mono word set at half a rem -- eight pixels, over a bright moving game, under a
 * countdown. A player who does not separate those two hues had only the word.
 *
 * These are Digital Extremes' own icons rather than marks of our own, because recognition here is
 * not a design problem to solve, it is a memory the player already has: they have seen the platinum
 * canister and the Orokin ducat sigil thousands of times in the game running behind this overlay.
 * Anything drawn in our own grammar, however well it fit the touchstone world, would be a third
 * shape to learn -- and the flattenings all read as something else (a battery, a SIM card, a media
 * control). Recognition wins over house style on the one surface read in under a second.
 *
 * They are bundled rather than fetched. Collection artwork may arrive over the network because a
 * missing thumbnail costs a placeholder; chrome that resolves which currency a figure is in must
 * never depend on the network, so these ship in the binary and need no CSP origin.
 *
 * Sized by the caller, and never below about 18px: these are rendered objects rather than flat
 * marks, and below that the canister loses its dark chip and reads as a grey lozenge. That floor is
 * why every placement sizes the mark independently of the text it sits with, instead of scaling
 * with it -- see `.metal-label`, `.price` and `.band-figure` in App.css.
 */
export function MetalMark({ metal, alt = '', className = 'metal-mark' }: {
  metal: 'plat' | 'ducat'
  alt?: string
  className?: string
}) {
  return <img
    className={className}
    data-metal={metal}
    src={metal === 'plat' ? platinumMark : ducatMark}
    alt={alt}
    aria-hidden={alt ? undefined : true}
    draggable={false}
    decoding="async"
  />
}
