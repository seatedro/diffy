'use strict'

const native = require('./binding.js')

const FORMAT_VERSION = native.FORMAT_VERSION

function languages() {
  return native.languages()
}

async function diff(request, options) {
  return JSON.parse(await diffJson(request, options))
}

// Async, so argument errors reject like every other failure.
async function diffJson(request, options) {
  const signal = options?.signal
  if (typeof request !== 'object' || request === null) throw invalid('request must be an object')
  if (signal !== undefined && typeof signal?.addEventListener !== 'function') {
    throw invalid('options.signal must be an AbortSignal')
  }
  if (signal?.aborted) throw abortError(signal)
  const token = new native.CancelToken()
  const work = call(() => native.diff(request, token))
  if (signal === undefined) return work

  // Abort rejects at once; a queued native request never starts, a
  // running one stops at its next file, and its own rejection is dropped.
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
    return await Promise.race([work, aborted])
  } catch (error) {
    if (error?.code === 'Cancelled') throw abortError(signal)
    throw error
  } finally {
    signal.removeEventListener('abort', onAbort)
  }
}

async function expand(file, gap, options) {
  return JSON.parse(await call(() => native.expand(file, gap, options?.syntax)))
}

function configure(options) {
  if (options !== undefined && (typeof options !== 'object' || options === null)) {
    throw invalid('options must be an object')
  }
  const state = call(() => native.queue(options?.max_concurrent_requests))
  return { max_concurrent_requests: state.limit }
}

// The binding throws synchronously only for its arguments (each conversion
// with its own code), or with GenericFailure when it cannot start a thread.
function call(binding) {
  try {
    return binding()
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

module.exports = { FORMAT_VERSION, languages, configure, diff, diffJson, expand }
