import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import App from './App'

describe('App', () => {
  it('identifies the application as Warframe Helper', () => {
    render(<App />)

    expect(
      screen.getByRole('heading', { name: 'Warframe Helper' }),
    ).toBeInTheDocument()
  })
})
