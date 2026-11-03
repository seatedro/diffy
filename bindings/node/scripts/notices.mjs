// Writes RUST_NOTICES.md: every Rust crate compiled into the addon for a
// target, with its license and the license files it ships. Fails, writing
// nothing, if a crate's license is not on the permissive allowlist.
//
//   node scripts/notices.mjs [--target TRIPLE] [--out DIR] [--locked]
//
// The grammars compiled in are credited separately, in the bundle's
// THIRD_PARTY_NOTICES.md.

import { spawnSync } from 'node:child_process'
import { mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseArgs } from 'node:util'

const PACKAGE = 'diffy-node'
// SPDX identifiers that need no more than this notice. MPL-2.0 is
// file-level copyleft: it binds only the crate's own files, unmodified here.
const PERMITTED = new Set([
  'MIT',
  'Apache-2.0',
  'BSD-2-Clause',
  'BSD-3-Clause',
  'ISC',
  'Unicode-3.0',
  'Zlib',
  '0BSD',
  'CC0-1.0',
  'BSL-1.0',
  'MPL-2.0',
])
const PERMITTED_EXCEPTIONS = new Set(['LLVM-exception'])
const LICENSE_FILE = /^(licen[cs]e|copying|notice)/i
// Crates whose packages ship no license file, with the text from their
// repository, so their copyright notice is still reproduced.
const NAPI_RS = { path: resolve(dirname(fileURLToPath(import.meta.url)), '../notices/napi-rs-LICENSE'), source: 'github.com/napi-rs/napi-rs' }
const FALLBACK_LICENSES = new Map(['napi', 'napi-sys', 'napi-derive', 'napi-derive-backend'].map((name) => [name, NAPI_RS]))

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const { values } = parseArgs({
  options: {
    target: { type: 'string' },
    out: { type: 'string', default: root },
    locked: { type: 'boolean', default: false },
  },
})
const target = values.target ?? hostTriple()
const locked = values.locked ? ['--locked'] : []

// The crate set comes from `cargo tree`, which resolves features for this
// package alone, as `cargo build -p` does. `cargo metadata` resolves them
// for the whole workspace, so its graph also holds crates only the app
// links (its HTTP client and TLS stack).
const tree = cargo(['tree', '-p', PACKAGE, '-e', 'normal', '--target', target, '--prefix', 'none', '--format', '{p}', ...locked])
const used = new Set()
for (const line of tree.split('\n')) {
  const match = line.match(/^(\S+) v(\S+)/)
  if (match) used.add(`${match[1]} ${match[2]}`)
}
if (used.size === 0) fail(`cargo tree listed no crates for ${PACKAGE}`)

const metadata = JSON.parse(
  cargo(['metadata', '--format-version', '1', '--filter-platform', target, ...locked]),
)
const workspace = new Set(metadata.workspace_members)
const packages = new Map()
for (const pkg of metadata.packages) {
  const key = `${pkg.name} ${pkg.version}`
  if (!used.has(key)) continue
  if (packages.has(key)) fail(`two packages are ${key}; cannot tell which is linked`)
  packages.set(key, pkg)
}
for (const key of used) if (!packages.has(key)) fail(`cargo metadata has no package ${key}`)

const crates = [...packages.values()].sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version))
const refused = crates.filter((pkg) => !permitted(pkg.license ?? ''))
if (refused.length > 0) {
  for (const pkg of refused) {
    console.error(`notices: ${pkg.name} ${pkg.version} is ${pkg.license ?? `unlicensed (license-file ${pkg.license_file})`}`)
  }
  fail(`${refused.length} crate(s) have a license outside the allowlist`)
}

const ours = crates.filter((pkg) => workspace.has(pkg.id))
const theirs = crates.filter((pkg) => !workspace.has(pkg.id))
const out = [
  '# Rust crates in this binary',
  '',
  `The Rust crates compiled into \`@diffy/node\` for \`${target}\`, with their licenses and the license files each one ships. The tree-sitter grammars compiled in are listed in THIRD_PARTY_NOTICES.md.`,
  '',
  `Diffy's own crates (${ours.map((pkg) => `\`${pkg.name}\``).join(', ')}) are ${[...new Set(ours.map((pkg) => pkg.license))].join('; ')}: see LICENSE-MIT and LICENSE-APACHE.`,
]
// Many crates ship the same license text; it is printed once.
const printed = new Map()
for (const pkg of theirs) {
  out.push('', `## ${pkg.name} ${pkg.version}`, '', `License: ${pkg.license}`)
  if (pkg.repository) out.push(`Repository: ${pkg.repository}`)
  const files = licenseFiles(pkg)
  const fallback = FALLBACK_LICENSES.get(pkg.name)
  if (files.length === 0 && !fallback) out.push('', `No license file in the package; its license is \`${pkg.license}\`.`)
  const paths = files.map((file) => [file, resolve(dirname(pkg.manifest_path), file)])
  if (files.length === 0 && fallback) paths.push([`LICENSE (from ${fallback.source})`, fallback.path])
  for (const [file, path] of paths) {
    const text = readFileSync(path, 'utf8')
    const key = text.replaceAll('\r\n', '\n').replace(/[ \t]+$/gm, '').trim()
    const first = printed.get(key)
    if (first) {
      out.push('', `${file}: the same text as ${first}.`)
      continue
    }
    printed.set(key, `${pkg.name} ${pkg.version}'s ${file}`)
    out.push('', `### ${file}`, '', ...fenced(key))
  }
}

mkdirSync(values.out, { recursive: true })
const output = join(values.out, 'RUST_NOTICES.md')
writeFileSync(output, `${out.join('\n')}\n`)
console.log(`${output}: ${theirs.length} crates`)

/** The top-level license files of `pkg`, and its `license-file`. */
function licenseFiles(pkg) {
  const dir = dirname(pkg.manifest_path)
  const files = new Set(readdirSync(dir).filter((name) => LICENSE_FILE.test(name) && statSync(join(dir, name)).isFile()))
  if (pkg.license_file) files.add(pkg.license_file)
  return [...files].sort()
}

/** Whether an SPDX expression (or the legacy `A/B`) lets us ship the crate. */
function permitted(expression) {
  const tokens = expression.replaceAll('/', ' OR ').match(/[()]|[^\s()]+/g) ?? []
  let at = 0
  // Each side is always parsed, so `at` moves past it.
  const any = () => {
    let ok = all()
    while (tokens[at] === 'OR') {
      at++
      ok = all() || ok
    }
    return ok
  }
  const all = () => {
    let ok = one()
    while (tokens[at] === 'AND') {
      at++
      ok = one() && ok
    }
    return ok
  }
  const one = () => {
    const token = tokens[at++]
    if (token === '(') {
      const ok = any()
      return tokens[at++] === ')' && ok
    }
    let ok = PERMITTED.has(token)
    if (tokens[at] === 'WITH') {
      at++
      ok = PERMITTED_EXCEPTIONS.has(tokens[at++]) && ok
    }
    return ok
  }
  const ok = any()
  return ok && at === tokens.length
}

/** `text` in a code fence longer than any backtick run inside it. */
function fenced(text) {
  const longest = Math.max(0, ...(text.match(/`+/g) ?? []).map((run) => run.length))
  const fence = '`'.repeat(Math.max(3, longest + 1))
  return [`${fence}text`, text, fence]
}

function cargo(args) {
  const run = spawnSync('cargo', args, { cwd: root, encoding: 'utf8', maxBuffer: 1 << 30, stdio: ['ignore', 'pipe', 'inherit'] })
  if (run.error) fail(`cargo: ${run.error.message}`)
  if (run.status !== 0) fail(`cargo ${args[0]} failed (${run.status})`)
  return run.stdout
}

function hostTriple() {
  const rustc = spawnSync('rustc', ['-vV'], { encoding: 'utf8' })
  const host = rustc.stdout?.match(/^host: (\S+)$/m)?.[1]
  if (!host) fail('could not read the host triple from `rustc -vV`')
  return host
}

function fail(message) {
  console.error(`notices: ${message}`)
  process.exit(1)
}
