import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener'
import { collectReport, collectReportText, type CollectedReport } from './backend'

export const ISSUE_URL = 'https://github.com/Deftera186/tennoscope/issues/new?template=bug_report.yml'

export async function copyReport(): Promise<void> {
  await writeText(await collectReportText())
}

export async function saveReport(): Promise<CollectedReport> {
  const result = await collectReport()
  // The reveal is a convenience -- on a Steam Deck in Game Mode there is no file manager to open,
  // and on Linux it is a D-Bus call to org.freedesktop.FileManager1 (or the OpenURI portal as a
  // fallback) that can sit unanswered far longer than a user will wait. The report is already
  // saved by this point, so the button must not stay busy on this best-effort step: fire it and
  // forget it instead of awaiting it.
  void revealItemInDir(result.folder_path).catch(() => {
    // ignored on purpose
  })
  return result
}

export async function openIssue(): Promise<void> {
  await openUrl(ISSUE_URL)
}
