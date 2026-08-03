import { beforeEach, describe, expect, it, vi } from 'vitest'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

import {
  acceptRiskDisclosure,
  getSetupStatus,
  marketLinkToken,
  marketSignIn,
  marketSignOut,
  marketStatus,
  refreshInventory,
  refreshOrders,
  removeOrder,
  setOrderQuantity,
} from './backend'

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

  it('uses stable market account command names', async () => {
    invoke.mockResolvedValueOnce({})
    await marketStatus()
    expect(invoke).toHaveBeenCalledWith('market_status')

    invoke.mockResolvedValueOnce({})
    await marketSignIn('player@example.invalid', 'not-a-real-password')
    expect(invoke).toHaveBeenCalledWith('market_sign_in', {
      email: 'player@example.invalid',
      password: 'not-a-real-password',
    })

    invoke.mockResolvedValueOnce({})
    await marketLinkToken('fake-token')
    expect(invoke).toHaveBeenCalledWith('market_link_token', { token: 'fake-token' })

    invoke.mockResolvedValueOnce({})
    await marketSignOut()
    expect(invoke).toHaveBeenCalledWith('market_sign_out')

    invoke.mockResolvedValueOnce({})
    await refreshOrders()
    expect(invoke).toHaveBeenCalledWith('refresh_orders')

    invoke.mockResolvedValueOnce({})
    await removeOrder('order-one')
    expect(invoke).toHaveBeenCalledWith('remove_order', { orderId: 'order-one' })

    invoke.mockResolvedValueOnce({})
    await setOrderQuantity('order-one')
    expect(invoke).toHaveBeenCalledWith('set_order_quantity', { orderId: 'order-one' })
  })
})
