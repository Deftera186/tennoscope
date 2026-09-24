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
