import App from './App'
import RewardOverlay from './RewardOverlay'
import { routeForPath } from './routing'

export function AppRoute({ pathname = window.location.pathname }: { pathname?: string }) {
  return routeForPath(pathname) === 'overlay' ? <RewardOverlay/> : <App/>
}
