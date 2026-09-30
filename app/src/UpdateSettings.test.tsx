import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const backend = vi.hoisted(() => ({
  getVersionInfo: vi.fn(),
  updateCheck: vi.fn(),
  updateDownloadAndInstall: vi.fn(),
}))
vi.mock('./backend', () => backend)
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn(() => Promise.resolve()) }))
vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({ writeText: vi.fn(() => Promise.resolve()) }))
vi.mock('@tauri-apps/plugin-process', () => ({ relaunch: vi.fn(() => Promise.resolve()) }))

import { UpdatesSetting } from './UpdateSettings'
import { bootUpdateChecks, resetUpdateStoreForTests } from './updateStore'

const portable = {
  version: '0.11.0',
  channel: 'stable',
  kind: 'appimage',
  writable: true,
  updatable: true,
  manager: null,
  manager_command: null,
}
const system = {
  ...portable,
  kind: 'system_linux',
  writable: false,
  updatable: false,
  manager: 'Gentoo (deftera overlay)',
  manager_command: 'sudo emerge --ask --update games-util/tennoscope-bin',
}
const update = {
  version: '0.12.0',
  current_version: '0.11.0',
  notes: null,
  date: '2026-09-22T00:00:00Z',
  feed: 'stable',
}

describe('UpdatesSetting', () => {
  afterEach(() => cleanup())
  beforeEach(() => {
    vi.clearAllMocks()
    localStorage.clear()
    resetUpdateStoreForTests()
  })

  it('offers Download on portable installs, never on system ones', async () => {
    backend.getVersionInfo.mockResolvedValue(portable)
    backend.updateCheck.mockResolvedValue({ kind: 'appimage', updatable: true, update })
    render(<UpdatesSetting observesGame={false} />)
    bootUpdateChecks()
    expect(await screen.findByRole('button', { name: 'Download update' })).toBeInTheDocument()
  })

  // An automatic check with no network says it could not check. It must not also throw away an
  // offer that was already standing: the masthead mark, the Download control and Not-now all hang
  // off that phase, and an offline check is no evidence the offer is stale.
  it('keeps a standing offer when the automatic check finds no network', async () => {
    const online = navigator.onLine
    // A stale last-check is what lets the automatic check reach runCheck at all.
    localStorage.setItem('tennoscope.update-last-check', new Date(0).toISOString())
    localStorage.setItem('tennoscope.update-offered', JSON.stringify(update))
    Object.defineProperty(navigator, 'onLine', { value: false, configurable: true })
    try {
      backend.getVersionInfo.mockResolvedValue(portable)
      backend.updateCheck.mockResolvedValue({ kind: 'appimage', updatable: true, update })
      render(<UpdatesSetting observesGame={false} />)
      bootUpdateChecks()
      expect(await screen.findByRole('button', { name: 'Download update' })).toBeInTheDocument()
      expect(backend.updateCheck).not.toHaveBeenCalled()
      // Still an offer, not a download failure, and the row says out loud that the check did not
      // run. Silently keeping the offer is the same defect as silently losing it.
      expect(screen.getByRole('button', { name: 'Download update' })).toBeInTheDocument()
      expect(screen.queryByRole('button', { name: 'Retry download' })).not.toBeInTheDocument()
      await waitFor(() => expect(screen.getAllByRole('status').some(
        node => node.textContent?.match(/is available.*offline/i))).toBe(true))
    } finally {
      Object.defineProperty(navigator, 'onLine', { value: online, configurable: true })
    }
  })


    // A version refused twice stays refused. The online path deliberately withholds the download
    // control for it, so the offline path must not offer it as a failed download either.
    it('keeps a twice-dismissed version dismissed when offline', async () => {
      const online = navigator.onLine
      localStorage.setItem('tennoscope.update-dismissed', JSON.stringify({ [update.version]: 2 }))
      backend.getVersionInfo.mockResolvedValue(portable)
      backend.updateCheck.mockResolvedValue({ kind: 'appimage', updatable: true, update })
      try {
        render(<UpdatesSetting observesGame={false} />)
        bootUpdateChecks()
        await screen.findByRole('button', { name: 'Check now' })
        Object.defineProperty(navigator, 'onLine', { value: false, configurable: true })
        await userEvent.click(screen.getByRole('button', { name: 'Check now' }))
        await waitFor(() => expect(screen.getAllByRole('status').some(
          node => node.textContent?.match(/offline/i))).toBe(true))
        expect(screen.queryByRole('button', { name: 'Download update' })).not.toBeInTheDocument()
        expect(screen.queryByRole('button', { name: 'Retry download' })).not.toBeInTheDocument()
      } finally {
        Object.defineProperty(navigator, 'onLine', { value: online, configurable: true })
      }
    })

  // The other half of the same branch: with nothing to keep, the row must admit the check did not
  // happen rather than reporting a last-checked time for one that never ran.
  it('says the check did not run when offline with nothing standing', async () => {
    const online = navigator.onLine
    Object.defineProperty(navigator, 'onLine', { value: false, configurable: true })
    try {
      backend.getVersionInfo.mockResolvedValue(portable)
      render(<UpdatesSetting observesGame={false} />)
      bootUpdateChecks()
      await waitFor(() => expect(screen.getAllByRole('status').some(
        node => node.textContent?.match(/offline/i))).toBe(true))
      expect(backend.updateCheck).not.toHaveBeenCalled()
    } finally {
      Object.defineProperty(navigator, 'onLine', { value: online, configurable: true })
    }
  })

  it('renders the nudge card with a copyable command on system installs', async () => {
    backend.getVersionInfo.mockResolvedValue(system)
    backend.updateCheck.mockResolvedValue({ kind: 'system_linux', updatable: false, update })
    render(<UpdatesSetting observesGame={false} />)
    bootUpdateChecks()
    expect(await screen.findByRole('button', { name: 'Open release page' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Download update' })).not.toBeInTheDocument()
    expect(screen.getByLabelText('Package manager command')).toHaveValue(system.manager_command)
  })

  it('offers Retry download after a failed download, keeping Check now', async () => {
    backend.getVersionInfo.mockResolvedValue(portable)
    backend.updateCheck.mockResolvedValue({ kind: 'appimage', updatable: true, update })
    backend.updateDownloadAndInstall.mockRejectedValueOnce(new Error('connection reset'))
    render(<UpdatesSetting observesGame={false} />)
    bootUpdateChecks()
    await userEvent.click(await screen.findByRole('button', { name: 'Download update' }))
    expect(await screen.findByRole('button', { name: 'Retry download' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Check now' })).toBeInTheDocument()
    await waitFor(() => expect(screen.getAllByRole('status').some(node => node.textContent?.match(/stopped before finishing/))).toBe(true))
  })

  it('surfaces the offer when version info never loaded', async () => {
    backend.getVersionInfo.mockRejectedValue(new Error('down'))
    backend.updateCheck.mockResolvedValue({ kind: 'unknown', updatable: false, update })
    render(<UpdatesSetting observesGame={false} />)
    bootUpdateChecks()
    // Boot bails without version info; a manual check still surfaces the offer.
    await userEvent.click(screen.getByRole('button', { name: 'Check now' }))
    // Generic nudge: no manager names, no download — but never a swallowed update.
    expect(await screen.findByRole('button', { name: 'Open release page' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Download update' })).not.toBeInTheDocument()
  })

  it('re-checks the stable feed when pre-releases are turned off', async () => {
    localStorage.setItem('tennoscope.update-prerelease', 'true')
    backend.getVersionInfo.mockResolvedValue(portable)
    backend.updateCheck.mockResolvedValue({ kind: 'appimage', updatable: true, update: { ...update, feed: 'beta' } })
    render(<UpdatesSetting observesGame={false} />)
    bootUpdateChecks()
    expect(await screen.findByRole('button', { name: 'Download update' })).toBeInTheDocument()
    expect(backend.updateCheck).toHaveBeenLastCalledWith('beta')
    await userEvent.click(screen.getAllByRole('checkbox')[1])
    await waitFor(() => expect(backend.updateCheck).toHaveBeenLastCalledWith('stable'))
  })

  it('re-offers an undismissed version with the same pub_date instead of suppressing', async () => {
    // The masthead must stay after reopen unless dismissed: an already-surfaced
    // date alone must not silence an undismissed offer.
    localStorage.setItem('tennoscope.update-last-surfaced', update.date)
    backend.getVersionInfo.mockResolvedValue(portable)
    backend.updateCheck.mockResolvedValue({ kind: 'appimage', updatable: true, update })
    render(<UpdatesSetting observesGame={false} />)
    bootUpdateChecks()
    expect(await screen.findByRole('button', { name: 'Download update' })).toBeInTheDocument()
  })

  it('restores a persisted offer on boot so the masthead stays after reopen', async () => {
    // Daily throttle skips the network check; the stored offer keeps the UI.
    localStorage.setItem('tennoscope.update-last-check', new Date().toISOString())
    localStorage.setItem('tennoscope.update-offered', JSON.stringify(update))
    backend.getVersionInfo.mockResolvedValue(portable)
    backend.updateCheck.mockResolvedValue({ kind: 'appimage', updatable: true, update })
    render(<UpdatesSetting observesGame={false} />)
    bootUpdateChecks()
    expect(await screen.findByRole('button', { name: 'Download update' })).toBeInTheDocument()
    expect(backend.updateCheck).not.toHaveBeenCalled()
  })

  it('exposes the Updates section as a deep-link target for the masthead mark', async () => {
    backend.getVersionInfo.mockResolvedValue(portable)
    backend.updateCheck.mockResolvedValue({ kind: 'appimage', updatable: true, update })
    const { container } = render(<UpdatesSetting observesGame={false} />)
    bootUpdateChecks()
    await screen.findByRole('button', { name: 'Download update' })
    expect(container.querySelector('#updates-setting')).not.toBeNull()
  })

  it('pauses reminders for twice-dismissed versions instead of calling them latest', async () => {
    localStorage.setItem('tennoscope.update-dismissed', JSON.stringify({ '0.12.0': 2 }))
    backend.getVersionInfo.mockResolvedValue(portable)
    backend.updateCheck.mockResolvedValue({ kind: 'appimage', updatable: true, update })
    render(<UpdatesSetting observesGame={false} />)
    bootUpdateChecks()
    expect(await screen.findAllByText(/dismissed 0\.12\.0 twice/)).toHaveLength(2)
    expect(screen.queryByText(/the latest version/)).not.toBeInTheDocument()
  })
})
