import { invoke } from '@tauri-apps/api/core'

export async function showRewardOverlay() {
  await invoke('show_reward_overlay')
}

export async function hideRewardOverlay() {
  await invoke('hide_reward_overlay')
}
