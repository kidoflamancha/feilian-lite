import fs from 'node:fs'

const version = process.argv[2]?.replace(/^v/, '')
const match = version?.match(/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/)

if (!match) {
  throw new Error(`Invalid release version: ${process.argv[2] ?? '(missing)'}`)
}

const [, major, minor, patch, prerelease] = match
const numericPrerelease = prerelease && /^\d+$/.test(prerelease)
  ? Number.parseInt(prerelease, 10)
  : 0

for (const [name, value, limit] of [
  ['major', major, 255],
  ['minor', minor, 255],
  ['patch', patch, 65535],
  ['build', numericPrerelease, 65535],
]) {
  if (Number(value) > limit) {
    throw new Error(`MSI ${name} version cannot be greater than ${limit}`)
  }
}

const configPath = new URL('../src-tauri/tauri.conf.json', import.meta.url)
const config = JSON.parse(fs.readFileSync(configPath, 'utf8'))
config.version = version
config.bundle ??= {}
config.bundle.windows ??= {}
config.bundle.windows.wix ??= {}
config.bundle.windows.wix.version = `${major}.${minor}.${patch}.${numericPrerelease}`
fs.writeFileSync(configPath, `${JSON.stringify(config, null, 2)}\n`)

console.log(`Set bundle version ${version} (MSI ${config.bundle.windows.wix.version})`)