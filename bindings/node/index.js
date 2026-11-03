'use strict'

const native = require('./binding.js')

const FORMAT_VERSION = native.FORMAT_VERSION

function languages() {
  return native.languages()
}

// Async, so argument errors reject like every other failure.
async function diff(request, options) {
  const signal = options?.signal
  if (typeof request !== 'object' || request === null) throw invalid('request must be an object')
  if (signal !== undefined && typeof signal?.addEventListener !== 'function') {
    throw invalid('options.signal must be an AbortSignal')
  }
  if (signal?.aborted) throw abortError(signal)
  const token = new native.CancelToken()
  const work = start(() => native.diff(request, token))
  if (signal === undefined) return JSON.parse(await work)

  // Abort rejects at once; the native request stops at its next file and
  // its own rejection is dropped.
  let onAbort
  const aborted = new Promise((_, reject) => {
    onAbort = () => {
      token.cancel()
      reject(abortError(signal))
    }
    signal.addEventListener('abort', onAbort, { once: true })
  })
  work.catch(() => {})
  try {
    return JSON.parse(await Promise.race([work, aborted]))
  } catch (error) {
    if (error?.code === 'Cancelled') throw abortError(signal)
    throw error
  } finally {
    signal.removeEventListener('abort', onAbort)
  }
}

async function expand(file, gap, options) {
  return JSON.parse(await start(() => native.expand(file, gap, options?.syntax)))
}

// The binding throws synchronously only for its arguments (each conversion
// with its own code), or with GenericFailure when it cannot start a thread.
function start(call) {
  try {
    return call()
  } catch (error) {
    if (error?.code === 'GenericFailure') throw error
    throw invalid(error?.message ?? String(error), error)
  }
}

function invalid(message, cause) {
  const error = new TypeError(message, { cause })
  error.code = 'InvalidArg'
  return error
}

function abortError(signal) {
  const error = new Error('The operation was aborted', { cause: signal.reason })
  error.name = 'AbortError'
  error.code = 'ABORT_ERR'
  return error
}

module.exports = { FORMAT_VERSION, languages, diff, expand }
