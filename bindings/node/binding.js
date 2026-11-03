'use strict'

// Loads $DIFFY_NODE_BINARY (resolved from the cwd), else a local
// `npm run build` output, else this platform's package.

const { existsSync, readFileSync } = require('node:fs')
const { join, resolve } = require('node:path')

function platformName() {
  const { platform, arch } = process
  if (platform === 'linux') return `linux-${arch}-${isMusl() ? 'musl' : 'gnu'}`
  if (platform === 'win32') return `win32-${arch}-msvc`
  return `${platform}-${arch}`
}

function isMusl() {
  try {
    return readFileSync('/usr/bin/ldd', 'utf8').includes('musl')
  } catch {
    return false
  }
}

function load() {
  if (process.env.DIFFY_NODE_BINARY) return require(resolve(process.env.DIFFY_NODE_BINARY))
  const name = platformName()
  const local = join(__dirname, `diffy.${name}.node`)
  if (existsSync(local)) return require(local)
  try {
    return require(`@seatedro/diffy-${name}`)
  } catch (error) {
    throw new Error(
      `@seatedro/diffy has no binary for ${name}: install @seatedro/diffy-${name} ` +
        '(npm skips optional dependencies with --omit=optional), or set DIFFY_NODE_BINARY',
      { cause: error },
    )
  }
}

module.exports = load()
