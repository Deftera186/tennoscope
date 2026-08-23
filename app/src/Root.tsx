import App from './App'
import KioskOverlay from './KioskOverlay'
import RewardOverlay from './RewardOverlay'
import { routeForPath } from './routing'

export function AppRoute({ pathname = window.location.pathname }: { pathname?: string }) {
  const route = routeForPath(pathname)
  return route === 'overlay' ? <RewardOverlay/> : route === 'kiosk' ? <KioskOverlay/> : <App/>
}
