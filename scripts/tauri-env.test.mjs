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

function writeClangDriver(file, resourceDir) {
  mkdirSync(path.dirname(file), { recursive: true })
  writeFileSync(file, `#!/bin/sh\nprintf '%s\\n' ${JSON.stringify(resourceDir)}\n`)
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

test('Linux derives the bindgen resource dir from the same LLVM slot', (t) => {
  const fixture = mkdtempSync(path.join(tmpdir(), 'tennoscope-tauri-env-'))
  t.after(() => rmSync(fixture, { recursive: true, force: true }))

  const llvmRoot = path.join(fixture, 'llvm')
  const llvm99Lib = path.join(llvmRoot, '99', 'lib64')
  const resourceDir = path.join(fixture, 'res99')
  mkdirSync(resourceDir, { recursive: true })
  writeClangDriver(
    path.join(llvmRoot, '99', 'bin', 'clang'),
    path.join(llvmRoot, '99', 'bin', '..', '..', '..', 'res99')
  )

  const env = configureTauriEnvironment(
    { LIBCLANG_PATH: llvm99Lib },
    { platform: 'linux', llvmRoots: [llvmRoot] }
  )

  assert.equal(env.BINDGEN_EXTRA_CLANG_ARGS, `-resource-dir=${resourceDir}`)
})

test('an explicit bindgen flag always wins', () => {
  const keys = [
    'BINDGEN_EXTRA_CLANG_ARGS',
    'BINDGEN_EXTRA_CLANG_ARGS_x86_64_unknown_linux_gnu',
    'BINDGEN_EXTRA_CLANG_ARGS_x86_64-unknown-linux-gnu'
  ]
  for (const key of keys) {
    const env = configureTauriEnvironment(
      { LIBCLANG_PATH: '/caller/libclang', [key]: 'caller' },
      { platform: 'linux', llvmRoots: ['/does/not/exist'] }
    )
    assert.equal(env[key], 'caller')
    assert.equal(env.BINDGEN_EXTRA_CLANG_ARGS, key === 'BINDGEN_EXTRA_CLANG_ARGS' ? 'caller' : undefined)
  }
})

test('a slot without a clang driver leaves bindgen flags alone', () => {
  const fixture = mkdtempSync(path.join(tmpdir(), 'tennoscope-tauri-env-'))
  const llvm98Lib = path.join(fixture, 'llvm', '98', 'lib64')

  const env = configureTauriEnvironment(
    { LIBCLANG_PATH: llvm98Lib },
    { platform: 'linux', llvmRoots: [path.join(fixture, 'llvm')] }
  )

  assert.equal(env.LIBCLANG_PATH, llvm98Lib)
  assert.equal(env.BINDGEN_EXTRA_CLANG_ARGS, undefined)
})

test('slot-local resources cover driver-less installs', (t) => {
  const fixture = mkdtempSync(path.join(tmpdir(), 'tennoscope-tauri-env-'))
  t.after(() => rmSync(fixture, { recursive: true, force: true }))

  const llvmRoot = path.join(fixture, 'llvm')
  const fallback = path.join(llvmRoot, '97', 'lib', 'clang', '97')
  mkdirSync(fallback, { recursive: true })

  const env = configureTauriEnvironment(
    { LIBCLANG_PATH: path.join(llvmRoot, '97', 'lib64') },
    { platform: 'linux', llvmRoots: [llvmRoot] }
  )

  assert.equal(env.BINDGEN_EXTRA_CLANG_ARGS, `-resource-dir=${fallback}`)
})

test('bindgen defaults stay Linux-only', () => {
  const env = configureTauriEnvironment(
    { LIBCLANG_PATH: '/caller/libclang' },
    { platform: 'darwin', llvmRoots: ['/does/not/exist'] }
  )

  assert.equal(env.BINDGEN_EXTRA_CLANG_ARGS, undefined)
})
