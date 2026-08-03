import { useState } from 'react'
import type { MarketAccount } from './backend'
import { backingLabel, fixLabel, isFlagged, sortOrders, statusLabel } from './orders'
import { snapshotFreshness } from './freshness'

type OrdersProps = {
  account: MarketAccount
  onSignIn: (email: string, password: string) => Promise<void>
  onLinkToken: (token: string) => Promise<void>
  onSignOut: () => Promise<void>
  onRefresh: () => Promise<void>
  onRemove: (orderId: string) => Promise<void>
  onLowerTo: (orderId: string, quantity: number) => Promise<void>
  busy: boolean
  error: string | null
}

/** Same shape `snapshotFreshness` expects, keyed to the order list's own fetch time instead of the
 * collection's. Reusing the one relative-time vocabulary rather than inventing a second. */
function fetchFreshness(fetchedAt: string | undefined, now = new Date()) {
  return snapshotFreshness(fetchedAt ? { observed_at: fetchedAt, game_build: '', source: 'warframe.market' } : null, now)
}

export function Orders({ account, onSignIn, onLinkToken, onSignOut, onRefresh, onRemove, onLowerTo, busy, error }: OrdersProps) {
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

    <div className="assay-band">
      <div className="band-cell orders" data-summary="orders">
        <span className="band-figure">{account.listed_platinum}<span className="metal plat" data-metal="plat" /></span>
        <span className="band-label">Listed value</span>
        <p className="band-note">{freshness.label}</p>
      </div>
      <div className="band-cell backing" data-summary="backing">
        <span className="band-figure">Linked</span>
        <span className="band-label">Status</span>
        <p className="band-note">{backingLabel(account.backing)}</p>
      </div>
    </div>

    {error && <p className="error-banner" role="alert">{error}</p>}

    {needsRelink
      ? <div className="setting">
          <div>
            <h3>Credential refused</h3>
            <p className="prose">warframe.market refused the stored credential. Sign in again, or link again with a fresh token, to keep listings up to date.</p>
          </div>
        </div>
      : null}

    {needsRelink && <LinkForms onSignIn={onSignIn} onLinkToken={onLinkToken} busy={busy} />}

    <div className="register-controls">
      <button type="button" className="stamp" onClick={onRefresh} disabled={busy}>
        <span>{busy ? 'Refreshing…' : 'Refresh orders'}</span>
      </button>
      <button type="button" className="stamp" onClick={onSignOut} disabled={busy}>
        <span>Unlink account</span>
      </button>
    </div>

    {account.orders.length
      ? <ul className="collection-grid" aria-label="Market orders">
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

function OrderRow({ entry, busy, onRemove, onLowerTo }: {
  entry: ReturnType<typeof sortOrders>[number]
  busy: boolean
  onRemove: (orderId: string) => Promise<void>
  onLowerTo: (orderId: string, quantity: number) => Promise<void>
}) {
  const flagged = isFlagged(entry.status)
  const label = statusLabel(entry)
  const fix = fixLabel(entry.status)
  return <article className={`entry${flagged ? ' hallmark doubt' : ''}`} aria-label={entry.name ?? entry.order.item_id}>
    <div className="entry-body">
      <h2 className="entry-name">{entry.name ?? entry.order.item_id}</h2>
      <div className="marks">
        <span className="hallmark owned">{entry.order.platinum}p × {entry.order.quantity}</span>
        {label && <span className="hallmark doubt">{label}</span>}
      </div>
    </div>
    {fix && entry.status.state !== 'ok' && entry.status.state !== 'unverifiable' && <button
      type="button"
      className="stamp"
      disabled={busy}
      onClick={() => entry.status.state === 'missing'
        ? onRemove(entry.order.id)
        : entry.status.state === 'overshoot'
          ? onLowerTo(entry.order.id, entry.status.owned)
          : undefined}
    ><span>{fix}</span></button>}
  </article>
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
      <p className="prose">Uses the market's own, undocumented sign-in route. It may stop working without notice.</p>
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
      <h2>Paste a token</h2>
      <p className="prose">Equally valid: paste a session token obtained directly from the market, without giving your password to this app.</p>
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
