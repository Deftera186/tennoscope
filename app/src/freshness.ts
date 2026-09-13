import type { AppView } from './backend'

type Snapshot = AppView['collection']['snapshot']

/** Relative and absolute readings of one instant. */
export type Reading = { relative: string; exact: string }

function readingOf(observed: Date, now: Date): Reading | null {
  if (Number.isNaN(observed.getTime())) return null
  const elapsedSeconds = Math.max(0, Math.floor((now.getTime() - observed.getTime()) / 1000))
  let relative = 'just now'
  if (elapsedSeconds >= 86_400) relative = `${Math.floor(elapsedSeconds / 86_400)} day${elapsedSeconds < 172_800 ? '' : 's'} ago`
  else if (elapsedSeconds >= 3_600) relative = `${Math.floor(elapsedSeconds / 3_600)} hour${elapsedSeconds < 7_200 ? '' : 's'} ago`
  else if (elapsedSeconds >= 60) relative = `${Math.floor(elapsedSeconds / 60)} minute${elapsedSeconds < 120 ? '' : 's'} ago`
  return {
    relative,
    exact: new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'medium' }).format(observed),
  }
}

/** Reads a backend stamp string, or `null` when it holds no usable time. Health rows and the market
 * order fetch time are emitted as Unix seconds (`now_unix_seconds`, `SystemTime::as_secs`) while the
 * price-dump row keeps a calendar date, so digit-only stamps are read as seconds and everything else
 * is left to `Date`. Owning that test here is what keeps callers from re-deriving it -- read as a
 * calendar string, `1785492000` is not a 2026 date but `Invalid Date`. */
export function stampReading(value: string, now = new Date()): Reading | null {
  return readingOf(/^\d{9,}$/.test(value) ? new Date(Number(value) * 1000) : new Date(value), now)
}

/** The masthead's label and hover/screen-reader detail for the inventory snapshot. */
export function snapshotFreshness(snapshot: Snapshot, now = new Date()): { label: string; detail: string } {
  if (!snapshot) return {
    label: 'No successful sync yet',
    detail: 'TennoScope has not saved a coherent inventory snapshot.',
  }
  const origin = `Source: ${snapshot.source} · Build: ${snapshot.game_build}`
  const reading = readingOf(new Date(snapshot.observed_at * 1000), now)
  if (!reading) return { label: 'Sync time unavailable', detail: origin }
  return { label: `Synced ${reading.relative}`, detail: `${reading.exact} · ${origin}` }
}
