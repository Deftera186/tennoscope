/**
 * Display preferences, kept in the window that draws them.
 *
 * The price floor changes one figure the frontend already computes from a view it already has. The
 * backend has no use for it, so putting it in SQLite would mean a schema migration, an IPC pair and
 * a round trip to move a slider -- for a number that never leaves this file.
 */
const FLOOR_KEY = 'tennoscope.price-floor'

/**
 * Where the slider stops. Above roughly this point the figure stops answering: on a real account
 * every floor from 21p up lands within a few percent of the last, because all that is left by then
 * is the few dozen items anybody would trade one at a time. A slider that travelled further would
 * spend most of its length saying nothing.
 */
export const MAX_PRICE_FLOOR = 20

/** Whatever came back from storage, made into a floor. Anything unreadable is no floor at all. */
export function clampPriceFloor(value: unknown): number {
  const floor = Math.round(Number(value))
  return Number.isFinite(floor) ? Math.min(Math.max(floor, 0), MAX_PRICE_FLOOR) : 0
}

/** Storage is allowed to be missing or refused -- a webview with it disabled still runs the app. */
export function readPriceFloor(): number {
  try {
    return clampPriceFloor(localStorage.getItem(FLOOR_KEY))
  } catch {
    return 0
  }
}

export function writePriceFloor(floor: number): void {
  try {
    localStorage.setItem(FLOOR_KEY, String(clampPriceFloor(floor)))
  } catch {
    // A preference that cannot be saved is still a preference for this session.
  }
}

/** Ducats show beside platinum on every prime part until somebody asks them not to. */
const DUCATS_KEY = 'tennoscope.show-ducats'

/** The only value that means "hidden". Anything unreadable is the default, not a refusal. */
export function readShowDucats(): boolean {
  try {
    return localStorage.getItem(DUCATS_KEY) !== 'false'
  } catch {
    return true
  }
}

export function writeShowDucats(show: boolean): void {
  try {
    localStorage.setItem(DUCATS_KEY, String(show))
  } catch {
    // A preference that cannot be saved is still a preference for this session.
  }
}
