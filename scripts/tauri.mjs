#!/usr/bin/env node
// Normalize the Linux toolchain environment before handing control to Tauri.
//
// `NO_STRIP` avoids linuxdeploy's bundled `strip`, which predates RELR
// relocations and fails on system libraries from current Arch, CachyOS,
// Gentoo, and Fedora Rawhide toolchains. Stripping is only a size optimization.
//
// Native screen capture builds PipeWire bindings through bindgen. Some Linux
// distributions keep libclang in a versioned LLVM slot outside the dynamic
// loader's search path, so the helper resolves that slot through llvm-config.
//
// Both defaults are Linux-only. Explicit caller values always win; release
// scripts and configured development shells therefore retain control.

import { spawn } from 'node:child_process'
import { createRequire } from 'node:module'
import { constants } from 'node:os'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

import { configureTauriEnvironment } from './tauri-env.mjs'

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

const env = configureTauriEnvironment(process.env)

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
