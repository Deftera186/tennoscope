import { execFileSync } from 'node:child_process'
import { accessSync, constants, readdirSync } from 'node:fs'
import path from 'node:path'

const DEFAULT_LLVM_ROOTS = ['/usr/lib/llvm', '/usr/lib64/llvm']
const LIBCLANG_NAME = /^libclang(?!-cpp)(?:-[^.]+)?\.so(?:\..+)?$/

function containsLibclang(directory) {
  try {
    return readdirSync(directory).some((name) => LIBCLANG_NAME.test(name))
  } catch {
    return false
  }
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
  return env
}
