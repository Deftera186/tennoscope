import { useState } from 'react'
import type { CollectionItem } from './backend'

/**
 * Publishing one sell listing, from wherever the player is standing.
 *
 * Shared between the collection card and the orders screen for the same reason `LinkForms` is
 * shared between the unlinked and refused screens: two copies of a form that posts to a real
 * account would drift, and the one that drifted would be the one nobody was looking at.
 *
 * The price is prefilled from whatever quote the card already carries and the quantity from one,
 * not from the whole stack -- a form that offers to sell everything by default is a form that
 * eventually does.
 */
export type SellHandler = (catalogPath: string, platinum: number, quantity: number, visible: boolean) => Promise<void>

export function SellForm({ item, busy, onSell, onDone }: {
  item: CollectionItem
  busy: boolean
  onSell: SellHandler
  onDone: () => void
}) {
  const [platinum, setPlatinum] = useState(String(item.platinum ?? 1))
  const [quantity, setQuantity] = useState('1')
  const [visible, setVisible] = useState(true)

  const price = Number(platinum)
  const count = Number(quantity)
  // The market's own bounds, and this device's: offering to sell more than the collection holds is
  // the mirror of the flag the orders screen exists to raise.
  const valid = Number.isInteger(price) && price >= 1 && price <= 900_000
    && Number.isInteger(count) && count >= 1 && count <= Math.max(item.quantity, 1)

  return <form
    className="sell-form"
    aria-label={`List ${item.name} for sale`}
    onSubmit={async event => {
      event.preventDefault()
      if (!valid) return
      await onSell(catalogPath(item.id), price, count, visible)
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
        someone pressing "sell" means. */}
    <label className="sell-visible">
      <input type="checkbox" checked={visible} onChange={event => setVisible(event.target.checked)} disabled={busy} />
      <span>Visible to buyers</span>
    </label>
    <div className="sell-actions">
      <button type="submit" className="stamp" disabled={busy || !valid}><span>{busy ? 'Listing…' : 'List for sale'}</span></button>
      <button type="button" className="stamp" disabled={busy} onClick={onDone}><span>Cancel</span></button>
    </div>
  </form>
}

/** The collection id without the rank suffix the market never sees. */
export function catalogPath(id: string): string {
  return id.split('#')[0]
}

/** Whether this item can be listed from here at all: held, and on the account's listable set. */
export function isListable(item: CollectionItem, listable: readonly string[]): boolean {
  return item.quantity > 0 && listable.includes(catalogPath(item.id))
}
