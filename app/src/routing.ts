export type AppRouteName = 'main' | 'overlay' | 'kiosk'

export function routeForPath(pathname: string): AppRouteName {
  if (pathname === '/overlay') return 'overlay'
  if (pathname === '/kiosk') return 'kiosk'
  return 'main'
}
