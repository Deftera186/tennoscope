#!/usr/bin/env node
// Wrapper around the Tauri CLI that exists for exactly one reason: linuxdeploy's
// bundled `strip` predates RELR relocations, so on distributions whose toolchain
// emits `.relr.dyn` (Arch, CachyOS, Gentoo, Fedora Rawhide) it fails on every
// bundled system library and takes the whole AppImage build down with an
// unhelpful `failed to run linuxdeploy`.
//
// Stripping is an optional size optimization, so `NO_STRIP` skips it without
// changing the produced binary. Setting it here means the documented
// `pnpm tauri build` works on those distributions instead of failing.
//
// Linux only: `NO_STRIP` is meaningless to the NSIS bundler, and the Windows
// release job runs this same script. An explicit value from the caller always
// wins, which is how scripts/build-linux-bundles.sh keeps control.

import { spawn } from 'node:child_process'
import { createRequire } from 'node:module'
import { constants } from 'node:os'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

// The dependency lives in app/node_modules, not next to this script and not at
// the repository root, so resolve from app/ explicitly rather than from
// import.meta.url or the current working directory.
const appDir = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'app')
const require = createRequire(path.join(appDir, 'package.json'))

let cli
try {
  cli = require.resolve('@tauri-apps/cli/tauri.js')
} catch {
  console.error(
    'scripts/tauri.mjs: could not resolve @tauri-apps/cli. Run `pnpm install --frozen-lockfile` in app/.'
  )
  process.exit(127)
}

const env = { ...process.env }
if (process.platform === 'linux' && env.NO_STRIP === undefined) {
  env.NO_STRIP = 'true'
}

const child = spawn(process.execPath, [cli, ...process.argv.slice(2)], {
  stdio: 'inherit',
  env
})

child.on('error', (err) => {
  console.error(`scripts/tauri.mjs: failed to start the Tauri CLI: ${err.message}`)
  process.exit(1)
})

child.on('exit', (code, signal) => {
  // Report a signal death as a shell would, so CI sees a non-zero status.
  process.exit(signal ? 128 + (constants.signals[signal] ?? 1) : (code ?? 1))
})
