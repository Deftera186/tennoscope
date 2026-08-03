import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { CollectionItem, MarketAccount, ReconciledOrder } from './backend'
import { Orders } from './Orders'

afterEach(() => {
  cleanup()
  // The handlers are shared across every test in this file, so a call recorded by one is visible
  // to the next. A test asserting something was *not* called is the one that catches this, and it
  // catches it as a false failure somewhere unrelated.
  vi.clearAllMocks()
})

function entry(id: string, status: ReconciledOrder['status'], quantity = 1): ReconciledOrder {
  return {
    order: {
      id,
      item_id: `item-${id}`,
      kind: 'sell',
      platinum: 12,
      quantity,
      per_trade: 1,
      visible: true,
      updated_at: '2026-07-30T10:00:00Z',
    },
    name: 'Braton Prime Blueprint',
    status,
  }
}

function account(overrides: Partial<MarketAccount> = {}): MarketAccount {
  return {
    link: 'linked',
    backing: 'keyring',
    orders: [],
    fetched_at: '2026-07-31T12:00:00Z',
    listed_platinum: 0,
    listable: [],
    presence: { status: null, wanted: null, auto: false },
    flagged: 0,
    ...overrides,
  }
}

const handlers = {
  items: [],
  onSell: vi.fn(),
  onPresence: vi.fn().mockResolvedValue(undefined),
  onSignIn: vi.fn().mockResolvedValue(undefined),
  onLinkToken: vi.fn().mockResolvedValue(undefined),
  onSignOut: vi.fn().mockResolvedValue(undefined),
  onRefresh: vi.fn().mockResolvedValue(undefined),
  onRemove: vi.fn().mockResolvedValue(undefined),
  onLowerTo: vi.fn().mockResolvedValue(undefined),
}

describe('the unlinked screen', () => {
  it('explains what linking does before offering to do it', () => {
    render(<Orders account={account({ link: 'unlinked', backing: undefined })} {...handlers} busy={false} error={null} />)

    // The consent statement is on this screen, at the moment the player decides -- not in a
    // settings page they would have to go looking for afterwards. Matched as one statement rather
    // than as two loose words: the screen now names warframe.market in several places, and an
    // assertion that any of them exists would pass on a screen that had lost the consent notice.
    expect(screen.getByText(/optional.*warframe\.market/is)).toBeInTheDocument()
  })

  it('offers both ways in, neither presented as the lesser', () => {
    render(<Orders account={account({ link: 'unlinked', backing: undefined })} {...handlers} busy={false} error={null} />)

    expect(screen.getByLabelText(/email/i)).toBeInTheDocument()
    expect(screen.getByLabelText(/password/i)).toBeInTheDocument()
    expect(screen.getByLabelText(/token/i)).toBeInTheDocument()
  })

  it('says where the token comes from, and what it is worth to whoever holds it', () => {
    render(<Orders account={account({ link: 'unlinked', backing: undefined })} {...handlers} busy={false} error={null} />)

    // The token path is unusable without these steps, and asking the player to go and find them
    // elsewhere is what makes an optional feature the one nobody links. Behind a disclosure
    // because they are read once; asserted here because a collapsed `details` still renders its
    // contents, and a reader can reach them.
    expect(screen.getByText(/cookie named/i)).toBeInTheDocument()
    expect(screen.getByText(/JWT/)).toBeInTheDocument()
    // A token is a credential. The screen that asks for one says so.
    expect(screen.getByText(/like a password/i)).toBeInTheDocument()
  })

  it('never renders the password as readable text', async () => {
    render(<Orders account={account({ link: 'unlinked', backing: undefined })} {...handlers} busy={false} error={null} />)

    const password = screen.getByLabelText(/password/i)
    expect(password).toHaveAttribute('type', 'password')
    await userEvent.type(password, 'not-a-real-password')
    expect(document.body.textContent).not.toContain('not-a-real-password')
  })
})

describe('the linked screen', () => {
  it('states the total and the age of the list', () => {
    render(
      <Orders
        account={account({ orders: [entry('a', { state: 'ok' })], listed_platinum: 24 })}
        {...handlers}
        busy={false}
        error={null}
      />,
    )

    expect(screen.getByText(/24/)).toBeInTheDocument()
  })

  it('shows where the credential is held, because the two are not equally strong', () => {
    render(<Orders account={account({ backing: 'database' })} {...handlers} busy={false} error={null} />)

    expect(screen.getByText(/local database file/i)).toBeInTheDocument()
  })

  it('never shows an email address', () => {
    render(<Orders account={account()} {...handlers} busy={false} error={null} />)

    expect(document.body.textContent).not.toContain('@')
  })

  it('flags an order for something unowned, and offers to remove it', async () => {
    render(
      <Orders
        account={account({ orders: [entry('gone', { state: 'missing' })], flagged: 1 })}
        {...handlers}
        busy={false}
        error={null}
      />,
    )

    expect(screen.getByText(/no longer own this/i)).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: /remove listing/i }))
    expect(handlers.onRemove).toHaveBeenCalledWith('gone')
  })

  /**
   * The row nobody flagged is the one that can be a misclick: it is currently selling something
   * the player still owns, and the only reason to take it down is a change of mind.
   */
  it('asks again before removing a listing nothing is wrong with', async () => {
    render(
      <Orders
        account={account({ orders: [entry('fine', { state: 'ok' })] })}
        {...handlers}
        busy={false}
        error={null}
      />,
    )

    await userEvent.click(screen.getByRole('button', { name: /remove listing/i }))
    expect(handlers.onRemove).not.toHaveBeenCalled()
    await userEvent.click(screen.getByRole('button', { name: /confirm remove/i }))
    expect(handlers.onRemove).toHaveBeenCalledWith('fine')
  })

  it('offers to lower an order that lists more than is owned', async () => {
    render(
      <Orders
        account={account({ orders: [entry('over', { state: 'overshoot', owned: 1 }, 3)], flagged: 1 })}
        {...handlers}
        busy={false}
        error={null}
      />,
    )

    expect(screen.getByText(/own 1 of 3 listed/i)).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: /lower to 1/i }))
    expect(handlers.onLowerTo).toHaveBeenCalledWith('over', 1)
  })

  /// The behaviour that makes the flags trustworthy, asserted at the screen: an order the backend
  /// declined to judge looks like an ordinary row, making no claim. It still offers removal --
  /// every row does -- but nothing on it says anything is wrong.
  it('says nothing about an order it cannot verify', () => {
    render(
      <Orders
        account={account({ orders: [entry('unknown', { state: 'unverifiable' })] })}
        {...handlers}
        busy={false}
        error={null}
      />,
    )

    expect(screen.queryByRole('button', { name: /lower to/i })).not.toBeInTheDocument()
    expect(screen.queryByText(/no longer own/i)).not.toBeInTheDocument()
  })

  it('disables the fixes while one is in flight, so a click is not sent twice', () => {
    render(
      <Orders
        account={account({ orders: [entry('gone', { state: 'missing' })], flagged: 1 })}
        {...handlers}
        busy={true}
        error={null}
      />,
    )

    expect(screen.getByRole('button', { name: /remove listing/i })).toBeDisabled()
  })
})

describe('failures', () => {
  it('shows what went wrong without losing the orders already listed', () => {
    render(
      <Orders
        account={account({ orders: [entry('a', { state: 'ok' })] })}
        {...handlers}
        busy={false}
        error="warframe.market could not be reached"
      />,
    )

    expect(screen.getByText(/could not be reached/i)).toBeInTheDocument()
    expect(screen.getByText(/Braton Prime Blueprint/)).toBeInTheDocument()
  })

  it('asks for a re-link when the credential was refused', () => {
    render(<Orders account={account({ link: 'needs_relink' })} {...handlers} busy={false} error={null} />)

    // The block that owns the recovery, not the band note that summarises it -- both say to sign
    // in again, and only this one carries the forms that let the player do it.
    expect(screen.getByRole('heading', { name: /credential refused/i })).toBeInTheDocument()
    expect(screen.getByText(/refused the stored credential/i)).toBeInTheDocument()
    // The status cell must not still read "Linked" while the credential is being refused.
    expect(screen.queryByText('Linked')).not.toBeInTheDocument()
  })

  it('offers both ways back in from the needs_relink screen, without requiring an unlink first', async () => {
    render(<Orders account={account({ link: 'needs_relink' })} {...handlers} busy={false} error={null} />)

    expect(screen.getByLabelText(/email/i)).toBeInTheDocument()
    expect(screen.getByLabelText(/password/i)).toBeInTheDocument()
    expect(screen.getByLabelText(/token/i)).toBeInTheDocument()

    await userEvent.type(screen.getByLabelText(/email/i), 'someone@example.com')
    await userEvent.type(screen.getByLabelText(/password/i), 'a-password')
    await userEvent.click(screen.getByRole('button', { name: /^sign in$/i }))
    expect(handlers.onSignIn).toHaveBeenCalledWith('someone@example.com', 'a-password')

    await userEvent.type(screen.getByLabelText(/token/i), 'a-token')
    await userEvent.click(screen.getByRole('button', { name: /link with token/i }))
    expect(handlers.onLinkToken).toHaveBeenCalledWith('a-token')
  })
})

const braton: CollectionItem = {
  id: '/Lotus/Weapons/BratonPrime',
  name: 'Braton Prime Blueprint',
  category: 'prime_part',
  quantity: 3,
  mastered: false,
  platinum: 14,
  live: false,
  priceable: true,
}

describe('publishing a listing', () => {
  it('offers nothing when the account can list nothing', () => {
    render(<Orders account={account()} {...handlers} items={[braton]} busy={false} error={null} />)
    expect(screen.queryByRole('button', { name: /new listing/i })).toBeNull()
  })

  it('sends the price and quantity typed, for the item chosen', async () => {
    const user = userEvent.setup()
    render(
      <Orders
        account={account({ listable: [braton.id] })}
        {...handlers}
        items={[braton]}
        busy={false}
        error={null}
      />,
    )
    await user.click(screen.getByRole('button', { name: /new listing/i }))
    await user.type(screen.getByLabelText('Item'), 'brat')
    await user.click(screen.getByRole('button', { name: /braton/i }))
    // The price prefills from the card's own quote, so only the quantity is typed here.
    await user.clear(screen.getByLabelText('Quantity'))
    await user.type(screen.getByLabelText('Quantity'), '2')
    await user.click(screen.getByRole('button', { name: /list for sale/i }))
    expect(handlers.onSell).toHaveBeenCalledWith(braton.id, 14, 2, true)
  })

  it('lists nothing until the query narrows the collection', async () => {
    const user = userEvent.setup()
    render(
      <Orders
        account={account({ listable: [braton.id] })}
        {...handlers}
        items={[braton]}
        busy={false}
        error={null}
      />,
    )
    await user.click(screen.getByRole('button', { name: /new listing/i }))
    // An empty field offering the whole collection is the wall of names this control avoids.
    expect(screen.queryByRole('button', { name: /braton/i })).toBeNull()
    await user.type(screen.getByLabelText('Item'), 'zzz')
    expect(screen.getByText(/nothing sellable here matches/i)).toBeInTheDocument()
  })

  it('refuses to send more than this device holds', async () => {
    const user = userEvent.setup()
    render(
      <Orders
        account={account({ listable: [braton.id] })}
        {...handlers}
        items={[braton]}
        busy={false}
        error={null}
      />,
    )
    await user.click(screen.getByRole('button', { name: /new listing/i }))
    await user.type(screen.getByLabelText('Item'), 'brat')
    await user.click(screen.getByRole('button', { name: /braton/i }))
    await user.clear(screen.getByLabelText('Quantity'))
    await user.type(screen.getByLabelText('Quantity'), '9')
    expect(screen.getByRole('button', { name: /list for sale/i })).toBeDisabled()
  })
})

describe('the market status switch', () => {
  it('asks for a status the server accepts, chosen by hand', async () => {
    const user = userEvent.setup()
    render(<Orders account={account()} {...handlers} busy={false} error={null} />)
    await user.click(screen.getByRole('button', { name: 'In game' }))
    expect(handlers.onPresence).toHaveBeenCalledWith('ingame', false)
  })

  it('spells offline as no status at all', async () => {
    const user = userEvent.setup()
    render(
      <Orders
        account={account({ presence: { status: 'online', wanted: 'online', auto: false } })}
        {...handlers}
        busy={false}
        error={null}
      />,
    )
    await user.click(screen.getByRole('button', { name: 'Offline' }))
    // warframe.market has no settable `offline`: the backend closes the socket instead.
    expect(handlers.onPresence).toHaveBeenCalledWith(null, false)
  })

  it('marks what was asked for, so a press registers before the socket answers', () => {
    render(
      <Orders
        account={account({ presence: { status: null, wanted: 'invisible', auto: false } })}
        {...handlers}
        busy={false}
        error={null}
      />,
    )
    expect(screen.getByRole('button', { name: 'Invisible' })).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByRole('button', { name: 'Online' })).toHaveAttribute('aria-pressed', 'false')
  })

  it('says so while the server has not confirmed the choice', () => {
    render(
      <Orders
        account={account({ presence: { status: null, wanted: 'ingame', auto: false } })}
        {...handlers}
        busy={false}
        error={null}
      />,
    )
    expect(screen.getByRole('status')).toHaveTextContent(/asking warframe\.market/i)
  })

  it('reports the status automatic mode settled on, in the same row', () => {
    render(
      <Orders
        account={account({ presence: { status: 'ingame', wanted: 'ingame', auto: true } })}
        {...handlers}
        busy={false}
        error={null}
      />,
    )
    // Automatic is not a fifth status: the row still names which of the four is in force.
    expect(screen.getByRole('button', { name: 'In game' })).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByRole('checkbox', { name: /follow the game/i })).toBeChecked()
    // Reporting, not offering -- pressing one would be undone by the next poll.
    expect(screen.getByRole('button', { name: 'Online' })).toBeDisabled()
  })

  it('hands the choice back when automatic is switched off', async () => {
    const user = userEvent.setup()
    render(
      <Orders
        account={account({ presence: { status: 'ingame', wanted: 'ingame', auto: true } })}
        {...handlers}
        busy={false}
        error={null}
      />,
    )
    await user.click(screen.getByRole('checkbox', { name: /follow the game/i }))
    // Whatever it had settled on is what is held, so switching off does not change what others see.
    expect(handlers.onPresence).toHaveBeenCalledWith('ingame', false)
  })
})
