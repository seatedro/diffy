// Output follows Diffy's display format, version 1, specified in
// https://github.com/seatedro/diffy/blob/master/crates/diffy-compare/FORMAT.md
// Ignore unknown fields: new optional fields keep FORMAT_VERSION.

/** Display format version of every Document this build returns. */
export const FORMAT_VERSION: number

/** Canonical names of the bundled grammars, e.g. `"rust"`. */
export function languages(): string[]

/**
 * Diffs off the JS thread; one File per input, in order. Waits in a
 * process-wide FIFO while `max_concurrent_requests` others run.
 * Rejects with an `AbortError` (`code: "ABORT_ERR"`, `cause`: the signal's
 * reason) on abort, and a `TypeError` (`code: "InvalidArg"`) for bad input.
 */
export function diff(request: DiffRequest, options?: DiffOptions): Promise<Document>

/** `diff`, unparsed: forward the Document's JSON without parsing it on the event loop. */
export function diffJson(request: DiffRequest, options?: DiffOptions): Promise<string>

/**
 * Context rows of `gap` (a Document's Gap or a sub-range of one); `file` must
 * hold the contents the Document was made from. Fewer than `gap.len` rows past
 * either side's end or where the sides differ. Bad input rejects as in `diff`.
 */
export function expand(file: FileInput, gap: Gap, options?: ExpandOptions): Promise<Row[]>

/** Sets the given options process-wide; returns all. Throws a `TypeError` (`code: "InvalidArg"`) for a bad value. */
export function configure(options?: Configuration): Required<Configuration>

export interface Configuration {
  /**
   * Concurrent `diff`/`diffJson` calls; the rest queue. Positive integer.
   * Default: cores / 4, at least 1. Raising it starts queued calls at once;
   * lowering it lets running ones finish. `expand` is never queued.
   */
  max_concurrent_requests?: number
}

// ---- Input -------------------------------------------------------------

export type Status = 'modified' | 'added' | 'deleted' | 'renamed' | 'copied'

/**
 * Contents are bytes; a string means its UTF-8. Missing contents are empty.
 * Needs a path; the language comes from `new_path ?? old_path`.
 */
export interface FileInput {
  status: Status
  /** Absent for an added file. */
  old_path?: string
  /** Absent for a deleted file. */
  new_path?: string
  old?: Uint8Array | string
  new?: Uint8Array | string
}

export interface DiffRequest {
  files: FileInput[]
  /**
   * Whole-request budget from the call, time queued included. Files started
   * after it get line diffs (`fallback.kind: "request_budget"`); with
   * `syntax`, files projected after it get no syntax spans or scope headers.
   */
  budget_ms?: number
  /** Per-file semantic diff budget. Default 400. */
  file_budget_ms?: number
  /** Add syntax spans to rows. Default false. */
  syntax?: boolean
  /** Sides longer than this, in bytes, get no syntax spans. Default 1 MiB. */
  syntax_max_bytes?: number
}

export interface DiffOptions {
  signal?: AbortSignal
}

export interface ExpandOptions {
  /** Add syntax spans to rows. Default false. */
  syntax?: boolean
}

// ---- Output: the display format, version 1 ------------------------------
//
// Line numbers are 1-based; ranges are half-open; span offsets are UTF-16
// code units within the row's `text`, so `text.slice(start, end)` is the
// span. Absent arrays mean `[]`, absent booleans `false`.

export interface Document {
  version: 1
  /** In request order. */
  files: File[]
}

export interface File {
  status: Status
  /** Absent for an added file. */
  old_path?: string
  /** Absent for a deleted file. Show `new_path ?? old_path`. */
  new_path?: string
  /**
   * The extension (`rs`) or, without one, the lowercased name (`dockerfile`).
   * Absent when the path names none or `fallback.kind` is `no_grammar`.
   */
  language?: string
  /** A binary file has no `hunks` and no `gap_after`. */
  binary?: boolean
  additions: number
  deletions: number
  /** The old file's last line has no newline. */
  old_missing_newline?: boolean
  /** The new file's last line has no newline. */
  new_missing_newline?: boolean
  /** Present when the file got a line diff instead of a semantic one. */
  fallback?: Fallback
  /** In file order. Render each hunk's `gap_before`, header, rows; then `gap_after`. */
  hunks?: Hunk[]
  /** Unchanged lines after the last hunk; the whole file when there are no hunks. */
  gap_after?: Gap
  /** Items the semantic diff found changed: new-side items, then old-side ones. */
  outline?: OutlineItem[]
  /** Code the semantic diff found moved. */
  moves?: Move[]
  /** Identifiers the semantic diff found renamed. */
  renames?: Rename[]
}

export interface Fallback {
  /**
   * `no_grammar`: no language for the path. `grammar_unavailable`: its grammar
   * failed to load. `semantic`: the semantic diff gave up, see `reason`.
   * `request_budget`: the request deadline passed first.
   */
  kind: 'no_grammar' | 'grammar_unavailable' | 'semantic' | 'request_budget'
  /** Present when `kind` is `semantic`. */
  reason?: 'too_large' | 'parse_errors' | 'error_coverage' | 'unparsed' | 'budget' | 'timeout' | 'invalid'
  /** For people; wording may change. Branch on `kind` and `reason`. */
  message: string
}

export interface Hunk {
  /** A unified diff's `@@ -old_start,old_len +new_start,new_len @@`. */
  old_start: number
  old_len: number
  new_start: number
  new_len: number
  /** Item enclosing the first change, for the header line. */
  header?: string
  /** Unchanged lines since the previous hunk or the top of the file. */
  gap_before?: Gap
  /** In display order; each side's line numbers ascend. */
  rows: Row[]
}

/** Unchanged lines left out of the hunks; identical on both sides. */
export interface Gap {
  old_start: number
  new_start: number
  /** At least 1. */
  len: number
}

export type Row = ContextRow | RemovedRow | AddedRow | ModifiedRow

/** An unchanged line. */
export interface ContextRow {
  kind: 'context'
  old_line: number
  new_line: number
  text: string
  /** From the new file. */
  syntax?: SyntaxSpan[]
}

export interface RemovedRow {
  kind: 'removed'
  old: Line
}

export interface AddedRow {
  kind: 'added'
  new: Line
}

/** A removed line beside the added line it became. */
export interface ModifiedRow {
  kind: 'modified'
  old: Line
  new: Line
}

/** One side of a changed row. */
export interface Line {
  /** Line number on its side. */
  line: number
  /** No terminator, no trailing `\r`; invalid UTF-8 shown as U+FFFD. */
  text: string
  /** What changed within the line. Sorted, never overlapping. */
  changes?: ChangeSpan[]
  /** From this side's file. */
  syntax?: SyntaxSpan[]
}

export interface ChangeSpan {
  /** UTF-16 offset within the line's `text`. */
  start: number
  end: number
  /**
   * `novel`: syntax only on this side; `novel_word`: a changed word in an
   * otherwise matched line; `unchanged_context`: unchanged syntax inside a
   * changed line; `renamed`: an identifier matched as renamed.
   */
  intensity: 'novel' | 'novel_word' | 'unchanged_context' | 'renamed'
}

export type SyntaxClass =
  | 'keyword'
  | 'string'
  | 'comment'
  | 'number'
  | 'type'
  | 'function'
  | 'operator'
  | 'punctuation'
  | 'variable'
  | 'constant'
  | 'builtin'
  | 'attribute'
  | 'tag'
  | 'property'
  | 'namespace'
  | 'label'
  | 'preprocessor'

export interface SyntaxSpan {
  /** UTF-16 offset within the row's `text`. Sorted, never overlapping. */
  start: number
  end: number
  /** May gain values without a version bump; treat unknown ones as plain text. */
  class: SyntaxClass | (string & {})
}

export interface OutlineItem {
  /** Which file `line` refers to: `removed` items are old, the rest new. */
  side: 'old' | 'new'
  /** The item's first line. */
  line: number
  /** The item's last line, inclusive. */
  end_line?: number
  /** The grammar's node kind, e.g. `function_item`. */
  kind: string
  name: string
  /** `renamed`: only identifiers inside it were renamed. */
  change: 'added' | 'removed' | 'renamed' | 'moved' | 'edited'
  /** Changed lines of an `edited` item. */
  edits?: number
}

/** `old_len` lines from old line `old_start` moved to `new_len` lines from new line `new_start`. */
export interface Move {
  old_start: number
  old_len: number
  new_start: number
  new_len: number
  /** Name of the item the code left. */
  from?: string
  /** Name of the item the code arrived in. */
  into?: string
  /** Name of the moved item itself, when the code is one. */
  item?: string
}

export interface Rename {
  old: string
  new: string
  /** Occurrences renamed. */
  uses: number
}
