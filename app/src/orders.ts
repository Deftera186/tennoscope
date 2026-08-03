/**
 * What the Orders screen computes, kept apart from what it draws.
 *
 * The interesting rule is that an unverifiable order is not a problem. The backend declines to
 * judge an order it cannot place against a coherent, newer snapshot, and this file has to carry
 * that declination all the way to the screen: a row that said "you no longer own this" on a
 * machine that has never read the game would be an accusation nothing supports, sitting next to a
 * button that deletes a listing the player wanted.
 */
import type { CredentialBacking, MarketOrder, OrderStatus, ReconciledOrder } from './backend'

/** Whether the backend is making a claim about this order, as opposed to declining to. */
export function isFlagged(status: OrderStatus): boolean {
  return status.state === 'missing' || status.state === 'overshoot'
}

/** What the row says is wrong, or null when nothing is claimed. */
export function statusLabel(entry: ReconciledOrder): string | null {
  switch (entry.status.state) {
    case 'missing':
      return 'You no longer own this'
    case 'overshoot':
      return `You own ${entry.status.owned} of ${entry.order.quantity} listed`
    default:
      return null
  }
}

/**
 * What the fix button does, named as the action rather than as "Fix".
 *
 * Both of these change a real listing on a real account, so the button says which one it is
 * before it is pressed.
 */
export function fixLabel(status: OrderStatus): string | null {
  switch (status.state) {
    case 'missing':
      return 'Remove listing'
    case 'overshoot':
      return `Lower to ${status.owned}`
    default:
      return null
  }
}

/**
 * Flagged rows first, then everything else in the order the backend sent.
 *
 * Stable within each group on purpose: the list refreshes after every write, and rows that
 * reshuffled underneath a pointer would move the button being aimed at.
 */
export function sortOrders(orders: readonly ReconciledOrder[]): ReconciledOrder[] {
  return [...orders].sort((left, right) => Number(isFlagged(right.status)) - Number(isFlagged(left.status)))
}

/**
 * The live listing for a collection item, for the badge on its card.
 *
 * Hidden orders and buy orders are not listings anybody can see, so an item with only those is not
 * "listed" and the card says nothing.
 */
export function listedOrderFor(orders: readonly ReconciledOrder[], itemId: string): MarketOrder | null {
  const found = orders.find(
    entry => entry.order.item_id === itemId && entry.order.visible && entry.order.kind === 'sell',
  )
  return found?.order ?? null
}

/** Where the credential lives. Stated plainly, because a keyring and a file are not the same. */
export function backingLabel(backing: CredentialBacking | undefined): string {
  if (backing === 'keyring') return 'System keyring'
  return backing === 'database' ? 'Local database file' : 'Not stored'
}
