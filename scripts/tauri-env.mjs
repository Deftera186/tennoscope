import { execFileSync } from 'node:child_process'
import { accessSync, constants, readdirSync } from 'node:fs'
import path from 'node:path'

const DEFAULT_LLVM_ROOTS = ['/usr/lib/llvm', '/usr/lib64/llvm']
const LIBCLANG_NAME = /^libclang(?!-cpp)(?:-[^.]+)?\.so(?:\..+)?$/
const LIBCLANG_FILE = /^libclang[^/]*\.so/
const BINDGEN_KEYS = [
  'BINDGEN_EXTRA_CLANG_ARGS',
  'BINDGEN_EXTRA_CLANG_ARGS_x86_64_unknown_linux_gnu',
  'BINDGEN_EXTRA_CLANG_ARGS_x86_64-unknown-linux-gnu'
]

function containsLibclang(directory) {
  try {
    return readdirSync(directory).some((name) => LIBCLANG_NAME.test(name))
  } catch {
    return false
  }
}

// Builtin headers sit next to the driver, not next to libclang, on split layouts.
function readable(directory) {
  try {
    accessSync(directory, constants.R_OK)
    return true
  } catch {
    return false
  }
}

function clangResourceDir(slotDir) {
  try {
    const resource = execFileSync(path.join(slotDir, 'bin', 'clang'), ['--print-resource-dir'], {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore']
    }).trim()
    if (resource !== '' && readable(resource)) return path.normalize(resource)
  } catch {
    // This LLVM slot has no usable clang driver.
  }
  return undefined
}

function resolveBindgenResourceDir(libclangPath) {
  const libdir = LIBCLANG_FILE.test(path.basename(libclangPath))
    ? path.dirname(libclangPath)
    : libclangPath
  const slotDir = path.dirname(libdir)
  const slot = path.basename(slotDir)
  const fromDriver = clangResourceDir(slotDir)
  if (fromDriver !== undefined) return fromDriver
  for (const candidate of [path.join(slotDir, 'lib', 'clang', slot), path.join('/usr/lib/clang', slot)]) {
    if (readable(candidate)) return candidate
  }
  return undefined
}

function versionedLlvmConfigs(roots) {
  const configs = []
  for (const root of roots) {
    let slots
    try {
      slots = readdirSync(root, { withFileTypes: true })
        .filter((entry) => entry.isDirectory())
        .map((entry) => entry.name)
        .sort((left, right) => right.localeCompare(left, undefined, { numeric: true }))
    } catch {
      continue
    }

    for (const slot of slots) {
      const config = path.join(root, slot, 'bin', 'llvm-config')
      try {
        accessSync(config, constants.X_OK)
        configs.push(config)
      } catch {
        // This LLVM slot has no executable llvm-config.
      }
    }
  }
  return configs
}

function discoverLibclang(env, roots) {
  const configs = [env.LLVM_CONFIG_PATH, 'llvm-config', ...versionedLlvmConfigs(roots)].filter(Boolean)
  for (const config of configs) {
    let libdir
    try {
      libdir = execFileSync(config, ['--libdir'], {
        encoding: 'utf8',
        env,
        stdio: ['ignore', 'pipe', 'ignore']
      }).trim()
    } catch {
      continue
    }
    if (containsLibclang(libdir)) return libdir
  }
  return undefined
}

export function configureTauriEnvironment(
  source,
  { platform = process.platform, llvmRoots = DEFAULT_LLVM_ROOTS } = {}
) {
  const env = { ...source }
  if (platform !== 'linux') return env

  if (env.NO_STRIP === undefined) env.NO_STRIP = 'true'
  if (env.LIBCLANG_PATH === undefined) {
    const libclangPath = discoverLibclang(env, llvmRoots)
    if (libclangPath !== undefined) env.LIBCLANG_PATH = libclangPath
  }
  // libclang alone cannot see builtin headers on split layouts, so bindgen fails on stdbool.h.
  if (env.LIBCLANG_PATH !== undefined && BINDGEN_KEYS.every((key) => env[key] === undefined)) {
    const resourceDir = resolveBindgenResourceDir(env.LIBCLANG_PATH)
    if (resourceDir !== undefined) env.BINDGEN_EXTRA_CLANG_ARGS = `-resource-dir=${resourceDir}`
  }
  return env
}
