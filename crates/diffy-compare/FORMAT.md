# Diffy display format, version 1

The JSON that Diffy's compare produces for clients to render diffs from
(`diffy_compare::wire`, behind the crate's `serde` feature). It is
layout-neutral: one row list renders both split and unified views, and
nothing needs recomputing on the client.

## Versioning

Every document carries `version`. It changes only for a breaking change:
removing or renaming a field or enum value, changing a field's type,
meaning, or coordinates, or making an optional field required. Adding an
optional field is not breaking and keeps the version, so clients must
ignore fields they do not know. The one enum that may gain values without
a bump is the syntax `class`; treat unknown classes as plain text.

## Conventions

- Field names are `snake_case`; enum values are `snake_case` strings.
- Optional fields are omitted when empty: an absent string or object means
  none, an absent array means `[]`, an absent boolean means `false`.
- Line numbers are 1-based.
- Ranges are half-open: `start` is included, `end` is not.
- **Span offsets are UTF-16 code units within the row's `text`**, the way
  JavaScript indexes strings: `text.slice(span.start, span.end)` is the
  span's text.
- `text` is one line as Diffy displays it: no line terminator, no trailing
  `\r` of a CRLF line, and each byte that is not valid UTF-8 shown as
  U+FFFD (`�`). Whether the last line ends with a newline is stated on the
  file (`old_missing_newline`, `new_missing_newline`).

## Document

| Field | Type | |
| --- | --- | --- |
| `version` | number | The format version, `1`. |
| `files` | File[] | In the order the files were requested. |

## File

| Field | Type | |
| --- | --- | --- |
| `status` | `"modified"` \| `"added"` \| `"deleted"` \| `"renamed"` \| `"copied"` | |
| `old_path` | string? | Absent for an added file. |
| `new_path` | string? | Absent for a deleted file. Show `new_path ?? old_path`. |
| `language` | string? | The language tag the path names, as Diffy names grammars: the extension (`rs`, `ts`, `py`) or, for a file without one, its lowercased name (`dockerfile`). Absent when the path names none, or when `fallback.kind` is `no_grammar`. |
| `binary` | boolean? | A binary file has no `hunks` and no `gap_after`. |
| `additions` | number | Added lines. |
| `deletions` | number | Removed lines. |
| `old_missing_newline` | boolean? | The old file's last line has no newline. |
| `new_missing_newline` | boolean? | The new file's last line has no newline. |
| `fallback` | Fallback? | Present when the file got a line diff instead of a semantic one. |
| `hunks` | Hunk[]? | In file order. |
| `gap_after` | Gap? | Unchanged lines after the last hunk. For a file with no hunks (unchanged, or renamed without edits) this is the whole file. |
| `outline` | OutlineItem[]? | Items the semantic diff found changed. |
| `moves` | Move[]? | Code the semantic diff found moved. |
| `renames` | Rename[]? | Identifiers the semantic diff found renamed. |

A file renders as: for each hunk, its `gap_before` (if any), its header,
its rows; then `gap_after` (if any).

### Fallback

| Field | Type | |
| --- | --- | --- |
| `kind` | `"no_grammar"` \| `"grammar_unavailable"` \| `"semantic"` \| `"request_budget"` | `no_grammar`: no language is known for the path. `grammar_unavailable`: the path names a language whose grammar could not load. `semantic`: the semantic diff gave up, see `reason`. `request_budget`: the request's deadline passed before this file was diffed. |
| `reason` | `"too_large"` \| `"parse_errors"` \| `"error_coverage"` \| `"unparsed"` \| `"budget"` \| `"timeout"`? | Present when `kind` is `semantic`. The file was too large; had too many parse errors; parse errors covered too much of it; the parser gave up; the work budget ran out; the time budget ran out. |
| `message` | string | For people, e.g. `"exceeded the semantic time budget"`. Wording may change; branch on `kind` and `reason`. |

Line-diff files still get changed-word spans on `modified` rows.

## Hunk

| Field | Type | |
| --- | --- | --- |
| `old_start`, `old_len`, `new_start`, `new_len` | number | The numbers of a unified diff's `@@ -old_start,old_len +new_start,new_len @@` line. A start is the side's first line in the hunk, or the line before it when that side is empty (`0` at the top of the file). |
| `header` | string? | The item enclosing the hunk's first change, for the hunk's header line: an item name from the semantic outline (`render_preview`), or a scope label from the grammar (`fn render_preview`, `impl Parser`, `class Parser`, `parse()`). Absent when neither is known. |
| `gap_before` | Gap? | Unchanged lines between the previous hunk (or the top of the file) and this one. |
| `rows` | Row[] | In display order; each side's line numbers ascend. |

## Gap

Unchanged lines left out of the hunks. The lines are identical on both
sides; the starts differ when earlier changes shifted lines.

| Field | Type | |
| --- | --- | --- |
| `old_start` | number | First line on the old side. |
| `new_start` | number | First line on the new side. |
| `len` | number | Lines in the gap, at least 1. |

To expand, send the server a Gap: the whole gap, or any sub-range of it
(the first 20 lines: `{old_start, new_start, len: 20}`; the last 20:
`{old_start: old_start + len - 20, new_start: new_start + len - 20, len:
20}`). The server answers with `context` rows (below), with syntax spans
when the request asked for syntax. The answer can be shorter than `len`
when the range runs past the end of the file or no longer matches it.

## Row

A row is one of four kinds, told apart by `kind`.

### `context`

An unchanged line.

| Field | Type | |
| --- | --- | --- |
| `kind` | `"context"` | |
| `old_line` | number | |
| `new_line` | number | |
| `text` | string | |
| `syntax` | SyntaxSpan[]? | From the new file. |

### `removed`, `added`, `modified`

| Kind | Fields |
| --- | --- |
| `removed` | `old`: Line |
| `added` | `new`: Line |
| `modified` | `old`: Line, `new`: Line — a removed line beside the added line it became. |

Split view: `old` goes left, `new` right; a `removed` row leaves the right
empty and an `added` row the left. Unified view: a `modified` row is its
`old` line as removed then its `new` line as added; to read like `git
diff`, show a run of consecutive `modified` rows as all their old lines,
then all their new lines.

### Line

One side of a changed row.

| Field | Type | |
| --- | --- | --- |
| `line` | number | Line number on its side. |
| `text` | string | |
| `changes` | ChangeSpan[]? | What changed within the line. Sorted, never overlapping. |
| `syntax` | SyntaxSpan[]? | From this side's file. |

A `removed` or `added` line without `changes` changed as a whole. A
`modified` line without `changes` has no word-level detail: the lines are
too different to compare word by word, too long, or differ only in what
`text` does not show (a `\r`, an invalid byte, the final newline).

### ChangeSpan

| Field | Type | |
| --- | --- | --- |
| `start`, `end` | number | UTF-16 offsets within the line's `text`. |
| `intensity` | `"novel"` \| `"novel_word"` \| `"unchanged_context"` \| `"renamed"` | `novel`: syntax that exists only on this side (an inserted or deleted expression); `novel_word`: a changed word in an otherwise matched line; `unchanged_context`: unchanged syntax inside a changed line, for a subdued highlight or none; `renamed`: an identifier matched as renamed (see `renames`). |

Line-diff files only use `novel_word`.

### SyntaxSpan

Present only when the request asked for syntax, the file's language has a
grammar, and the side is not too large. Text outside every span is plain.

| Field | Type | |
| --- | --- | --- |
| `start`, `end` | number | UTF-16 offsets within the line's `text`. Sorted, never overlapping. |
| `class` | string | One of `keyword`, `string`, `comment`, `number`, `type`, `function`, `operator`, `punctuation`, `variable`, `constant`, `builtin`, `attribute`, `tag`, `property`, `namespace`, `label`, `preprocessor`. Map each to a theme colour; treat any other value as plain. |

Syntax comes from parsing each whole file, so a line inside a multi-line
string or comment is highlighted as such. Changes and syntax are
independent layers: draw the change background, then the syntax colour.

## OutlineItem

| Field | Type | |
| --- | --- | --- |
| `side` | `"old"` \| `"new"` | Which file `line` refers to: `removed` items are on the old side, the rest on the new. |
| `line` | number | The item's first line. |
| `kind` | string | The grammar's node kind, e.g. `function_item`, `class_declaration`. |
| `name` | string | |
| `change` | `"added"` \| `"removed"` \| `"renamed"` \| `"moved"` \| `"edited"` | `renamed`: only identifiers inside it were renamed. |
| `edits` | number? | Changed lines of an `edited` item. |

New-side items come first, then old-side ones.

## Move

`old_len` lines starting at old line `old_start` moved to `new_len` lines
starting at new line `new_start`.

| Field | Type | |
| --- | --- | --- |
| `old_start`, `old_len`, `new_start`, `new_len` | number | |
| `from` | string? | Name of the item the code left. |
| `into` | string? | Name of the item the code arrived in. |
| `item` | string? | Name of the moved item itself, when the code is one. |

## Rename

| Field | Type | |
| --- | --- | --- |
| `old`, `new` | string | The identifier before and after. |
| `uses` | number | Occurrences renamed. |

## TypeScript

```ts
type Document = { version: 1; files: File[] };

type File = {
  status: "modified" | "added" | "deleted" | "renamed" | "copied";
  old_path?: string;
  new_path?: string;
  language?: string;
  binary?: boolean;
  additions: number;
  deletions: number;
  old_missing_newline?: boolean;
  new_missing_newline?: boolean;
  fallback?: Fallback;
  hunks?: Hunk[];
  gap_after?: Gap;
  outline?: OutlineItem[];
  moves?: Move[];
  renames?: Rename[];
};

type Fallback = {
  kind: "no_grammar" | "grammar_unavailable" | "semantic" | "request_budget";
  reason?: "too_large" | "parse_errors" | "error_coverage" | "unparsed" | "budget" | "timeout";
  message: string;
};

type Hunk = {
  old_start: number;
  old_len: number;
  new_start: number;
  new_len: number;
  header?: string;
  gap_before?: Gap;
  rows: Row[];
};

type Gap = { old_start: number; new_start: number; len: number };

type Row =
  | { kind: "context"; old_line: number; new_line: number; text: string; syntax?: SyntaxSpan[] }
  | { kind: "removed"; old: Line }
  | { kind: "added"; new: Line }
  | { kind: "modified"; old: Line; new: Line };

type Line = { line: number; text: string; changes?: ChangeSpan[]; syntax?: SyntaxSpan[] };

type ChangeSpan = {
  start: number; // UTF-16
  end: number;
  intensity: "novel" | "novel_word" | "unchanged_context" | "renamed";
};

type SyntaxSpan = { start: number; end: number; class: string }; // UTF-16

type OutlineItem = {
  side: "old" | "new";
  line: number;
  kind: string;
  name: string;
  change: "added" | "removed" | "renamed" | "moved" | "edited";
  edits?: number;
};

type Move = {
  old_start: number;
  old_len: number;
  new_start: number;
  new_len: number;
  from?: string;
  into?: string;
  item?: string;
};

type Rename = { old: string; new: string; uses: number };
```

## Example

A Rust file where `render` became `render_preview` and an argument
changed after a line with non-ASCII text, diffed semantically, without
syntax. Lines 1–9 and 14–20 are unchanged and collapsed. `é` is one UTF-16
unit and `😀` two, so the changed `2` starts at 16 (byte 19).

```json
{
  "version": 1,
  "files": [
    {
      "status": "modified",
      "old_path": "src/view.rs",
      "new_path": "src/view.rs",
      "language": "rs",
      "additions": 2,
      "deletions": 2,
      "hunks": [
        {
          "old_start": 10,
          "old_len": 4,
          "new_start": 10,
          "new_len": 4,
          "header": "render_preview",
          "gap_before": { "old_start": 1, "new_start": 1, "len": 9 },
          "rows": [
            { "kind": "context", "old_line": 10, "new_line": 10, "text": "" },
            {
              "kind": "modified",
              "old": {
                "line": 11,
                "text": "fn render() {",
                "changes": [{ "start": 3, "end": 9, "intensity": "renamed" }]
              },
              "new": {
                "line": 11,
                "text": "fn render_preview() {",
                "changes": [{ "start": 3, "end": 17, "intensity": "renamed" }]
              }
            },
            {
              "kind": "modified",
              "old": {
                "line": 12,
                "text": "    draw(\"é😀\", 1);",
                "changes": [{ "start": 16, "end": 17, "intensity": "novel_word" }]
              },
              "new": {
                "line": 12,
                "text": "    draw(\"é😀\", 2);",
                "changes": [{ "start": 16, "end": 17, "intensity": "novel_word" }]
              }
            },
            { "kind": "context", "old_line": 13, "new_line": 13, "text": "}" }
          ]
        }
      ],
      "gap_after": { "old_start": 14, "new_start": 14, "len": 7 },
      "outline": [
        {
          "side": "new",
          "line": 11,
          "kind": "function_item",
          "name": "render_preview",
          "change": "edited",
          "edits": 2
        }
      ],
      "renames": [{ "old": "render", "new": "render_preview", "uses": 1 }]
    },
    {
      "status": "added",
      "new_path": "notes.txt",
      "language": "txt",
      "additions": 1,
      "deletions": 0,
      "new_missing_newline": true,
      "hunks": [
        {
          "old_start": 0,
          "old_len": 0,
          "new_start": 1,
          "new_len": 1,
          "rows": [{ "kind": "added", "new": { "line": 1, "text": "hello" } }]
        }
      ]
    }
  ]
}
```

With syntax requested, rows also carry spans, e.g. the old line 12 above
gains:

```json
"syntax": [
  { "start": 4, "end": 8, "class": "function" },
  { "start": 9, "end": 14, "class": "string" },
  { "start": 16, "end": 17, "class": "constant" }
]
```
