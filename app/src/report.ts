import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener'
import { collectReport, collectReportText, type CollectedReport } from './backend'

export const ISSUE_URL = 'https://github.com/Deftera186/tennoscope/issues/new?template=bug_report.yml'

export async function copyReport(): Promise<void> {
  await writeText(await collectReportText())
}

export async function saveReport(): Promise<CollectedReport> {
  const result = await collectReport()
  // The reveal is a convenience -- on a Steam Deck in Game Mode there is no file manager to open.
  // Losing it must not lose the folder path, which is the only thing the player actually needs.
  try {
    await revealItemInDir(result.folder_path)
  } catch {
    // ignored on purpose
  }
  return result
}

export async function openIssue(): Promise<void> {
  await openUrl(ISSUE_URL)
}
