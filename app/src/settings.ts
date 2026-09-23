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

/**
 * Update checking runs daily on launch, downloads only when asked. One default
 * for every install kind avoids a settings fork: system installs check too, and
 * only the *action* differs (nudge instead of install).
 */
const AUTO_CHECK_KEY = 'tennoscope.update-auto-check'

export function readUpdateAutoCheck(): boolean {
  try {
    return localStorage.getItem(AUTO_CHECK_KEY) !== 'false'
  } catch {
    return true
  }
}

export function writeUpdateAutoCheck(auto: boolean): void {
  try {
    localStorage.setItem(AUTO_CHECK_KEY, String(auto))
  } catch {
    // A preference that cannot be saved is still a preference for this session.
  }
}

/** Pre-release channel. Off unless asked: betas may break reward reads. */
const PRERELEASE_KEY = 'tennoscope.update-prerelease'

export function readUpdatePrerelease(): boolean {
  try {
    return localStorage.getItem(PRERELEASE_KEY) === 'true'
  } catch {
    return false
  }
}

export function writeUpdatePrerelease(beta: boolean): void {
  try {
    localStorage.setItem(PRERELEASE_KEY, String(beta))
  } catch {
    // A preference that cannot be saved is still a preference for this session.
  }
}

/** Last successful check, ISO instant. Missing or unreadable means "never". */
const LAST_CHECK_KEY = 'tennoscope.update-last-check'

export function readUpdateLastCheck(): number | null {
  try {
    const raw = localStorage.getItem(LAST_CHECK_KEY)
    const at = raw ? Date.parse(raw) : NaN
    return Number.isFinite(at) ? at : null
  } catch {
    return null
  }
}

export function writeUpdateLastCheck(at: number): void {
  try {
    localStorage.setItem(LAST_CHECK_KEY, new Date(at).toISOString())
  } catch {
    // A stamp that cannot be saved simply means "check again next launch".
  }
}

/**
 * Feed `pub_date` of the last version the UI surfaced, so an installed RC is
 * not re-offered when its version equals the feed version (see `beta_offers`
 * on the backend). A new date re-nudges exactly once.
 */
const LAST_SURFACED_KEY = 'tennoscope.update-last-surfaced'

export function readUpdateLastSurfaced(): string | null {
  try {
    return localStorage.getItem(LAST_SURFACED_KEY)
  } catch {
    return null
  }
}

export function writeUpdateLastSurfaced(pubDate: string): void {
  try {
    localStorage.setItem(LAST_SURFACED_KEY, pubDate)
  } catch {
    // Worst case the same version nudges twice instead of once.
  }
}

/** Dismissals per version. Two strikes and this version stays quiet. */
const DISMISS_KEY = 'tennoscope.update-dismissed'

function readDismissed(): Record<string, number> {
  try {
    const raw = localStorage.getItem(DISMISS_KEY)
    const parsed: unknown = raw ? JSON.parse(raw) : {}
    if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
      return Object.fromEntries(
        Object.entries(parsed).filter(([, n]) => typeof n === 'number'),
      )
    }
    return {}
  } catch {
    return {}
  }
}

export function dismissedCount(version: string): number {
  return readDismissed()[version] ?? 0
}

export function dismissVersion(version: string): void {
  try {
    const all = readDismissed()
    all[version] = (all[version] ?? 0) + 1
    localStorage.setItem(DISMISS_KEY, JSON.stringify(all))
  } catch {
    // A dismissal that cannot be saved simply nudges again.
  }
}

/**
 * "Not now" snoozes a version for 7 days. Separate from the double-dismiss
 * rule: a snoozed version stays quiet until the timer lapses, a twice-dismissed
 * one until the next version — whichever surfaces first wins.
 */
const SNOOZE_KEY = 'tennoscope.update-snoozed'

function readSnoozed(): Record<string, number> {
  try {
    const raw = localStorage.getItem(SNOOZE_KEY)
    const parsed: unknown = raw ? JSON.parse(raw) : {}
    if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
      return Object.fromEntries(
        Object.entries(parsed).filter(([, at]) => typeof at === 'number'),
      )
    }
    return {}
  } catch {
    return {}
  }
}

export function snoozedUntil(version: string): number | null {
  return readSnoozed()[version] ?? null
}

export function snoozeVersion(version: string, days = 7): void {
  try {
    const all = readSnoozed()
    all[version] = Date.now() + days * 86_400_000
    localStorage.setItem(SNOOZE_KEY, JSON.stringify(all))
  } catch {
    // A snooze that cannot be saved simply nudges again.
  }
}

/** A fulfilled snooze is spent: clear it so later checks re-arm normally. */
export function clearSnooze(version: string): void {
  try {
    const all = readSnoozed()
    delete all[version]
    localStorage.setItem(SNOOZE_KEY, JSON.stringify(all))
  } catch {
    // An uncleared snooze simply suppresses one more round.
  }
}
