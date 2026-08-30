import { execFileSync } from 'node:child_process'
import { chmodSync, copyFileSync, mkdirSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDirectory = dirname(fileURLToPath(import.meta.url))
const workspace = resolve(scriptDirectory, '../../..')
const profile = process.argv[2] ?? 'release'
const rustcVersion = execFileSync('rustc', ['-vV'], { encoding: 'utf8' })
const host = rustcVersion.match(/^host: (.+)$/m)?.[1]

if (!host) {
  throw new Error('Could not determine the Rust target triple')
}

const target =
  process.env.FEILIAN_TARGET_TRIPLE ??
  process.env.CARGO_BUILD_TARGET ??
  process.env.TAURI_ENV_TARGET_TRIPLE ??
  host
const extension = target.includes('windows') ? '.exe' : ''
const targetDirectory = process.env.CARGO_BUILD_TARGET || process.env.FEILIAN_TARGET_TRIPLE
  ? resolve(workspace, 'target', target)
  : resolve(workspace, 'target')
const source = resolve(targetDirectory, profile, `feilian-helper${extension}`)
const destinationDirectory = resolve(workspace, 'apps/desktop/src-tauri/binaries')
const destination = resolve(destinationDirectory, `feilian-helper-${target}${extension}`)

mkdirSync(destinationDirectory, { recursive: true })
copyFileSync(source, destination)
if (!extension) chmodSync(destination, 0o755)

if (extension) {
  execFileSync(
    process.execPath,
    [
      resolve(scriptDirectory, 'stage-wintun.mjs'),
      target,
      destinationDirectory,
    ],
    { stdio: 'inherit' },
  )
}

console.log(`Staged ${destination}`)