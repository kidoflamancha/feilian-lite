import assert from 'node:assert/strict'
import fs from 'node:fs'
import { execFileSync } from 'node:child_process'
import { after, before, test } from 'node:test'
import { fileURLToPath } from 'node:url'

const configPath = new URL('../src-tauri/tauri.conf.json', import.meta.url)
const scriptPath = fileURLToPath(new URL('./set-release-version.mjs', import.meta.url))
let originalConfig

before(() => {
  originalConfig = fs.readFileSync(configPath, 'utf8')
})

after(() => {
  fs.writeFileSync(configPath, originalConfig)
})

function setVersion(version) {
  execFileSync(process.execPath, [scriptPath, version], { stdio: 'pipe' })
  return JSON.parse(fs.readFileSync(configPath, 'utf8'))
}

test('maps a textual prerelease to a valid MSI version', () => {
  const config = setVersion('0.1.0-dev')

  assert.equal(config.version, '0.1.0-dev')
  assert.equal(config.bundle.windows.wix.version, '0.1.0.0')
})

test('preserves a numeric prerelease as the MSI build number', () => {
  const config = setVersion('v1.2.3-42')

  assert.equal(config.version, '1.2.3-42')
  assert.equal(config.bundle.windows.wix.version, '1.2.3.42')
})

test('rejects invalid versions before bundling', () => {
  assert.throws(
    () => setVersion('1.2'),
    /Invalid release version/,
  )
})

test('rejects versions outside MSI component limits', () => {
  assert.throws(
    () => setVersion('256.1.0'),
    /MSI major version cannot be greater than 255/,
  )
})