// Builds the addon with the `node` profile and copies it to
// `diffy.<platform>.node`, the name binding.js and the platform packages use.
//
//   node scripts/build.mjs [--target TRIPLE] [--out DIR] [--profile NAME] [--locked]
//
// Set DIFFY_SYNTAX_BUNDLE to the absolute `syntax-pack bundle --out` directory
// to compile grammars in; without it the build has none.

import { spawnSync } from 'node:child_process'
import { copyFileSync, mkdirSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseArgs } from 'node:util'

const PLATFORMS = {
  'aarch64-apple-darwin': 'darwin-arm64',
  'x86_64-apple-darwin': 'darwin-x64',
  'x86_64-unknown-linux-gnu': 'linux-x64-gnu',
  'aarch64-unknown-linux-gnu': 'linux-arm64-gnu',
  'x86_64-pc-windows-msvc': 'win32-x64-msvc',
}

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const { values } = parseArgs({
  options: {
    target: { type: 'string' },
    out: { type: 'string', default: root },
    profile: { type: 'string', default: 'node' },
    locked: { type: 'boolean', default: false },
  },
})

const target = values.target ?? hostTriple()
const platform = PLATFORMS[target]
if (!platform) fail(`no npm platform for ${target}; known: ${Object.keys(PLATFORMS).join(', ')}`)
if (!process.env.DIFFY_SYNTAX_BUNDLE) console.warn('DIFFY_SYNTAX_BUNDLE is not set: no grammars')

const args = ['build', '-p', 'diffy-node', '--profile', values.profile, '--message-format=json-render-diagnostics']
if (values.target) args.push('--target', values.target)
if (values.locked) args.push('--locked')
const cargo = spawnSync('cargo', args, {
  cwd: root,
  stdio: ['ignore', 'pipe', 'inherit'],
  encoding: 'utf8',
  maxBuffer: 1 << 30,
})
if (cargo.error) fail(`cargo: ${cargo.error.message}`)
if (cargo.status !== 0) fail(`cargo build failed (${cargo.status})`)

let library
for (const line of cargo.stdout.split('\n')) {
  if (!line.startsWith('{')) continue
  const message = JSON.parse(line)
  if (message.reason === 'compiler-artifact' && message.target?.name === 'diffy_node') {
    library = message.filenames.find((file) => /\.(so|dylib|dll)$/.test(file)) ?? library
  }
}
if (!library) fail('cargo reported no diffy_node library')

mkdirSync(values.out, { recursive: true })
const output = join(values.out, `diffy.${platform}.node`)
copyFileSync(library, output)
console.log(output)

function hostTriple() {
  const rustc = spawnSync('rustc', ['-vV'], { encoding: 'utf8' })
  const host = rustc.stdout?.match(/^host: (\S+)$/m)?.[1]
  if (!host) fail('could not read the host triple from `rustc -vV`')
  return host
}

function fail(message) {
  console.error(`build: ${message}`)
  process.exit(1)
}
