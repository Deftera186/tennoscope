import { useSyncExternalStore } from 'react'
import { listen } from '@tauri-apps/api/event'
import { relaunch } from '@tauri-apps/plugin-process'
import {
  getVersionInfo,
  updateCheck,
  updateDownloadAndInstall,
  type CheckResult,
  type UpdateSummary,
  type VersionInfo,
} from './backend'
import {
  clearSnooze,
  dismissedCount,
  dismissVersion,
  readUpdateAutoCheck,
  readUpdateLastCheck,
  readUpdateLastSurfaced,
  readUpdatePrerelease,
  snoozedUntil,
  snoozeVersion,
  writeUpdateLastCheck,
  writeUpdateLastSurfaced,
} from './settings'

export type UpdatePhase =
  | 'loading'
  | 'idle'
  | 'checking'
  | 'current'
  | 'offered'
  | 'suppressed'
  | 'downloading'
  | 'ready'
  | 'failed'

export interface UpdateSnapshot {
  phase: UpdatePhase
  info: VersionInfo | null
  available: UpdateSummary | null
  downloaded: number | null
  total: number | null
  /** Band-note text for the current phase; failures carry register-voice copy. */
  note: string | null
  lastCheck: number | null
}

const DAY_MS = 86_400_000

let snapshot: UpdateSnapshot = {
  phase: 'loading',
  info: null,
  available: null,
  downloaded: null,
  total: null,
  note: null,
  lastCheck: readUpdateLastCheck(),
}
const listeners = new Set<() => void>()
let booted = false
let progressUnlisten: (() => void) | null = null

function set(patch: Partial<UpdateSnapshot>): void {
  snapshot = { ...snapshot, ...patch }
  listeners.forEach(notify => notify())
}

function feed(): string {
  return readUpdatePrerelease() ? 'beta' : 'stable'
}

/** An offered version the UI should stay quiet about on this check. Manual
 *  checks bypass snooze and surface-dedupe (the user is asking again) but
 *  never strike; only a twice-dismissed version stays silent everywhere. */
function suppressed(version: string, date: string | null, manual: boolean): boolean {
  if (dismissedCount(version) >= 2) return true
  if (manual) return false
  const snoozed = snoozedUntil(version)
  if (snoozed && Date.now() < snoozed) return true
  // An expired snooze re-arms the nudge once: the user asked to be reminded.
  if (snoozed) return false
  if (date && date === readUpdateLastSurfaced()) return true
  return false
}

/** Best-effort metered detection: checks always run, downloads warn first. */
export function isMeteredConnection(): boolean {
  try {
    const connection = (navigator as Navigator & {
      connection?: { saveData?: boolean; type?: string }
    }).connection
    return connection?.saveData === true || connection?.type === 'cellular'
  } catch {
    return false
  }
}

let checking = false

async function runCheck(manual: boolean): Promise<void> {
  // A download in flight owns the phase: a concurrent check would wipe its
  // progress and let the stale download resolve over the newer result.
  if (checking || snapshot.phase === 'downloading') return
  checking = true
  try {
    if (!navigator.onLine) {
      set(manual
        ? { phase: 'failed', note: 'Could not check — you are offline. Reconnect, then press Check now.' }
        : { phase: snapshot.info ? snapshot.phase : 'idle' })
      return
    }
    set({ phase: 'checking', note: null, downloaded: null, total: null })
    try {
      const result: CheckResult = await updateCheck(feed())
      const at = Date.now()
      writeUpdateLastCheck(at)
      const update = result.update
      if (!update) {
        set({ phase: 'current', available: null, lastCheck: at, note: null })
        return
      }
      if (suppressed(update.version, update.date, manual)) {
        set({ phase: 'suppressed', available: update, lastCheck: at, note: null })
        return
      }
      // A manual check bypasses snooze and surface-dedupe but never strikes:
      // the record is spent so a later Not-now snoozes fresh. An auto-check
      // keeps the record: dismissing an already-snoozed version accrues the
      // strike, so the re-offer must not clear it first.
      if (manual && snoozedUntil(update.version)) clearSnooze(update.version)
      if (update.date) writeUpdateLastSurfaced(update.date)
      set({ phase: 'offered', available: update, lastCheck: at })
    } catch {
      set({
        phase: 'failed',
        note: 'Could not reach the release feed. Check your connection, then press Check now.',
      })
    }
  } finally {
    checking = false
  }
}
/** Masthead notice: an available update (offered) or a downloaded one (ready).
 *  The mark never borrows assay-state grammar; it keeps its own diamond. */
export function useUpdateNotice(): { version: string; downloaded: boolean } | null {
  const { phase, available } = useUpdateStore()
  if (!available) return null
  if (phase === 'ready') return { version: available.version, downloaded: true }
  if (phase === 'offered') return { version: available.version, downloaded: false }
  return null
}

function subscribe(notify: () => void): () => void {
  listeners.add(notify)
  return () => {
    listeners.delete(notify)
  }
}

export function useUpdateStore(): UpdateSnapshot {
  return useSyncExternalStore(subscribe, () => snapshot, () => snapshot)
}

/** Daily auto-check. Runs once per mount; CI-light by contact, not by timer. */
export function bootUpdateChecks(): void {
  if (booted) return
  booted = true
  void (async () => {
    try {
      const info = await getVersionInfo()
      set({ info, phase: 'idle' })
    } catch {
      set({ phase: 'idle', note: 'Could not read the installed version. Press Check now to try again.' })
      return
    }
    if (!readUpdateAutoCheck()) return
    const last = readUpdateLastCheck()
    if (last && Date.now() - last < DAY_MS) return
    await runCheck(false)
  })()
}

export function checkForUpdatesNow(): Promise<void> {
  return runCheck(true)
}

/** C1: `update_download_and_install` takes (feed, expected_version) and
 *  rejects with `superseded:{v}` when the feed moves under the offer. The
 *  shared wrapper types only the feed, so the call site widens it here and
 *  the backend file itself stays untouched. */
const installUpdate = updateDownloadAndInstall as unknown as (
  feed: string,
  expectedVersion: string,
) => Promise<UpdateSummary>

export async function downloadUpdate(): Promise<void> {
  const { available } = snapshot
  if (!available || snapshot.phase === 'downloading') return
  set({ phase: 'downloading', downloaded: 0, total: null, note: null })
  try {
    progressUnlisten?.()
    try {
      progressUnlisten = await listen<{ downloaded: number; total: number | null }>(
        'update-progress',
        event => {
          set({ downloaded: event.payload.downloaded, total: event.payload.total })
        },
      )
    } catch {
      set({
        phase: 'failed',
        downloaded: null,
        total: null,
        note: 'Could not start the download. Press Check now to try again.',
      })
      return
    }
    try {
      // The offer's feed, not the current pref: a channel toggle after the
      // offer cannot redirect the install to a different version. The
      // expected version rides along so a feed that moved under the offer
      // fails as `superseded:{v}` instead of installing the wrong build.
      const done = await installUpdate(available.feed, available.version)
      set({ phase: 'ready', available: done, downloaded: null, total: null })
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      if (message.startsWith('superseded:')) {
        // The feed moved: leave downloading first (a concurrent check would
        // no-op while it owns the phase), then re-check to re-offer the new
        // version. Never auto-install it.
        set({ phase: 'checking', downloaded: null, total: null, note: null })
        await checkForUpdatesNow()
        return
      }
      set({
        phase: 'failed',
        downloaded: null,
        total: null,
        note: /signature/i.test(message)
          ? 'Update blocked — the signature check failed. Nothing was installed. Try again from Check now, or get this version from the release page.'
          : 'The download stopped before finishing. Press Retry download to try again.',
      })
    }
  } finally {
    progressUnlisten?.()
    progressUnlisten = null
  }
}

/** Test-only reset: isolates the module singleton between cases. */
export function resetUpdateStoreForTests(): void {
  progressUnlisten?.()
  progressUnlisten = null
  listeners.clear()
  booted = false
  checking = false
  snapshot = {
    phase: 'idle',
    info: null,
    available: null,
    downloaded: null,
    total: null,
    note: null,
    lastCheck: null,
  }
}

export function dismissOffered(): void {
  const { available } = snapshot
  if (available) {
    // Honest model: "Not now" snoozes 7 days with no strike. A strike
    // accrues only when dismissing a version that already has a snooze
    // record — the second dismissal after a lapse — then the snooze renews.
    if (snoozedUntil(available.version) !== null) dismissVersion(available.version)
    snoozeVersion(available.version)
  }
  set({ phase: 'idle', available: null, note: null })
}

export function restartToUpdate(): Promise<void> {
  return relaunch()
}
