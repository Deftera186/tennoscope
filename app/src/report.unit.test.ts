import { afterEach, expect, it, vi } from 'vitest'

const backend = vi.hoisted(() => ({ collectReport: vi.fn(), collectReportText: vi.fn() }))
const opener = vi.hoisted(() => ({ openUrl: vi.fn(), revealItemInDir: vi.fn() }))
const clipboard = vi.hoisted(() => ({ writeText: vi.fn() }))

vi.mock('./backend', () => backend)
vi.mock('@tauri-apps/plugin-opener', () => opener)
vi.mock('@tauri-apps/plugin-clipboard-manager', () => clipboard)

import { saveReport } from './report'

afterEach(() => { vi.clearAllMocks() })

it('reveals the report folder without making reveal failure fatal', async () => {
  const collected = {
    folder_path: '/tmp/reports/2026-08-28-205420',
    report_text: 'diagnostics',
    ee_log_included: true,
  }
  backend.collectReport.mockResolvedValue(collected)
  opener.revealItemInDir.mockRejectedValue(new Error('no file manager'))

  await expect(saveReport()).resolves.toEqual(collected)
  expect(opener.revealItemInDir).toHaveBeenCalledWith(collected.folder_path)
})

it('returns the saved report while revealing its folder is still pending', async () => {
  const collected = {
    folder_path: '/tmp/reports/2026-08-28-205420',
    report_text: 'diagnostics',
    ee_log_included: true,
  }
  backend.collectReport.mockResolvedValue(collected)
  opener.revealItemInDir.mockReturnValue(new Promise<void>(() => {}))

  await expect(saveReport()).resolves.toEqual(collected)
  expect(opener.revealItemInDir).toHaveBeenCalledWith(collected.folder_path)
})
