// npm test  (after `npm run build` with a bundle holding rust and typescript)

import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { availableParallelism } from 'node:os'
import { afterEach, beforeEach, describe, it } from 'node:test'

import { FORMAT_VERSION, configure, diff, diffJson, expand, languages } from '../index.js'

const native = createRequire(import.meta.url)('../binding.js')

const OLD_RS = `use std::fmt;

fn render(width: usize) -> String {
    let label = "é😀";
    format!("{label} {width}")
}

fn unchanged() {}
`
const NEW_RS = OLD_RS.replace('fn render(', 'fn render_preview(').replace('{width}', '{width}!')

const modified = (path, old, next) => ({ status: 'modified', old_path: path, new_path: path, old, new: next })

/** Many files with real edits, so a request runs long enough to abort. */
function bigRequest(count) {
  const body = Array.from({ length: 400 }, (_, i) => `fn f${i}(x: u32) -> u32 { x + ${i} }`).join('\n')
  const files = Array.from({ length: count }, (_, i) =>
    modified(`src/f${i}.rs`, body, body.replaceAll('x + 1', 'x * 2').replace('fn f3(', 'fn g3(')),
  )
  return { files }
}

describe('build', () => {
  it('reports the format version', () => {
    assert.equal(FORMAT_VERSION, 1)
  })

  it('lists the bundled grammars', () => {
    const names = languages()
    assert.ok(Array.isArray(names))
    for (const name of ['rust', 'typescript']) assert.ok(names.includes(name), `${name} is bundled`)
  })
})

describe('expand', () => {
  const file = modified('src/view.rs', OLD_RS, OLD_RS)

  it('returns context rows', async () => {
    const rows = await expand(file, { old_start: 3, new_start: 3, len: 2 })
    assert.deepEqual(rows, [
      { kind: 'context', old_line: 3, new_line: 3, text: 'fn render(width: usize) -> String {' },
      { kind: 'context', old_line: 4, new_line: 4, text: '    let label = "é😀";' },
    ])
  })

  it('adds syntax spans in UTF-16 offsets', async () => {
    const [fn, label] = await expand(file, { old_start: 3, new_start: 3, len: 2 }, { syntax: true })
    const at = (row, cls) => row.syntax.filter((s) => s.class === cls).map((s) => row.text.slice(s.start, s.end))
    assert.ok(at(fn, 'keyword').includes('fn'))
    assert.ok(at(fn, 'function').includes('render'))
    assert.ok(at(label, 'string').some((text) => text.includes('é😀')))
  })

  it('takes bytes and strings alike', async () => {
    const bytes = new TextEncoder().encode(OLD_RS)
    // A view into a larger buffer: only the view's bytes count.
    const padded = new Uint8Array(bytes.length + 8)
    padded.set(bytes, 4)
    const view = padded.subarray(4, 4 + bytes.length)
    const gap = { old_start: 1, new_start: 1, len: 100 }
    const want = await expand(file, gap)
    assert.deepEqual(await expand({ ...file, old: bytes, new: Buffer.from(OLD_RS) }, gap), want)
    assert.deepEqual(await expand({ ...file, old: view, new: view }, gap), want)
    assert.equal(want.length, 8)
  })

  it('stops where the sides differ or end', async () => {
    const changed = modified('a.rs', 'a\nb\nc\n', 'a\nB\nc\n')
    assert.equal((await expand(changed, { old_start: 1, new_start: 1, len: 3 })).length, 1)
    assert.equal((await expand(changed, { old_start: 3, new_start: 3, len: 10 })).length, 1)
    assert.deepEqual(await expand(changed, { old_start: 1, new_start: 1, len: 0 }), [])
  })
})

describe('abort', () => {
  it('rejects without starting when already aborted', async () => {
    const controller = new AbortController()
    const reason = new Error('navigated away')
    controller.abort(reason)
    await assert.rejects(diff(bigRequest(1), { signal: controller.signal }), (error) => {
      assert.equal(error.name, 'AbortError')
      assert.equal(error.code, 'ABORT_ERR')
      assert.equal(error.cause, reason)
      return true
    })
  })

  it('rejects a running request at once', async () => {
    const controller = new AbortController()
    const running = diff(bigRequest(64), { signal: controller.signal })
    controller.abort()
    await assert.rejects(running, { name: 'AbortError' })
  })

  it('leaves the binding usable', async () => {
    const controller = new AbortController()
    const running = diff(bigRequest(256), { signal: controller.signal })
    setImmediate(() => controller.abort())
    await assert.rejects(running, { name: 'AbortError' })
    const rows = await expand(modified('a.rs', 'x\n', 'x\n'), { old_start: 1, new_start: 1, len: 1 })
    assert.equal(rows.length, 1)
  })
})

describe('invalid input rejects', () => {
  const bad = {
    'no request': () => diff(),
    'null request': () => diff(null),
    'files not an array': () => diff({ files: 'a.rs' }),
    'unknown status': () => diff({ files: [{ status: 'changed', new_path: 'a.rs' }] }),
    'missing status': () => diff({ files: [{ new_path: 'a.rs' }] }),
    'no path': () => diff({ files: [{ status: 'modified', old: 'a', new: 'b' }] }),
    'contents a number': () => diff({ files: [{ status: 'added', new_path: 'a.rs', new: 42 }] }),
    'path a number': () => diff({ files: [{ status: 'added', new_path: 7 }] }),
    'negative budget': () => diff({ files: [], budget_ms: -1 }),
    'NaN file budget': () => diff({ files: [], file_budget_ms: NaN }),
    'fractional syntax_max_bytes': () => diff({ files: [], syntax_max_bytes: 1.5 }),
    'syntax not a boolean': () => diff({ files: [], syntax: 'yes' }),
    'signal not a signal': () => diff({ files: [] }, { signal: {} }),
    'negative gap start': () => expand(modified('a.rs', 'a', 'a'), { old_start: -1, new_start: 1, len: 1 }),
    'fractional gap length': () => expand(modified('a.rs', 'a', 'a'), { old_start: 1, new_start: 1, len: 1.5 }),
    'gap missing len': () => expand(modified('a.rs', 'a', 'a'), { old_start: 1, new_start: 1 }),
    'no gap': () => expand(modified('a.rs', 'a', 'a')),
    'diffJson null request': () => diffJson(null),
  }
  for (const [name, call] of Object.entries(bad)) {
    it(name, async () => {
      const result = call()
      assert.ok(result instanceof Promise, 'errors are rejections, not throws')
      await assert.rejects(result, (error) => {
        assert.ok(error instanceof Error)
        assert.equal(error.code, 'InvalidArg', error.message)
        return true
      })
    })
  }
})

describe('diff', () => {
  it('diffs a modified file semantically', async () => {
    const doc = await diff({ files: [modified('src/view.rs', OLD_RS, NEW_RS)] })
    assert.equal(doc.version, FORMAT_VERSION)
    assert.equal(doc.files.length, 1)
    const [file] = doc.files
    assert.equal(file.status, 'modified')
    assert.equal(file.language, 'rs')
    assert.equal(file.fallback, undefined)
    assert.equal(file.additions, 2)
    assert.equal(file.deletions, 2)
    const rows = file.hunks.flatMap((hunk) => hunk.rows)
    const changed = rows.filter((row) => row.kind === 'modified')
    assert.equal(changed.length, 2)
    const [renamed] = changed
    assert.equal(renamed.old.text, 'fn render(width: usize) -> String {')
    assert.equal(renamed.new.text, 'fn render_preview(width: usize) -> String {')
    for (const span of renamed.new.changes ?? []) {
      assert.ok(span.start < span.end && span.end <= renamed.new.text.length)
    }
    assert.ok(file.renames?.some((r) => r.old === 'render' && r.new === 'render_preview'))
  })

  it('adds syntax spans on request', async () => {
    const plain = await diff({ files: [modified('src/view.rs', OLD_RS, NEW_RS)] })
    const highlighted = await diff({ files: [modified('src/view.rs', OLD_RS, NEW_RS)], syntax: true })
    const spans = (doc) =>
      doc.files[0].hunks.flatMap((h) => h.rows).flatMap((row) => [row.syntax, row.old?.syntax, row.new?.syntax])
    assert.ok(spans(plain).every((s) => s === undefined))
    assert.ok(spans(highlighted).some((s) => s?.length > 0))
  })

  it('keeps input order across statuses and kinds', async () => {
    const files = [
      { status: 'added', new_path: 'notes.txt', new: 'hello' },
      { status: 'deleted', old_path: 'old.ts', old: 'export const a = 1\n' },
      modified('logo.png', new Uint8Array([0x89, 0x50, 0, 1]), new Uint8Array([0x89, 0x50, 0, 2])),
      modified('data.unknownext', 'a\nb\n', 'a\nc\n'),
      { status: 'renamed', old_path: 'a.ts', new_path: 'b.ts', old: 'let a = 1\n', new: 'let a = 1\n' },
      modified('bad.rs', Buffer.from([0x66, 0x6e, 0xff, 0x0a]), Buffer.from([0x66, 0x6f, 0xff, 0x0a])),
    ]
    const doc = await diff({ files })
    assert.deepEqual(
      doc.files.map((f) => f.new_path ?? f.old_path),
      ['notes.txt', 'old.ts', 'logo.png', 'data.unknownext', 'b.ts', 'bad.rs'],
    )
    const [added, deleted, binary, unknown, renamed, invalid] = doc.files
    assert.equal(added.status, 'added')
    assert.equal(added.additions, 1)
    assert.equal(added.new_missing_newline, true)
    assert.equal(deleted.deletions, 1)
    assert.equal(binary.binary, true)
    assert.equal(binary.hunks, undefined)
    // The extension may name no language or one with no grammar here.
    assert.ok(['no_grammar', 'grammar_unavailable'].includes(unknown.fallback?.kind))
    assert.equal(renamed.status, 'renamed')
    assert.equal(renamed.hunks, undefined)
    assert.deepEqual(renamed.gap_after, { old_start: 1, new_start: 1, len: 1 })
    const texts = invalid.hunks.flatMap((h) => h.rows).flatMap((row) => [row.old?.text, row.new?.text])
    assert.ok(texts.includes('fn�'))
  })

  it('falls back to line diffs past the request budget', async () => {
    const { files } = bigRequest(8)
    const doc = await diff({ files, budget_ms: 0 })
    assert.equal(doc.files.length, 8)
    for (const file of doc.files) {
      assert.equal(file.fallback?.kind, 'request_budget')
      assert.ok(file.hunks.length > 0)
    }
  })

  it('expands the gaps it returns', async () => {
    // Unchanged functions on both ends, so context collapses before and after.
    const pad = Array.from({ length: 12 }, (_, i) => `fn pad${i}() {}`).join('\n')
    const file = modified('src/view.rs', `${pad}\n${OLD_RS}${pad}\n`, `${pad}\n${NEW_RS}${pad}\n`)
    const doc = await diff({ files: [file] })
    const gaps = [...doc.files[0].hunks.map((h) => h.gap_before), doc.files[0].gap_after].filter(Boolean)
    assert.equal(gaps.length, 2)
    for (const gap of gaps) {
      const rows = await expand(file, gap)
      assert.equal(rows.length, gap.len)
    }
  })

  it('runs requests concurrently', async () => {
    const docs = await Promise.all(Array.from({ length: 8 }, () => diff(bigRequest(4))))
    for (const doc of docs) assert.equal(doc.files.length, 4)
  })

  it('stops the native request on cancel', async () => {
    const token = new native.CancelToken()
    token.cancel()
    await assert.rejects(native.diff(bigRequest(256), token), { code: 'Cancelled' })
  })

  it('finishes when the signal never aborts', async () => {
    const controller = new AbortController()
    const doc = await diff({ files: [modified('a.rs', 'a\n', 'b\n')] }, { signal: controller.signal })
    assert.equal(doc.files.length, 1)
    controller.abort()
  })
})

describe('diffJson', () => {
  it('resolves to the JSON diff resolves to', async () => {
    const request = { files: [modified('src/view.rs', OLD_RS, NEW_RS)], syntax: true }
    const json = await diffJson(request)
    assert.equal(typeof json, 'string')
    assert.deepEqual(JSON.parse(json), await diff(request))
  })

  it('aborts like diff', async () => {
    const controller = new AbortController()
    const running = diffJson(bigRequest(64), { signal: controller.signal })
    controller.abort()
    await assert.rejects(running, { name: 'AbortError' })
  })
})

// The queue is process-wide; each test restores the default limit.
describe('queue', () => {
  const { max_concurrent_requests: initial } = configure()
  const small = () => diff({ files: [modified('a.rs', 'a\n', 'b\n')] })
  // Aborted requests from earlier tests reject at once but leave the queue
  // only when their native work stops.
  beforeEach(async () => {
    const deadline = Date.now() + 30_000
    while (native.queue().running + native.queue().queued > 0) {
      assert.ok(Date.now() < deadline, 'queue never drained')
      await new Promise((resolve) => setTimeout(resolve, 10))
    }
  })
  afterEach(() => configure({ max_concurrent_requests: initial }))

  it('defaults to a quarter of the cores, at least one', () => {
    assert.ok(Number.isInteger(initial) && initial >= 1)
    assert.ok(initial <= Math.max(1, Math.floor(availableParallelism() / 4)))
  })

  it('runs requests in order, at most the limit at once', async () => {
    assert.deepEqual(configure({ max_concurrent_requests: 1 }), { max_concurrent_requests: 1 })
    const order = []
    const done = [diff(bigRequest(64)).then(() => order.push('big'))]
    for (let i = 0; i < 4; i++) done.push(small().then(() => order.push(i)))
    assert.deepEqual(native.queue(), { limit: 1, running: 1, queued: 4 })
    await Promise.all(done)
    assert.deepEqual(order, ['big', 0, 1, 2, 3])
    assert.deepEqual(native.queue(), { limit: 1, running: 0, queued: 0 })
  })

  it('never runs more than the limit', async () => {
    configure({ max_concurrent_requests: 2 })
    let settled = false
    const all = Promise.all(Array.from({ length: 6 }, () => diff(bigRequest(16)))).finally(() => {
      settled = true
    })
    let most = 0
    while (!settled) {
      const { running, queued } = native.queue()
      most = Math.max(most, running)
      assert.ok(running + queued <= 6)
      await new Promise((resolve) => setImmediate(resolve))
    }
    await all
    assert.equal(most, 2)
  })

  it('starts queued requests when the limit rises', async () => {
    configure({ max_concurrent_requests: 1 })
    const done = Promise.all(Array.from({ length: 3 }, () => diff(bigRequest(16))))
    assert.equal(native.queue().queued, 2)
    configure({ max_concurrent_requests: 3 })
    assert.deepEqual(native.queue(), { limit: 3, running: 3, queued: 0 })
    await done
  })

  it('rejects a request aborted while queued at once, without starting it', async () => {
    configure({ max_concurrent_requests: 1 })
    const running = diff(bigRequest(64))
    const controller = new AbortController()
    const reason = new Error('navigated away')
    const queued = diff(bigRequest(64), { signal: controller.signal })
    assert.deepEqual(native.queue(), { limit: 1, running: 1, queued: 1 })
    controller.abort(reason)
    assert.deepEqual(native.queue(), { limit: 1, running: 1, queued: 0 })
    let pending = true
    running.finally(() => {
      pending = false
    })
    await assert.rejects(queued, (error) => {
      assert.equal(error.name, 'AbortError')
      assert.equal(error.cause, reason)
      return true
    })
    assert.ok(pending, 'rejected before the running request finished')
    await running
  })

  it('counts time queued against budget_ms', async () => {
    configure({ max_concurrent_requests: 1 })
    const running = diff(bigRequest(64))
    const doc = await diff({ files: [modified('a.rs', 'fn a() {}\n', 'fn b() {}\n')], budget_ms: 1 })
    assert.equal(doc.files[0].fallback?.kind, 'request_budget')
    await running
  })

  it('does not queue expand', async () => {
    configure({ max_concurrent_requests: 1 })
    let pending = true
    const running = diff(bigRequest(64)).finally(() => {
      pending = false
    })
    const rows = await expand(modified('a.rs', 'x\n', 'x\n'), { old_start: 1, new_start: 1, len: 1 })
    assert.equal(rows.length, 1)
    assert.ok(pending, 'expand finished while a diff held the only slot')
    await running
  })

  for (const [name, options] of Object.entries({
    zero: { max_concurrent_requests: 0 },
    fractional: { max_concurrent_requests: 1.5 },
    'not a number': { max_concurrent_requests: '2' },
    'not an object': 4,
    null: null,
  })) {
    it(`rejects a ${name} limit`, () => {
      assert.throws(() => configure(options), (error) => {
        assert.ok(error instanceof TypeError)
        assert.equal(error.code, 'InvalidArg')
        return true
      })
      assert.equal(configure().max_concurrent_requests, initial)
    })
  }
})
