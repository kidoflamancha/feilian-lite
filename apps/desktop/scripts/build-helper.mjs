import { execFileSync } from 'node:child_process'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDirectory = dirname(fileURLToPath(import.meta.url))
const workspace = resolve(scriptDirectory, '../../..')
const profile = process.argv[2] ?? 'debug'
const rustcVersion = execFileSync('rustc', ['-vV'], { encoding: 'utf8' })
const host = rustcVersion.match(/^host: (.+)$/m)?.[1]

if (!host) {
  throw new Error('Could not determine the Rust host triple')
}

const target =
  process.env.FEILIAN_TARGET_TRIPLE ??
  process.env.CARGO_BUILD_TARGET ??
  process.env.TAURI_ENV_TARGET_TRIPLE ??
  host
const cargoArguments = [
  'build',
  '--manifest-path',
  resolve(workspace, 'Cargo.toml'),
  '-p',
  'feilian-helper',
]

if (profile === 'release') cargoArguments.push('--release')
if (target !== host) cargoArguments.push('--target', target)

execFileSync('cargo', cargoArguments, { cwd: workspace, stdio: 'inherit' })
execFileSync(process.execPath, [resolve(scriptDirectory, 'stage-helper.mjs'), profile], {
  cwd: workspace,
  env: {
    ...process.env,
    ...(target !== host ? { FEILIAN_TARGET_TRIPLE: target } : {}),
  },
  stdio: 'inherit',
})
