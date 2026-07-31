export const COLLECTION_PAGE_SIZE = 48

/** What the pile is worth, or null when the item has no price. */
export function stackValue(item: { quantity: number; platinum?: number }): number | null {
  return item.platinum === undefined ? null : item.platinum * item.quantity
}

/**
 * What the pile is worth to a seller: the same unit price, over only as many copies as
 * warframe.market completes in a month, and only if one copy clears `floor`.
 *
 * A correct unit price is half of what a holding is worth, and the collection total was reading as
 * if it were all of it. This account owns 182 Quickdraw at a true 2p; the entire game traded two of
 * them in twenty-eight days, so 364p of that total is 4p. An item nobody bought is worth nothing in
 * bulk however dear one copy is — `Scan Matter` is 240p and has traded zero times — so a missing
 * count means zero rather than "no limit".
 *
 * The floor is the player's own, and defaults to counting everything. What the market completes is
 * measured; whether a 3p mod is worth an evening of arranging the trade by hand is not something
 * this app can measure for somebody else, and 62% of a real account's sellable total sits at 1–5p.
 */
export function sellableValue(
  item: { quantity: number; platinum?: number; monthly_trades?: number },
  floor = 0,
): number | null {
  if (item.platinum === undefined) return null
  if (item.platinum < floor) return 0
  return item.platinum * Math.min(item.quantity, item.monthly_trades ?? 0)
}

/**
 * Whether these copies are fully ranked, or null when the ceiling is unknown — which is not the
 * same answer as "no". A riven publishes a sentinel instead of a rank limit, and a card that might
 * be maxed must not be drawn as one that certainly is not.
 */
export function atMaxRank(item: { rank?: number; max_rank?: number }): boolean | null {
  if (item.max_rank === undefined) return null
  return (item.rank ?? 0) >= item.max_rank
}

/**
 * How the rank reads on the card, or null for the unranked stack — which is every mod's default
 * state and most of the collection. Labelling 674 cards "Rank 0/10" would bury the 268 that say
 * something.
 */
export function rankLabel(item: { rank?: number; max_rank?: number }): string | null {
  if (item.rank === undefined) return null
  return item.max_rank === undefined ? `Rank ${item.rank}` : `Rank ${item.rank}/${item.max_rank}`
}

export function pageCount(itemCount: number): number {
  return Math.max(1, Math.ceil(itemCount / COLLECTION_PAGE_SIZE))
}

export function clampPage(page: number, itemCount: number): number {
  return Math.min(Math.max(1, Math.trunc(page) || 1), pageCount(itemCount))
}

export function pageItems<T>(items: readonly T[], page: number): T[] {
  const current = clampPage(page, items.length)
  const start = (current - 1) * COLLECTION_PAGE_SIZE
  return items.slice(start, start + COLLECTION_PAGE_SIZE)
}

export function pageNumbers(currentPage: number, totalPages: number): number[] {
  const total = Math.max(1, totalPages)
  const current = Math.min(Math.max(1, currentPage), total)
  if (total <= 7) return Array.from({ length: total }, (_, index) => index + 1)
  const edge = current <= 3 || current >= total - 2
  const start = current <= 3 ? 2 : current >= total - 2 ? total - 4 : current - 2
  const middle = Array.from({ length: edge ? 4 : 5 }, (_, index) => start + index)
  return [...new Set([1, ...middle, total])].sort((left, right) => left - right)
}
