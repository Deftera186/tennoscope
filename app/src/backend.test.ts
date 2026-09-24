import { beforeEach, describe, expect, it, vi } from 'vitest'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

import {
  authorizeScreenCapture,
  getSetupStatus,
  marketLinkToken,
  marketSignIn,
  marketSignOut,
  marketStatus,
  refreshInventory,
  refreshOrders,
  removeOrder,
  setAccessMode,
  setOrderQuantity,
  updateOrder,
  updateDownloadAndInstall,
} from './backend'

describe('typed Tauri command bridge', () => {
  beforeEach(() => invoke.mockReset())

  it('uses the access-mode setup contract and stable refresh commands', async () => {
    const firstRun = { setup_complete: false, access_mode: null, desktop_capture_action_available: false }
    invoke.mockResolvedValueOnce(firstRun)
    await expect(getSetupStatus()).resolves.toEqual(firstRun)
    expect(invoke).toHaveBeenCalledWith('get_setup_status')

    const configured = { setup_complete: true, access_mode: 'overlay' as const, desktop_capture_action_available: true }
    invoke.mockResolvedValueOnce(configured)
    await expect(setAccessMode('overlay')).resolves.toEqual(configured)
    expect(invoke).toHaveBeenCalledWith('set_access_mode', { accessMode: 'overlay' })

    invoke.mockResolvedValueOnce({ ...configured, desktop_capture_action_available: false })
    await expect(authorizeScreenCapture()).resolves.toEqual({ ...configured, desktop_capture_action_available: false })
    expect(invoke).toHaveBeenCalledWith('authorize_screen_capture')

    invoke.mockResolvedValueOnce({ collection: { items: [], total_entries: 0 } })
    await refreshInventory()
    expect(invoke).toHaveBeenCalledWith('refresh_inventory')
  })


  it('waits out a backend that has not finished starting', async () => {
    invoke
      .mockRejectedValueOnce(new Error('command get_setup_status not found'))
      .mockRejectedValueOnce(new Error('command get_setup_status not found'))
      .mockResolvedValueOnce({ setup_complete: true, access_mode: 'full', desktop_capture_action_available: false })

    await expect(getSetupStatus(5, 0)).resolves.toEqual({ setup_complete: true, access_mode: 'full', desktop_capture_action_available: false })
    expect(invoke).toHaveBeenCalledTimes(3)
  })

  it('gives up once the backend is plainly not coming', async () => {
    const unavailable = () => Promise.reject(new Error('command get_setup_status not found'))
    invoke
      .mockImplementationOnce(unavailable)
      .mockImplementationOnce(unavailable)
      .mockImplementationOnce(unavailable)

    let caught: unknown
    try {
      await getSetupStatus(3, 0)
    } catch (error) {
      caught = error
    }

    expect(caught).toBeInstanceOf(Error)
    expect(invoke).toHaveBeenCalledTimes(3)
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

    invoke.mockResolvedValueOnce({})
    await updateOrder('order-one', 19, 3)
    expect(invoke).toHaveBeenCalledWith('update_order', {
      orderId: 'order-one',
      platinum: 19,
      quantity: 3,
    })
  })
  it('pins the offered version on the install call', async () => {
    invoke.mockResolvedValueOnce({ version: '0.12.0' })
    await updateDownloadAndInstall('stable', '0.12.0')
    expect(invoke).toHaveBeenCalledWith('update_download_and_install', {
      feed: 'stable',
      expectedVersion: '0.12.0',
    })
  })
})
