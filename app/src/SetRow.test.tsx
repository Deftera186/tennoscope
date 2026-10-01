import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'
import { SetRow } from './SetRow'

const guandao = [
  { name: 'Blueprint', image: 'blueprint.png', uses: 1, held: 3, this: false },
  { name: 'Blade', image: 'GenericWeaponPrimeBlade.png', uses: 2, held: 1, this: true },
  { name: 'Handle', image: 'GenericWeaponPrimeHandle.png', uses: 1, held: 0, this: false },
]

// Each test renders a fresh row, and Testing Library leaves the previous one in the document, so a
// slot name would otherwise match twice. The repo's convention is the same teardown everywhere.
afterEach(cleanup)

describe('SetRow', () => {
  it('names each slot with what is held and marks the slot this reward fills', () => {
    render(<SetRow parts={guandao} />)
    expect(screen.getByRole('list', { name: 'Set' })).toBeInTheDocument()
    expect(screen.getByRole('listitem', { name: 'Blade, 1 of 2 held, this reward' })).toHaveAttribute('data-this')
    expect(screen.getByRole('listitem', { name: 'Handle, 0 of 1 held' })).not.toHaveAttribute('data-this')
    // Three held against a recipe that takes one is a covered slot, named as such rather than "3 of 1".
    expect(screen.getByRole('listitem', { name: 'Blueprint, held in full' })).toBeInTheDocument()
  })

  it('draws a part used twice as a pair, one copy held and one not', () => {
    render(<SetRow parts={guandao} />)
    const glyphs = screen.getByRole('listitem', { name: /^Blade/ }).querySelectorAll('.set-glyph')
    expect([...glyphs].map(glyph => glyph.getAttribute('data-held'))).toEqual(['true', 'false'])
  })

  it('draws the blueprint with its own glyph, not the lozenge an unbundled part gets', () => {
    render(<SetRow parts={[guandao[0], { name: 'Bronco Prime', image: 'bronco-prime.png', uses: 2, held: 0, this: true }]} />)
    const blueprint = screen.getByRole('listitem', { name: /^Blueprint/ }).querySelector('svg.set-glyph path')
    const lozenges = screen.getByRole('listitem', { name: /^Bronco Prime/ }).querySelectorAll('svg.set-glyph path')
    expect(lozenges).toHaveLength(2)
    expect(blueprint).toHaveAttribute('d')
    expect(blueprint?.getAttribute('d')).not.toBe(lozenges[0].getAttribute('d'))
  })
})
