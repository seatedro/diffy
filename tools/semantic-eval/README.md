# semantic-eval

Measures semantic diff engines against real history and hand-checked
fixtures, so a replacement for difftastic is judged on data. Build it in
release mode; debug timings are meaningless.

```sh
cargo build --release -p semantic-eval
target/release/semantic-eval corpus --repo ../some-repo --commits 300 --phases
target/release/semantic-eval fixtures
target/release/semantic-eval mutate --repo ../some-repo --ext rs,py
target/release/semantic-eval show --repo ../some-repo --commit <sha> src/lib.rs
```

## corpus

Replays the first-parent, non-merge history of `--rev` (default `HEAD`):
each modified or renamed text file whose extension has a grammar pack, old
side from the parent, through every `--engines` engine (default
`difftastic,line`), one file at a time.

- Times are per file, warm: the first file of each engine and extension is
  reported separately as `coldms` (grammar loading).
- `--phases` reruns itself with `DFT_TIMINGS` set and attributes
  difftastic's phase timings to each file.
- `--jsonl FILE` writes one record per file and engine.

Columns: `fall%` files shown as a line diff, `novel%` share of changed
non-space bytes marked as changed, `frag` marked fragments per marked line
(scattered marks read worse), `unmark` changed lines with no marks on a
structural file, `viol` files breaking an invariant.

Invariants: `DiffDocument::verify_integrity`, aligned rows strictly
increasing per hunk, and rows tiling every change block in order (what the
view requires before it uses structural alignment).

`novel%` is not a quality score on its own: false matches lower it too.
Judge quality with the fixtures.

## fixtures

Each `fixtures/<name>.<ext>.txt` holds `<<<< old`, `<<<< new` and one or
more `<<<< expect` sections. Each `expect` is an acceptable rendering,
checked by hand, not any engine's output; an engine passes when it matches
any of them. In it `⟦…⟧` marks novel text, `⟨…⟩` changed words
inside a string or comment, `<`/`>` a paired row, and `-`/`+` an unpaired
one.

`--bless ENGINE` fills empty expectations with that engine's output, and
`--force` overwrites the existing ones too; review the result before
committing it.

## mutate

Applies edits with a known reading to the tracked files of `--repo` and
checks the native engine reports exactly that edit, in both directions:

- `rename`/`rename`: an identifier renamed at every use is a rename, with
  every changed line paired and nothing else marked.
- `swap`: two adjacent functions swapped is one move.
- `reflow`/`join`: a call's arguments split over lines, or joined back,
  marks nothing and folds.
- `wrap`/`unwrap`: statements wrapped in an `if` mark only the wrapper
  lines and pair every inner line with its original.

Edits that add parse errors count as `invalid` and are skipped.
`--out DIR` writes each failure as a fixture with an empty expectation.
