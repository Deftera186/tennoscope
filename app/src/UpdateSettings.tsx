import { useEffect, useRef, useState } from 'react'
import { openUrl } from '@tauri-apps/plugin-opener'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { stampReading } from './freshness'
import { dismissedCount, readUpdateAutoCheck, readUpdatePrerelease, writeUpdateAutoCheck, writeUpdatePrerelease } from './settings'
import { checkForUpdatesNow, dismissOffered, downloadUpdate, isMeteredConnection, restartToUpdate, useUpdateNotice, useUpdateStore } from './updateStore'


/** The masthead's dedicated update mark: offered and downloaded states share one
 *  diamond and never borrow assay-state grammar. The sr-only status line stays
 *  mounted so async arrival is always announced. */
export function UpdateMark({ onOpen }: { onOpen: () => void }) {
  const notice = useUpdateNotice()
  return <>
    <span className="sr-only" role="status">{notice ? (notice.downloaded ? `Update ready — ${notice.version}. Open Settings to restart.` : `Update available — ${notice.version}. Open Settings to review it.`) : ''}</span>
    {notice && <button type="button" className="update-mark" onClick={onOpen} aria-label={notice.downloaded ? `Update ready — ${notice.version}. Open Settings to restart.` : `Update available — ${notice.version}. Open Settings to review it.`}>
      <span className="update-dot" aria-hidden="true"/>
      <span className="update-text" aria-hidden="true">{notice.downloaded ? `Update ready — ${notice.version}` : `Update available — ${notice.version}`}</span>
    </button>}
  </>
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
  const [copyNote, setCopyNote] = useState<string | null>(null)
  const [deferred, setDeferred] = useState(false)
  const statusRef = useRef<HTMLParagraphElement>(null)
  const actedRef = useRef(false)
  const busy = store.phase === 'checking' || store.phase === 'downloading'
  const info = store.info
  const updatable = info?.updatable ?? false
  const actionable = (store.phase === 'offered' || store.phase === 'suppressed' || store.phase === 'downloading' || store.phase === 'ready' || store.phase === 'failed') ? store.available : null
  const lastReading = store.lastCheck ? stampReading(new Date(store.lastCheck).toISOString()) : null
  const lastChecked = lastReading ? `Last checked ${lastReading.relative}.` : 'Never checked on this device.'
  const percent = store.total ? Math.round((store.downloaded ?? 0) / store.total * 100) : null
  const milestone = percent === null ? null : Math.floor(percent / 25) * 25
  const metered = isMeteredConnection()
  // User-initiated transitions move focus to the new status so keyboard users
  // are not dropped at body when their button unmounts. Background auto-checks
  // never steal focus.
  useEffect(() => {
    if (actedRef.current) {
      actedRef.current = false
      statusRef.current?.focus()
    }
  }, [store.phase])

  function act(action: () => void): void {
    actedRef.current = true
    action()
  }

  async function copyCommand(command: string) {
    try {
      await writeText(command)
      setCopied(true)
      setCopyNote('Copied — paste it into a terminal.')
    } catch {
      setCopied(false)
      setCopyNote('Copy failed — select the command text by hand.')
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
    <button type="button" className="stamp" onClick={() => act(() => void checkForUpdatesNow())} disabled={busy}>Check now</button>
    <p ref={statusRef} tabIndex={-1} className="band-note capture-status" role="status" aria-live="polite" aria-atomic="true">
      {store.phase === 'checking' && 'Checking for updates…'}
      {store.phase === 'current' && info && `You are on ${info.version} — the latest version. ${lastChecked}`}
      {store.phase === 'idle' && info && `You are on ${info.version}. ${lastChecked}`}
      {store.phase === 'idle' && !info && (store.note ?? 'Update checks are unavailable while the backend is down. Press Check now to try again.')}
      {store.phase === 'failed' && store.note}
      {store.phase === 'offered' && actionable && `${actionable.version} is available — actions below.`}
      {store.phase === 'ready' && actionable && `${actionable.version} is downloaded — restart to finish.`}
      {store.phase === 'suppressed' && actionable && dismissedCount(actionable.version) >= 2
        && `You dismissed ${actionable.version} twice, so automatic reminders stay off until the next version.`}
      {store.phase === 'suppressed' && actionable && dismissedCount(actionable.version) < 2
        && `${actionable.version} is available, actions below.`}
      {store.phase === 'downloading' && actionable && `Downloading ${actionable.version} — progress below.`}
    </p>
    {store.phase === 'downloading' && <>
      <progress className="update-progress" max={store.total ?? undefined} value={store.total == null ? undefined : (store.downloaded ?? undefined)} aria-label={`Downloading ${actionable?.version ?? 'update'}`} aria-valuetext={actionable ? (percent === null ? 'Download starting' : `Download ${percent} percent${store.total ? `, ${((store.downloaded ?? 0) / 1048576).toFixed(1)} of ${(store.total / 1048576).toFixed(1)} megabytes` : ''}`) : undefined}/>
      <p className="band-note" aria-hidden="true">{actionable && (percent === null ? 'Starting…' : `${percent}%${store.total ? ` · ${((store.downloaded ?? 0) / 1048576).toFixed(1)} of ${(store.total / 1048576).toFixed(1)} MB` : ''}`)}</p>
    </>}
    <p className="sr-only" role="status">{store.phase === 'downloading' && milestone !== null && milestone > 0 ? `Download ${milestone}%` : ''}</p>
    {store.phase === 'offered' && actionable && updatable && <div className="update-actions">
      <p className="prose">{actionable.version} is available. Press Download to fetch it, then restart to finish.</p>
      {metered && <p className="prohibition-note">You are on a metered connection. Press Download only when ready.</p>}
      <button type="button" className="stamp" onClick={() => act(() => void downloadUpdate())} disabled={busy}>Download update</button>
      <button type="button" className="stamp" onClick={() => act(() => dismissOffered())}>Not now — remind me in 7 days</button>
      <button type="button" className="stamp" onClick={() => void openUrl(releaseTagUrl(actionable.version))}>Open release page</button>
    </div>}
    {store.phase === 'offered' && actionable && !updatable && <div className="update-actions">
      {info?.kind === 'appimage' && !info.writable
        ? <p className="prose">{actionable.version} is available, but TennoScope cannot replace its own file where it lives. Move the AppImage somewhere writable, or press Open release page to get it by hand.</p>
        : <p className="prose">{actionable.version} is available{info?.manager ? ` through ${info.manager}` : ' from your system package manager'}. Press Open release page to get it.</p>}
      {info?.manager_command && <>
        <p className="band-note" id="update-command-offered">Copy, then run in a terminal:</p>
        <div className="command-chip">
          <input value={info.manager_command} readOnly onFocus={event => event.currentTarget.select()} aria-label="Package manager command" aria-describedby="update-command-offered"/>
          <button type="button" className="stamp" onClick={() => void copyCommand(info.manager_command ?? '')}>{copied ? 'Copied' : 'Copy'}</button>
        </div>
        <p className="sr-only" role="status">{copyNote ?? ''}</p>
      </>}
      <button type="button" className="stamp" onClick={() => void openUrl(releaseTagUrl(actionable.version))}>Open release page</button>
      <button type="button" className="stamp" onClick={() => act(() => dismissOffered())}>Not now — remind me in 7 days</button>
    </div>}
    {store.phase === 'suppressed' && actionable && <div className="update-actions">
      {actionable && dismissedCount(actionable.version) >= 2
        ? <p className="prose">You dismissed {actionable.version} twice, so automatic reminders stay off until the next version. Press Open release page to get it.</p>
        : <p className="prose">{actionable.version} is available. Reminders are paused — press Check now to see it again.</p>}
      <button type="button" className="stamp" onClick={() => void openUrl(releaseTagUrl(actionable.version))}>Open release page</button>
    </div>}
    {store.phase === 'failed' && actionable && updatable && <div className="update-actions">
      {metered && <p className="prohibition-note">You are on a metered connection. Press Retry download only when ready.</p>}
      <button type="button" className="stamp" onClick={() => act(() => void downloadUpdate())} disabled={busy}>Retry download</button>
      <button type="button" className="stamp" onClick={() => void openUrl(releaseTagUrl(actionable.version))}>Open release page</button>
    </div>}
    {store.phase === 'failed' && actionable && !updatable && <div className="update-actions">
      {info?.manager_command && <>
        <p className="band-note" id="update-command-failed">Copy, then run in a terminal:</p>
        <div className="command-chip">
          <input value={info.manager_command} readOnly onFocus={event => event.currentTarget.select()} aria-label="Package manager command" aria-describedby="update-command-failed"/>
          <button type="button" className="stamp" onClick={() => void copyCommand(info.manager_command ?? '')}>{copied ? 'Copied' : 'Copy'}</button>
        </div>
        <p className="sr-only" role="status">{copyNote ?? ''}</p>
      </>}
      <button type="button" className="stamp" onClick={() => void openUrl(releaseTagUrl(actionable.version))}>Open release page</button>
    </div>}
    {store.phase === 'ready' && actionable && !deferred && <div className="update-actions">
      <p className="prose">{actionable.version} is downloaded. Restart TennoScope to finish — your settings stay as they are.{observesGame && ' Restart hides the reward overlay until relaunch.'}</p>
      <button type="button" className="stamp" onClick={() => act(() => void restartToUpdate())}>Restart now</button>
      <button type="button" className="stamp" onClick={() => act(() => setDeferred(true))}>Later</button>
    </div>}
    {store.phase === 'ready' && actionable && deferred && <div className="update-actions">
      <p className="prose">{actionable.version} is downloaded and stays downloaded until you restart. Restart TennoScope to finish — your settings stay as they are.{observesGame && ' Restart hides the reward overlay until relaunch.'}</p>
      <button type="button" className="stamp" onClick={() => act(() => void restartToUpdate())}>Restart now</button>
    </div>}
  </div>
}
