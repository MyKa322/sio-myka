/**
 * Builds the elevated helper and stages it where the Tauri bundler expects a sidecar.
 *
 * Without this an *installed* copy of SIO has no `sio-broker.exe` beside it and cannot
 * elevate at all — `cargo tauri build` only compiles the app crate. It works when
 * running from source purely because cargo happens to put both binaries in the same
 * target directory.
 *
 * Tauri requires sidecars to carry a target-triple suffix and strips it again when
 * bundling, so `sio-broker-x86_64-pc-windows-msvc.exe` is installed as `sio-broker.exe`
 * next to the app — exactly what `broker::broker_path()` looks for.
 */
import { execFileSync } from 'node:child_process'
import { copyFileSync, mkdirSync, existsSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const repoRoot = resolve(here, '../../..')

// Tauri sets TAURI_ENV_DEBUG for `tauri dev` and debug builds.
const debug = process.env.TAURI_ENV_DEBUG === 'true' || process.argv.includes('--debug')
const profile = debug ? 'debug' : 'release'
const triple = process.env.TAURI_ENV_TARGET_TRIPLE ?? 'x86_64-pc-windows-msvc'

const cargoArgs = ['build', '-p', 'sio-broker', ...(debug ? [] : ['--release'])]
console.log(`[prepare-broker] cargo ${cargoArgs.join(' ')}`)
execFileSync('cargo', cargoArgs, { cwd: repoRoot, stdio: 'inherit' })

const source = join(repoRoot, 'target', profile, 'sio-broker.exe')
if (!existsSync(source)) {
  throw new Error(`[prepare-broker] cargo reported success but ${source} is missing`)
}

const destinationDir = join(here, '..', 'src-tauri', 'binaries')
mkdirSync(destinationDir, { recursive: true })

const destination = join(destinationDir, `sio-broker-${triple}.exe`)
copyFileSync(source, destination)
console.log(`[prepare-broker] staged ${destination}`)
