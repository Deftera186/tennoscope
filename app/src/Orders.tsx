import { useState } from 'react'
import type { CollectionItem, MarketAccount } from './backend'
import { SellForm, isListable, type SellHandler } from './SellForm'
import { backingLabel, fixLabel, isFlagged, orderValue, sortOrders, statusLabel, uncountedReason } from './orders'
import { snapshotFreshness } from './freshness'
import { MetalMark } from './MetalMark'

type OrdersProps = {
  account: MarketAccount
  onSignIn: (email: string, password: string) => Promise<void>
  onLinkToken: (token: string) => Promise<void>
  onSignOut: () => Promise<void>
  onRefresh: () => Promise<void>
  onRemove: (orderId: string) => Promise<void>
  onLowerTo: (orderId: string, quantity: number) => Promise<void>
  onSell: SellHandler
  items: CollectionItem[]
  busy: boolean
  error: string | null
}

/** Same shape `snapshotFreshness` expects, keyed to the order list's own fetch time instead of the
 * collection's. Reusing the one relative-time vocabulary rather than inventing a second. */
function fetchFreshness(fetchedAt: string | undefined, now = new Date()) {
  return snapshotFreshness(fetchedAt ? { observed_at: fetchedAt, game_build: '', source: 'warframe.market' } : null, now)
}

export function Orders({ account, onSignIn, onLinkToken, onSignOut, onRefresh, onRemove, onLowerTo, onSell, items, busy, error }: OrdersProps) {
  if (account.link === 'unlinked') {
    return <UnlinkedPanel onSignIn={onSignIn} onLinkToken={onLinkToken} busy={busy} error={error} />
  }

  const freshness = fetchFreshness(account.fetched_at)
  const needsRelink = account.link === 'needs_relink'

  return <section className="page" aria-labelledby="orders-title">
    <div className="mark-head">
      <h1 id="orders-title" className="mark">Market orders</h1>
      <p className="prose">Sell listings on your linked warframe.market account, checked against this device's inventory.</p>
    </div>

    <div className="assay-band orders-band">
      <div className="band-cell orders" data-summary="orders">
        <span className="band-figure">{account.listed_platinum}<MetalMark metal="plat" alt=" platinum"/></span>
        <span className="band-label">Listed value</span>
        <p className="band-note">Visible sell listings only. {freshness.label}</p>
      </div>
      <div className="band-cell flagged" data-summary="flagged" data-count={account.flagged}>
        <span className="band-figure">{account.flagged}</span>
        <span className="band-label">Need attention</span>
        <p className="band-note">{account.flagged
          ? 'Listed above what this device says you hold'
          : 'Every listing matches the collection'}</p>
      </div>
      <div className="band-cell backing" data-summary="backing">
        {/* A refused credential is not a linked account. Reading "Linked" beside a refusal notice
            is the band contradicting the screen it heads. */}
        <span className="band-figure">{needsRelink ? 'Refused' : 'Linked'}</span>
        <span className="band-label">Status</span>
        <p className="band-note">{needsRelink
          ? 'Sign in again, or link a fresh token'
          : `Credential held in ${backingLabel(account.backing).toLowerCase()}`}</p>
      </div>
    </div>

    {/* Suppressed while the relink block is up: that block already names the refusal and offers
        the way out, and a banner above it saying the same thing twice is noise, not emphasis. */}
    {error && !needsRelink && <p className="error-banner" role="alert">{error}</p>}

    {/* The refusal states its case and the two ways back in follow immediately, as one block: a
        heading whose instruction is answered a screen further down is an instruction nobody
        follows. */}
    {needsRelink && <div className="relink">
      <h2>Credential refused</h2>
      <p className="prose">warframe.market refused the stored credential. The listings below are the last that were fetched; sign in again, or link a fresh token, to bring them up to date.</p>
      <LinkForms onSignIn={onSignIn} onLinkToken={onLinkToken} busy={busy} />
    </div>}

    <div className="register-controls">
      <button type="button" className="stamp" onClick={onRefresh} disabled={busy}>
        <span>{busy ? 'Refreshing…' : 'Refresh orders'}</span>
      </button>
      <button type="button" className="stamp" onClick={onSignOut} disabled={busy}>
        <span>Unlink account</span>
      </button>
    </div>

    {!needsRelink && <NewListing items={items} listable={account.listable} busy={busy} onSell={onSell}/>}

    {account.orders.length
      ? <ul className="docket" aria-label="Market orders">
        {sortOrders(account.orders).map(entry => <li key={entry.order.id}>
          <OrderRow entry={entry} busy={busy} onRemove={onRemove} onLowerTo={onLowerTo} />
        </li>)}
      </ul>
      : <div className="empty-state">
        <span className="void-mark" aria-hidden="true" />
        <h2>No sell orders found</h2>
        <p>Nothing is currently listed on this account.</p>
      </div>}
  </section>
}

/**
 * One order, as a ledger line rather than a collection card. An order has no art to show and its
 * figures are read down a column -- price against price, quantity against quantity -- so the
 * columns are fixed and the fix button sits in a reserved slot that stays empty on the rows that
 * need nothing. A row whose button appeared and vanished would shift the rows beneath it.
 */
function OrderRow({ entry, busy, onRemove, onLowerTo }: {
  entry: ReturnType<typeof sortOrders>[number]
  busy: boolean
  onRemove: (orderId: string) => Promise<void>
  onLowerTo: (orderId: string, quantity: number) => Promise<void>
}) {
  const flagged = isFlagged(entry.status)
  const label = statusLabel(entry)
  const fix = fixLabel(entry.status)
  const { state } = entry.status
  const value = orderValue(entry.order)
  return <article className={`docket-line${flagged ? ' doubt' : ''}`} aria-label={entry.name ?? entry.order.item_id}>
    {/* The claim is struck as a shape as well as a colour: a row is legible as flagged with the
        hue removed, which is what keeps the whole thing readable to anyone who cannot separate
        oxblood from platinum. Ordinary rows keep the slot so the names still line up. */}
    <span className={`line-mark${flagged ? ' struck' : ''}`} aria-hidden="true" />
    <h2 className="line-name">{entry.name ?? entry.order.item_id}</h2>
    <span className="line-figure">
      {entry.order.platinum}<MetalMark metal="plat" alt=" platinum" className="line-metal"/>
      <i>× {entry.order.quantity}</i>
    </span>
    {/* Every row states its own share of the headline figure, or why it has none. The total is a
        sum of these and nothing else, so it can be checked by reading down the column. */}
    <span className="line-value">{value === null
      ? <em>{uncountedReason(entry.order)}</em>
      : <>{value}<MetalMark metal="plat" alt=" platinum" className="line-metal"/></>}</span>
    <span className="line-claim">{label}</span>
    <span className="line-fix">
      {state === 'overshoot' && <button
        type="button"
        className="stamp"
        disabled={busy}
        onClick={() => onLowerTo(entry.order.id, entry.status.owned)}
      ><span>{fix}</span></button>}
      <RemoveControl
        entry={entry}
        busy={busy}
        onRemove={onRemove}
      />
    </span>
  </article>
}

/**
 * Taking a listing down, on any row rather than only the ones carrying a claim.
 *
 * A flagged row's removal is the repair the row is asking for and goes on one press. An ordinary
 * row's is a change of mind about something that is currently selling, and only that one can be a
 * misclick -- so it asks again, in place. A modal for this would be a heavier interruption than
 * the action deserves, and would take the pointer away from the row it belongs to.
 */
function RemoveControl({ entry, busy, onRemove }: {
  entry: ReturnType<typeof sortOrders>[number]
  busy: boolean
  onRemove: (orderId: string) => Promise<void>
}) {
  const [armed, setArmed] = useState(false)
  const asked = entry.status.state === 'missing'
  return <button
    type="button"
    className={`stamp${armed ? ' armed' : ''}`}
    disabled={busy}
    // The row list is rebuilt after every write, so an armed button that lost its row cannot
    // fire on whatever slid into its place.
    onBlur={() => setArmed(false)}
    onClick={() => {
      if (asked || armed) return void onRemove(entry.order.id)
      setArmed(true)
    }}
  ><span>{armed ? 'Confirm remove' : 'Remove listing'}</span></button>
}

/**
 * Publishing from the orders screen, where the player is looking at listings rather than at items.
 *
 * The picker is over the same collection the cards are drawn from and the same listable set the
 * backend authorises against, so an item that cannot be sold from a card cannot be sold from here
 * either -- the refusal lives in one place, on the backend, and neither surface offers what it
 * would refuse.
 */
function NewListing({ items, listable, busy, onSell }: {
  items: CollectionItem[]
  listable: string[]
  busy: boolean
  onSell: SellHandler
}) {
  const [open, setOpen] = useState(false)
  const [chosen, setChosen] = useState('')
  const sellable = items.filter(item => isListable(item, listable))
  if (!sellable.length) return null
  const item = sellable.find(entry => entry.id === chosen)

  if (!open) {
    return <div className="register-controls">
      <button type="button" className="stamp" disabled={busy} onClick={() => setOpen(true)}><span>New listing</span></button>
    </div>
  }
  return <div className="new-listing">
    <label className="dial-slot">
      <span>Item</span>
      <select aria-label="Item" value={chosen} onChange={event => setChosen(event.target.value)} disabled={busy}>
        <option value="">Choose an item…</option>
        {sellable.map(entry => <option key={entry.id} value={entry.id}>{entry.name} (×{entry.quantity})</option>)}
      </select>
    </label>
    {item
      ? <SellForm item={item} busy={busy} onSell={onSell} onDone={() => { setOpen(false); setChosen('') }}/>
      : <button type="button" className="stamp" disabled={busy} onClick={() => setOpen(false)}><span>Cancel</span></button>}
  </div>
}

function UnlinkedPanel({ onSignIn, onLinkToken, busy, error }: {
  onSignIn: (email: string, password: string) => Promise<void>
  onLinkToken: (token: string) => Promise<void>
  busy: boolean
  error: string | null
}) {
  return <section className="page" aria-labelledby="orders-title">
    <div className="mark-head">
      <h1 id="orders-title" className="mark">Market orders</h1>
      <p className="prose">
        Linking a warframe.market account is entirely optional. TennoScope itself has no accounts of its own --
        this connects to warframe.market directly, and order data (listings, quantities, prices) leaves this
        device to check it against warframe.market's servers.
      </p>
    </div>

    {error && <p className="error-banner" role="alert">{error}</p>}

    <LinkForms onSignIn={onSignIn} onLinkToken={onLinkToken} busy={busy} />
  </section>
}

/**
 * The two ways back in, of equal standing wherever they appear -- the unlinked screen, and the
 * needs_relink screen where a refused credential leaves the player with no obvious next click.
 * Extracted so a re-link is not a second, drifting copy of these two forms.
 */
function LinkForms({ onSignIn, onLinkToken, busy }: {
  onSignIn: (email: string, password: string) => Promise<void>
  onLinkToken: (token: string) => Promise<void>
  busy: boolean
}) {
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [token, setToken] = useState('')

  return <div className="clause-pair">
    <article>
      <h2>Sign in</h2>
      <p className="prose">Your warframe.market email and password. They are sent once to warframe.market and never stored on this device.</p>
      <form onSubmit={event => { event.preventDefault(); void onSignIn(email, password) }}>
        <label className="dial-slot">
          <span>Email</span>
          <input type="email" aria-label="Email" value={email} onChange={event => setEmail(event.target.value)} disabled={busy} />
        </label>
        <label className="dial-slot">
          <span>Password</span>
          <input type="password" aria-label="Password" value={password} onChange={event => setPassword(event.target.value)} disabled={busy} />
        </label>
        <button type="submit" className="stamp" disabled={busy}><span>{busy ? 'Signing in…' : 'Sign in'}</span></button>
      </form>
    </article>
    <article>
      <h2>Use a token instead</h2>
      <p className="prose">Sign in on the warframe.market website and paste the token your browser was given. Your password never reaches TennoScope.</p>
      {/* A native disclosure: the steps are needed once, by the people who want this path, and
          they are long enough that leaving them open would bury the field they explain. No
          popover library for something `<details>` already does with a keyboard and a reader. */}
      <details className="token-help">
        <summary>Where do I find the token?</summary>
        <ol>
          <li>Sign in at <b>warframe.market</b> in your browser.</li>
          <li>Open developer tools with <kbd>F12</kbd>.</li>
          <li>Go to <b>Application</b> (Chrome) or <b>Storage</b> (Firefox), then <b>Cookies → warframe.market</b>.</li>
          <li>Copy the value of the cookie named <b>JWT</b>.</li>
        </ol>
        <p>Treat that value like a password: anyone holding it can post and delete orders on your account. TennoScope stores it in your system keyring where one is available, and in its local database file otherwise — the Status panel says which you got.</p>
      </details>
      <form onSubmit={event => { event.preventDefault(); void onLinkToken(token) }}>
        <label className="dial-slot">
          <span>Token</span>
          <input type="password" aria-label="Token" value={token} onChange={event => setToken(event.target.value)} disabled={busy} />
        </label>
        <button type="submit" className="stamp" disabled={busy}><span>{busy ? 'Linking…' : 'Link with token'}</span></button>
      </form>
    </article>
  </div>
}
