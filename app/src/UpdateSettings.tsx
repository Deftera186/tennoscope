import { useState } from 'react'
import { openUrl } from '@tauri-apps/plugin-opener'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { stampReading } from './freshness'
import { readUpdateAutoCheck, readUpdatePrerelease, writeUpdateAutoCheck, writeUpdatePrerelease } from './settings'
import { checkForUpdatesNow, dismissOffered, downloadUpdate, isMeteredConnection, restartToUpdate, useUpdateReady, useUpdateStore } from './updateStore'


/** The masthead's dedicated update mark: rendered only once downloaded, never
 *  borrowing the assay-state grammar (ready there means Watching Warframe). */
export function UpdateMark({ onOpen }: { onOpen: () => void }) {
  const ready = useUpdateReady()
  if (!ready) return null
  return <button type="button" className="update-mark" onClick={onOpen}>
    <span className="update-dot" aria-hidden="true"/>
    <span>Update ready — {ready}</span>
  </button>
}

function releaseTagUrl(version: string): string {
  return `https://github.com/Deftera186/tennoscope/releases/tag/v${version}`
}

/**
 * The Updates row: source of truth for update state. Portable installs get
 * check/download/restart here; system installs get the nudge card instead of
 * an install button that could never work.
 */
export function UpdatesSetting({ observesGame }: { observesGame: boolean }) {
  const store = useUpdateStore()
  const [autoCheck, setAutoCheck] = useState(readUpdateAutoCheck)
  const [prerelease, setPrerelease] = useState(readUpdatePrerelease)
  const [copied, setCopied] = useState(false)
  const [deferred, setDeferred] = useState(false)
  const busy = store.phase === 'checking' || store.phase === 'downloading'
  const info = store.info
  const updatable = info?.updatable ?? false
  const actionable = (store.phase === 'offered' || store.phase === 'suppressed' || store.phase === 'downloading' || store.phase === 'ready' || store.phase === 'failed') ? store.available : null
  const lastReading = store.lastCheck ? stampReading(new Date(store.lastCheck).toISOString()) : null
  const lastChecked = lastReading ? `Last checked ${lastReading.relative}.` : 'Never checked on this device.'
  const percent = store.total ? Math.round((store.downloaded ?? 0) / store.total * 100) : null
  const milestone = percent === null ? null : Math.floor(percent / 25) * 25
  const metered = isMeteredConnection()

  async function copyCommand(command: string) {
    try {
      await writeText(command)
      setCopied(true)
    } catch {
      setCopied(false)
    }
  }

  return <div className="setting">
    <div>
      <h3>Updates</h3>
      <p className="prose">TennoScope checks for new versions daily. Downloads start only when you press Download — nothing installs itself.</p>
      <label className="check-row">
        <input
          type="checkbox"
          checked={autoCheck}
          onChange={() => {
            const next = !autoCheck
            writeUpdateAutoCheck(next)
            setAutoCheck(next)
            if (next) void checkForUpdatesNow()
          }}
        />
        <span>Check for updates daily</span>
      </label>
      <details className="update-channel">
        <summary>Pre-releases</summary>
        <label className="check-row">
          <input
            type="checkbox"
            checked={prerelease}
            onChange={() => {
              const next = !prerelease
              writeUpdatePrerelease(next)
              setPrerelease(next)
              // Either direction changes the feed: re-check so a stale beta
              // offer can never linger after opting out.
              void checkForUpdatesNow()
            }}
          />
          <span>Get beta builds. Betas may break reward reads; switch back anytime.</span>
        </label>
      </details>
    </div>
    <button type="button" className="stamp" onClick={() => void checkForUpdatesNow()} disabled={busy}>Check now</button>
    <p className="band-note capture-status" role="status" aria-live="polite" aria-atomic="true">
      {store.phase === 'checking' && 'Checking for updates…'}
      {store.phase === 'current' && info && `You are on ${info.version} — the latest version. ${lastChecked}`}
      {store.phase === 'idle' && info && `You are on ${info.version}. ${lastChecked}`}
      {store.phase === 'idle' && !info && (store.note ?? 'Update checks are unavailable while the backend is down. Press Check now to try again.')}
      {store.phase === 'failed' && store.note}
      {store.phase === 'suppressed' && actionable && `${actionable.version} is available. Reminders are paused — press Check now to see it again.`}
      {store.phase === 'downloading' && actionable && `Downloading ${actionable.version} — progress below.`}
    </p>
    {store.phase === 'downloading' && <>
      <progress className="update-progress" max={store.total ?? undefined} value={store.downloaded ?? undefined} aria-label={`Downloading ${actionable?.version ?? 'update'}`}/>
      <p className="band-note" aria-hidden="true">{actionable && (percent === null ? 'Starting…' : `${percent}%${store.total ? ` · ${((store.downloaded ?? 0) / 1048576).toFixed(1)} of ${(store.total / 1048576).toFixed(1)} MB` : ''}`)}</p>
      <p className="sr-only" role="status">{milestone !== null ? `Download ${milestone}%` : 'Download started'}</p>
    </>}
    {store.phase === 'offered' && actionable && updatable && <div className="update-actions">
      <p className="prose">{actionable.version} is available. Press Download to fetch it, then restart to finish.</p>
      {metered && <p className="prose">You are on a metered connection. Press Download only when ready.</p>}
      <button type="button" className="stamp" onClick={() => void downloadUpdate()} disabled={busy}>Download update</button>
      <button type="button" className="stamp" onClick={() => dismissOffered()}>Not now — remind me in 7 days</button>
      <button type="button" className="stamp" onClick={() => void openUrl(releaseTagUrl(actionable.version))}>What is new</button>
    </div>}
    {store.phase === 'offered' && actionable && !updatable && <div className="update-actions">
      {info?.kind === 'appimage' && !info.writable
        ? <p className="prose">{actionable.version} is available, but TennoScope cannot replace its own file where it lives. Move the AppImage somewhere writable, or install the new version by hand.</p>
        : <p className="prose">{actionable.version} is available{info?.manager ? ` through ${info.manager}` : ' from your system package manager'}.</p>}
      {info?.manager_command && <>
        <p className="band-note">Copy, then run in a terminal:</p>
        <div className="command-chip">
          <input value={info.manager_command} readOnly onFocus={event => event.currentTarget.select()} aria-label="Package manager command"/>
          <button type="button" className="stamp" onClick={() => void copyCommand(info.manager_command ?? '')}>{copied ? 'Copied' : 'Copy'}</button>
        </div>
      </>}
      <button type="button" className="stamp" onClick={() => void openUrl(releaseTagUrl(actionable.version))}>Open release page</button>
      <button type="button" className="stamp" onClick={() => dismissOffered()}>Not now — remind me in 7 days</button>
    </div>}
    {store.phase === 'failed' && actionable && updatable && <div className="update-actions">
      <button type="button" className="stamp" onClick={() => void downloadUpdate()} disabled={busy}>Retry download</button>
      <button type="button" className="stamp" onClick={() => void openUrl(releaseTagUrl(actionable.version))}>Open release page</button>
    </div>}
    {store.phase === 'ready' && actionable && !deferred && <div className="update-actions">
      <p className="prose">{actionable.version} is downloaded. Restart TennoScope to finish — your settings stay as they are.{observesGame && ' Restart hides the reward overlay until relaunch.'}</p>
      <button type="button" className="stamp" onClick={() => void restartToUpdate()}>Restart now</button>
      <button type="button" className="stamp" onClick={() => setDeferred(true)}>Later</button>
    </div>}
    {store.phase === 'ready' && actionable && deferred && <div className="update-actions">
      <p className="prose">{actionable.version} is downloaded — restart to finish.</p>
      <button type="button" className="stamp" onClick={() => void restartToUpdate()}>Restart now</button>
    </div>}
  </div>
}
