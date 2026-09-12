import { getCurrentWindow } from '@tauri-apps/api/window'

/**
 * The main window runs with the compositor's own decorations off, so window management is this
 * app's job rather than the desktop's. These are the three moves a titlebar offers, plus the
 * maximized readout that names the maximize control by its next action.
 */

export async function minimizeWindow(): Promise<void> {
  await getCurrentWindow().minimize()
}

export async function toggleMaximizeWindow(): Promise<void> {
  await getCurrentWindow().toggleMaximize()
}

export async function closeWindow(): Promise<void> {
  await getCurrentWindow().close()
}

export async function readWindowMaximized(): Promise<boolean> {
  return await getCurrentWindow().isMaximized()
}

/** The state can change from outside the app -- KDE's snap and keyboard shortcuts never touch
 * this code -- so the glyph follows the window rather than the button. */
export async function watchWindowResized(handler: () => void): Promise<() => void> {
  return await getCurrentWindow().onResized(() => handler())
}
