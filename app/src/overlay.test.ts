import { beforeEach, describe, expect, it, vi } from 'vitest'

const coreMock = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => coreMock)

import { hideRewardOverlay, showRewardOverlay } from './overlay'

describe('reward overlay window actions', () => {
  beforeEach(() => coreMock.invoke.mockReset())

  it('routes preview through native overlay configuration', async () => {
    await showRewardOverlay()
    expect(coreMock.invoke).toHaveBeenCalledWith('show_reward_overlay')
  })

  it('routes hide through the native overlay adapter', async () => {
    await hideRewardOverlay()
    expect(coreMock.invoke).toHaveBeenCalledWith('hide_reward_overlay')
  })
})
