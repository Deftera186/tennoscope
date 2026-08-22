/**
 * What the Orders screen computes, kept apart from what it draws.
 *
 * The interesting rule is that an unverifiable order is not a problem. The backend declines to
 * judge an order it cannot place against a coherent, newer snapshot, and this file has to carry
 * that declination all the way to the screen: a row that said "you no longer own this" on a
 * machine that has never read the game would be an accusation nothing supports, sitting next to a
 * button that deletes a listing the player wanted.
 */
import type { CollectionItem, CredentialBacking, MarketOrder, OrderStatus, ReconciledOrder } from './backend'

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
 * What the quantity repair does, named as the action rather than as "Fix".
 *
 * Removal is not here: every row offers it now, flagged or not, so it is not a repair a status
 * chooses. Only the overshoot has a fix that depends on what the status carries.
 */
export function fixLabel(status: OrderStatus): string | null {
  return status.state === 'overshoot' ? `Lower to ${status.owned}` : null
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
 * What one order contributes to the listed total, and nothing when it contributes nothing.
 *
 * The same predicate the backend sums by, mirrored here so a row can show its own share. A total
 * that no row accounts for reads as broken even when it is right: a screen of orders each priced
 * above zero, headed by a zero, is only explicable if the screen says which rows it counted.
 *
 * `platinum` prices one trade of `per_trade` units, so a bulk listing of 12 at 38p per 6 is asking
 * 76p, not 456p.
 */
export function orderValue(order: MarketOrder): number | null {
  if (!order.visible || order.kind !== 'sell') return null
  return order.platinum * Math.floor(order.quantity / Math.max(order.per_trade, 1))
}

/** Why a row contributes nothing, for the rows that do not. */
export function uncountedReason(order: MarketOrder): string | null {
  if (order.kind !== 'sell') return 'Buy order'
  return order.visible ? null : 'Hidden'
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

/** Whether this item can be listed from here at all: held, and a row the account may publish.
 *
 * Row ids, not paths, because the rows are what a listing names: the backend resolves an unranked
 * stack and a maxed copy to two different listings, and a part-ranked copy to none. Matching the
 * path would offer one of them for all three. */
export function isListable(item: CollectionItem, listable: readonly string[]): boolean {
  return item.quantity > 0 && listable.includes(item.id)
}
