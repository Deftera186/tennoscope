export const COLLECTION_PAGE_SIZE = 48

/** What the pile is worth, or null when the item has no price. */
export function stackValue(item: { quantity: number; platinum?: number }): number | null {
  return item.platinum === undefined ? null : item.platinum * item.quantity
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
