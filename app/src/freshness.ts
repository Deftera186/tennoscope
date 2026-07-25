import type { AppView } from './backend'

type Snapshot = AppView['collection']['snapshot']

function snapshotDate(value: string): Date | null {
  const date = /^\d{9,}$/.test(value) ? new Date(Number(value) * 1000) : new Date(value)
  return Number.isNaN(date.getTime()) ? null : date
}

export function snapshotFreshness(snapshot: Snapshot, now = new Date()): { label: string; detail: string } {
  if (!snapshot) return {
    label: 'No successful sync yet',
    detail: 'TennoScope has not saved a coherent inventory snapshot.',
  }
  const observed = snapshotDate(snapshot.observed_at)
  if (!observed) return {
    label: 'Sync time unavailable',
    detail: `Source: ${snapshot.source} · Build: ${snapshot.game_build}`,
  }
  const elapsedSeconds = Math.max(0, Math.floor((now.getTime() - observed.getTime()) / 1000))
  let relative = 'just now'
  if (elapsedSeconds >= 86_400) relative = `${Math.floor(elapsedSeconds / 86_400)} day${elapsedSeconds < 172_800 ? '' : 's'} ago`
  else if (elapsedSeconds >= 3_600) relative = `${Math.floor(elapsedSeconds / 3_600)} hour${elapsedSeconds < 7_200 ? '' : 's'} ago`
  else if (elapsedSeconds >= 60) relative = `${Math.floor(elapsedSeconds / 60)} minute${elapsedSeconds < 120 ? '' : 's'} ago`
  const exact = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'medium' }).format(observed)
  return {
    label: `Synced ${relative}`,
    detail: `${exact} · Source: ${snapshot.source} · Build: ${snapshot.game_build}`,
  }
}
