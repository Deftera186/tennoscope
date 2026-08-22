import { useState } from 'react'
import type { CollectionItem, MarketOrder } from './backend'

/**
 * Publishing or editing one sell listing, from wherever the player is standing.
 *
 * Shared between the collection card and the orders screen for the same reason `LinkForms` is
 * shared between the unlinked and refused screens: two copies of a form that posts to a real
 * account would drift, and the one that drifted would be the one nobody was looking at.
 *
 * Without a `listing` this publishes: the price is prefilled from whatever quote the card already
 * carries and the quantity from one, not from the whole stack -- a form that offers to sell
 * everything by default is a form that eventually does.
 *
 * With a `listing` this edits that order: both fields prefilled from the listing itself, and the
 * save patches the price and the count of the order named. warframe.market allows one sell order
 * per item, so selling more of a partly-listed holding is an edit of the existing listing, not a
 * second listing -- and a create attempted against one would be refused by the market after the
 * request. No visibility choice is offered in edit mode because the save sends none: a checkbox
 * that changed nothing it sent would be a control lying about what it does.
 *
 * The item is named by its whole row id, rank suffix or relic tier included: the row is what the
 * backend resolves the listing's rank, subtype and per-trade size from, and the form asks for
 * none of them because the row already knows.
 */
export type SellHandler = (collectionId: string, platinum: number, quantity: number, visible: boolean) => Promise<void>
export type UpdateHandler = (orderId: string, platinum: number, quantity: number) => Promise<void>

export function SellForm({ item, listing, busy, onSell, onUpdate, onDone }: {
  item: CollectionItem
  listing?: MarketOrder
  busy: boolean
  onSell: SellHandler
  onUpdate: UpdateHandler
  onDone: () => void
}) {
  const [platinum, setPlatinum] = useState(String(listing?.platinum ?? item.platinum ?? 1))
  const [quantity, setQuantity] = useState(String(listing?.quantity ?? 1))
  const [visible, setVisible] = useState(true)

  const price = Number(platinum)
  const count = Number(quantity)
  // The market's own bounds, and this device's: offering to sell more than the collection holds is
  // the mirror of the flag the orders screen exists to raise.
  const valid = Number.isInteger(price) && price >= 1 && price <= 900_000
    && Number.isInteger(count) && count >= 1 && count <= Math.max(item.quantity, 1)

  return <form
    className="sell-form"
    aria-label={listing ? `Edit listing for ${item.name}` : `List ${item.name} for sale`}
    onSubmit={async event => {
      event.preventDefault()
      if (!valid) return
      if (listing) {
        await onUpdate(listing.id, price, count)
      } else {
        await onSell(item.id, price, count, visible)
      }
      onDone()
    }}
  >
    <label className="dial-slot">
      <span>Platinum</span>
      <input type="number" min={1} max={900000} aria-label="Platinum" value={platinum} onChange={event => setPlatinum(event.target.value)} disabled={busy} />
    </label>
    <label className="dial-slot">
      <span>Quantity</span>
      <input type="number" min={1} max={Math.max(item.quantity, 1)} aria-label="Quantity" value={quantity} onChange={event => setQuantity(event.target.value)} disabled={busy} />
    </label>
    {/* Hidden is offered rather than assumed either way: a hidden listing is a real way to hold a
        price ready without showing it, and warframe.market's own default of hidden is not what
        someone pressing "sell" means. Edit mode sends no visibility at all. */}
    {!listing && <label className="sell-visible">
      <input type="checkbox" checked={visible} onChange={event => setVisible(event.target.checked)} disabled={busy} />
      <span>Visible to buyers</span>
    </label>}
    <div className="sell-actions">
      <button type="submit" className="stamp" disabled={busy || !valid}><span>{busy
        ? (listing ? 'Saving…' : 'Listing…')
        : (listing ? 'Save listing' : 'List for sale')}</span></button>
      <button type="button" className="stamp" disabled={busy} onClick={onDone}><span>Cancel</span></button>
    </div>
  </form>
}
