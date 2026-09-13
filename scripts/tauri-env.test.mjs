import assert from 'node:assert/strict'
import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import test from 'node:test'

import { configureTauriEnvironment } from './tauri-env.mjs'

function writeLlvmConfig(file, libdir) {
  mkdirSync(path.dirname(file), { recursive: true })
  writeFileSync(file, `#!/bin/sh\nprintf '%s\\n' ${JSON.stringify(libdir)}\n`)
  chmodSync(file, 0o755)
}

test('Linux finds a versioned LLVM slot that actually contains libclang', (t) => {
  const fixture = mkdtempSync(path.join(tmpdir(), 'tennoscope-tauri-env-'))
  t.after(() => rmSync(fixture, { recursive: true, force: true }))

  const selectedBin = path.join(fixture, 'bin')
  const llvmRoot = path.join(fixture, 'llvm')
  const selectedLib = path.join(fixture, 'selected-lib')
  const llvm23Lib = path.join(llvmRoot, '23', 'lib64')
  const llvm22Lib = path.join(llvmRoot, '22', 'lib64')

  mkdirSync(selectedLib, { recursive: true })
  mkdirSync(llvm23Lib, { recursive: true })
  mkdirSync(llvm22Lib, { recursive: true })
  writeFileSync(path.join(llvm23Lib, 'libclang-cpp.so.23.1'), '')
  writeFileSync(path.join(llvm22Lib, 'libclang-22.so.1'), '')
  writeLlvmConfig(path.join(selectedBin, 'llvm-config'), selectedLib)
  writeLlvmConfig(path.join(llvmRoot, '23', 'bin', 'llvm-config'), llvm23Lib)
  writeLlvmConfig(path.join(llvmRoot, '22', 'bin', 'llvm-config'), llvm22Lib)

  const env = configureTauriEnvironment(
    { PATH: `${selectedBin}:${process.env.PATH ?? ''}` },
    { platform: 'linux', llvmRoots: [llvmRoot] }
  )

  assert.equal(env.LIBCLANG_PATH, llvm22Lib)
  assert.equal(env.NO_STRIP, 'true')
})

test('an explicit caller environment always wins', () => {
  const source = {
    LIBCLANG_PATH: '/caller/libclang',
    NO_STRIP: 'false'
  }

  const env = configureTauriEnvironment(source, {
    platform: 'linux',
    llvmRoots: ['/does/not/exist']
  })

  assert.equal(env.LIBCLANG_PATH, '/caller/libclang')
  assert.equal(env.NO_STRIP, 'false')
})
