import { useState } from 'react'
import type { CollectionItem, MarketAccount, Presence } from './backend'
import { SellForm, type SellHandler } from './SellForm'
import { backingLabel, fixLabel, isFlagged, isListable, orderValue, sortOrders, statusLabel, uncountedReason } from './orders'
import { snapshotFreshness } from './freshness'
import { MetalMark } from './MetalMark'

type OrdersViewProps = {
  account: MarketAccount
  onSignIn: (email: string, password: string) => Promise<void>
  onLinkToken: (token: string) => Promise<void>
  onSignOut: () => Promise<void>
  onRefresh: () => Promise<void>
  onRemove: (orderId: string) => Promise<void>
  onLowerTo: (orderId: string, quantity: number) => Promise<void>
  onSell: SellHandler
  onPresence: (status: Presence | null, auto: boolean) => Promise<void>
  items: CollectionItem[]
  busy: boolean
  error: string | null
}

/** Same shape `snapshotFreshness` expects, keyed to the order list's own fetch time instead of the
 * collection's. Reusing the one relative-time vocabulary rather than inventing a second. */
function fetchFreshness(fetchedAt: string | undefined, now = new Date()) {
  return snapshotFreshness(fetchedAt ? { observed_at: fetchedAt, game_build: '', source: 'warframe.market' } : null, now)
}

export function OrdersView({ account, onSignIn, onLinkToken, onSignOut, onRefresh, onRemove, onLowerTo, onSell, onPresence, items, busy, error }: OrdersViewProps) {
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

    {!needsRelink && <PresenceSwitch presence={account.presence} busy={busy} onPresence={onPresence}/>}

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
  const value = orderValue(entry.order)
  const overshoot = entry.status.state === 'overshoot' ? entry.status.owned : null
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
      {/* The count is read out here rather than inside the handler: a narrowing on a property
          does not survive into a closure, so `entry.status.owned` there is not the overshoot's. */}
      {overshoot !== null && <button
        type="button"
        className="stamp"
        disabled={busy}
        onClick={() => onLowerTo(entry.order.id, overshoot)}
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
 * What warframe.market shows this account as.
 *
 * Four choices, three of which are values the server accepts. Offline is the fourth and is not one
 * of them: warframe.market has no settable `offline`, and going offline means closing the socket,
 * so the control says the word the player means and the backend spells it as a disconnection.
 *
 * Automatic is not a fifth choice beside them. It is how the choice is *made*, so it sits on its
 * own line as a toggle, and the status it settles on is marked in the same row a hand-picked one
 * would be -- the player still reads their status in one place either way.
 *
 * The row marks what was asked for, so a press registers on the press. What the server has
 * actually committed is a separate claim, made in the note below: the socket takes a moment to
 * answer, and a switch that stayed blank across that moment read as broken.
 */
function PresenceSwitch({ presence, busy, onPresence }: {
  presence: MarketAccount['presence']
  busy: boolean
  onPresence: (status: Presence | null, auto: boolean) => Promise<void>
}) {
  const choices: { value: Presence | null; label: string }[] = [
    { value: null, label: 'Offline' },
    { value: 'online', label: 'Online' },
    { value: 'ingame', label: 'In game' },
    { value: 'invisible', label: 'Invisible' },
  ]
  const wanted = presence.wanted ?? null
  const settled = presence.status === wanted
  return <div className="presence">
    <div className="presence-head">
      <span className="presence-label" id="presence-label">Market status</span>
      {/* A real checkbox: it is a mode with two states, a reader announces it as one without
          being told to, and the space bar works on it for free. */}
      <label className="presence-auto">
        <input
          type="checkbox"
          checked={presence.auto}
          disabled={busy}
          onChange={event => onPresence(event.target.checked ? null : wanted, event.target.checked)}
        />
        <span>Follow the game</span>
      </label>
    </div>
    <div className="presence-choices" role="group" aria-labelledby="presence-label" data-auto={presence.auto || undefined}>
      {choices.map(choice => <button
        key={choice.label}
        type="button"
        className={`stamp${wanted === choice.value ? ' struck' : ''}`}
        aria-pressed={wanted === choice.value}
        // In automatic mode the row reports rather than accepts: pressing one would silently be
        // overridden by the next poll, and a control that undoes itself is worse than a disabled one.
        disabled={busy || presence.auto}
        onClick={() => onPresence(choice.value, false)}
      ><span>{choice.label}</span></button>)}
    </div>
    <p className="presence-note" role="status">{
      !settled ? 'Asking warframe.market…'
        : presence.auto ? `Following this device's Warframe process. Others see you as ${statusWord(wanted)}.`
          : wanted ? 'Held for as long as TennoScope is running.'
            : 'Nothing is announced to warframe.market while offline.'
    }</p>
  </div>
}

/** The status as it is said aloud in a sentence, rather than as the wire spells it. */
function statusWord(status: Presence | null): string {
  return status === 'ingame' ? 'in game' : status ?? 'offline'
}

/**
 * Publishing from the orders screen, where the player is looking at listings rather than at items.
 *
 * The picker is over the same collection the cards are drawn from and the same listable set the
 * backend authorises against, so an item that cannot be sold from a card cannot be sold from here
 * either -- the refusal lives in one place, on the backend, and neither surface offers what it
 * would refuse.
 *
 * Typed rather than picked from a list. A collection runs to a couple of thousand items, and no
 * one scrolls a list that long to find the one they already have a name for -- so the field takes
 * the name and the register answers with the few that match.
 */
function NewListing({ items, listable, busy, onSell }: {
  items: CollectionItem[]
  listable: string[]
  busy: boolean
  onSell: SellHandler
}) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [chosen, setChosen] = useState('')
  const sellable = items.filter(item => isListable(item, listable))
  if (!sellable.length) return null
  const item = sellable.find(entry => entry.id === chosen)

  function close() {
    setOpen(false)
    setQuery('')
    setChosen('')
  }

  if (!open) {
    return <div className="register-controls">
      <button type="button" className="stamp" disabled={busy} onClick={() => setOpen(true)}><span>New listing</span></button>
    </div>
  }

  const needle = query.trim().toLowerCase()
  // Shown only once the query narrows things: the whole collection listed under an empty field is
  // the wall of names this control exists to avoid. The cap is on what is drawn, and the count
  // below says how many were left out, so a too-broad query reads as too broad rather than as all
  // there is.
  const matches = needle ? sellable.filter(entry => entry.name.toLowerCase().includes(needle)) : []
  const shown = matches.slice(0, 8)

  return <div className="new-listing">
    <div className="new-listing-head">
      <h2>New listing</h2>
      <button type="button" className="stamp" disabled={busy} onClick={close}><span>Cancel</span></button>
    </div>
    {item
      ? <>
        <p className="new-listing-chosen">
          <b>{item.name}</b> <i>· {item.quantity} held</i>
          <button type="button" className="link-button" disabled={busy} onClick={() => setChosen('')}>Choose another</button>
        </p>
        <SellForm item={item} busy={busy} onSell={onSell} onDone={close}/>
      </>
      : <>
        <label className="dial-slot">
          <span>Item</span>
          <input
            type="search"
            aria-label="Item"
            placeholder="Type part of a name…"
            value={query}
            autoFocus
            onChange={event => setQuery(event.target.value)}
            disabled={busy}
          />
        </label>
        {needle && (shown.length
          ? <ul className="pick-list">
            {shown.map(entry => <li key={entry.id}>
              <button type="button" disabled={busy} onClick={() => setChosen(entry.id)}>
                <span className="pick-name">{entry.name}</span>
                <span className="pick-held">{entry.quantity} held</span>
              </button>
            </li>)}
          </ul>
          : <p className="pick-empty">Nothing sellable here matches that. Sets, part-ranked copies and star-set Ayatan sculptures cannot be listed from TennoScope.</p>)}
        {matches.length > shown.length && <p className="pick-more">{matches.length - shown.length} more match — keep typing to narrow it.</p>}
      </>}
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
