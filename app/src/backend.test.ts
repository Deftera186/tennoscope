import { beforeEach, describe, expect, it, vi } from 'vitest'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

import { acceptRiskDisclosure, getSetupStatus, refreshInventory } from './backend'

describe('typed Tauri command bridge', () => {
  beforeEach(() => invoke.mockReset())

  it('uses stable setup and refresh command names', async () => {
    invoke.mockResolvedValueOnce({ risk_accepted: false })
    await expect(getSetupStatus()).resolves.toEqual({ risk_accepted: false })
    expect(invoke).toHaveBeenCalledWith('get_setup_status')

    invoke.mockResolvedValueOnce({ risk_accepted: true })
    await acceptRiskDisclosure()
    expect(invoke).toHaveBeenCalledWith('accept_risk_disclosure')

    invoke.mockResolvedValueOnce({ collection: { items: [], total_entries: 0 } })
    await refreshInventory()
    expect(invoke).toHaveBeenCalledWith('refresh_inventory')
  })
})
