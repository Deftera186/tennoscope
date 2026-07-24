import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import App from './App'

describe('App', () => {
  it('renders the minimal Warframe Helper foundation shell', () => {
    render(<App />)

    expect(
      screen.getByRole('heading', { name: 'Warframe Helper' }),
    ).toBeInTheDocument()
    expect(screen.getByText('Foundation ready')).toBeInTheDocument()
    expect(
      screen.getByText('A local-first companion for your Warframe sessions.'),
    ).toBeInTheDocument()
  })
})
