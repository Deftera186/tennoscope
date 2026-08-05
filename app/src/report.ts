import { writeText } from '@tauri-apps/plugin-clipboard-manager'
import { openUrl, revealItemInDir } from '@tauri-apps/plugin-opener'
import { collectReport, collectReportText, type CollectedReport } from './backend'

export const ISSUE_URL = 'https://github.com/Deftera186/tennoscope/issues/new?template=bug_report.yml'

export async function copyReport(): Promise<void> {
  await writeText(await collectReportText())
}

export async function saveReport(): Promise<CollectedReport> {
  const result = await collectReport()
  await revealItemInDir(result.folder_path)
  return result
}

export async function openIssue(): Promise<void> {
  await openUrl(ISSUE_URL)
}
