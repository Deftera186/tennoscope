import { WebviewWindow } from '@tauri-apps/api/webviewWindow'

const OVERLAY_LABEL = 'reward-overlay'

export async function showRewardOverlay() {
  const overlay = await WebviewWindow.getByLabel(OVERLAY_LABEL)
  if (!overlay) throw new Error('reward overlay window is unavailable')
  await overlay.show()
}

export async function hideRewardOverlay() {
  const overlay = await WebviewWindow.getByLabel(OVERLAY_LABEL)
  if (!overlay) throw new Error('reward overlay window is unavailable')
  await overlay.hide()
}
