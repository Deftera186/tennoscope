import { beforeEach, describe, expect, it, vi } from 'vitest'

const windowMock = vi.hoisted(() => ({ getByLabel: vi.fn() }))
vi.mock('@tauri-apps/api/webviewWindow', () => ({ WebviewWindow: windowMock }))

import { hideRewardOverlay, showRewardOverlay } from './overlay'

describe('reward overlay window actions', () => {
  beforeEach(() => windowMock.getByLabel.mockReset())

  it('shows and focuses the configured overlay window', async () => {
    const window = { show: vi.fn(), setFocus: vi.fn(), hide: vi.fn() }
    windowMock.getByLabel.mockResolvedValue(window)
    await showRewardOverlay()
    expect(windowMock.getByLabel).toHaveBeenCalledWith('reward-overlay')
    expect(window.show).toHaveBeenCalledOnce()
    expect(window.setFocus).toHaveBeenCalledOnce()
  })

  it('hides the configured overlay window', async () => {
    const window = { show: vi.fn(), setFocus: vi.fn(), hide: vi.fn() }
    windowMock.getByLabel.mockResolvedValue(window)
    await hideRewardOverlay()
    expect(window.hide).toHaveBeenCalledOnce()
  })
})
