export type AppRouteName = 'main' | 'overlay'

export function routeForPath(pathname: string): AppRouteName {
  return pathname === '/overlay' ? 'overlay' : 'main'
}
