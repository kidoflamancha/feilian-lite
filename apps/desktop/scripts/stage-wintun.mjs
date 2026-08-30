import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

const VERSION = '0.14.1'
const EXPECTED_SHA256 = '07c256185d6ee3652e09fa55c0b673e2624b565e02c4b9091c79ca7d2f24ef51'
const target = process.argv[2]
const destinationDirectory = process.argv[3]

if (!target || !destinationDirectory) {
  throw new Error('Usage: stage-wintun.mjs <target-triple> <destination-directory>')
}

const architecture = target.startsWith('x86_64')
  ? 'amd64'
  : target.startsWith('aarch64')
    ? 'arm64'
    : target.startsWith('i686')
      ? 'x86'
      : null

if (!architecture) throw new Error(`Unsupported Wintun target: ${target}`)

const response = await fetch(`https://www.wintun.net/builds/wintun-${VERSION}.zip`)
if (!response.ok) throw new Error(`Wintun download failed with HTTP ${response.status}`)
const archiveBytes = Buffer.from(await response.arrayBuffer())
const actualSha256 = createHash('sha256').update(archiveBytes).digest('hex')
if (actualSha256 !== EXPECTED_SHA256) {
  throw new Error(`Wintun archive checksum mismatch: ${actualSha256}`)
}

const workDirectory = mkdtempSync(join(tmpdir(), 'feilian-wintun-'))
try {
  const archive = join(workDirectory, 'wintun.zip')
  writeFileSync(archive, archiveBytes)
  if (process.platform === 'win32') {
    execFileSync('tar.exe', ['-xf', archive, '-C', workDirectory])
  } else {
    execFileSync('unzip', ['-q', archive, '-d', workDirectory])
  }
  mkdirSync(destinationDirectory, { recursive: true })
  copyFileSync(
    join(workDirectory, 'wintun', 'bin', architecture, 'wintun.dll'),
    resolve(destinationDirectory, 'wintun.dll'),
  )
} finally {
  rmSync(workDirectory, { recursive: true, force: true })
}

console.log(`Staged Wintun ${VERSION} (${architecture})`)
