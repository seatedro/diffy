# diffy-diff

The diff model behind [Diffy](../../README.md)'s diff view, with no UI
dependencies:

- `parse_unified` reads unified diffs (git's format with renames, copies,
  modes, and binary markers, or plain `diff -u`); `write_unified` writes
  git's format; `apply` applies a file diff.
- `diff_texts` diffs two texts by line with Myers' algorithm;
  `inline_diff` finds the changed words of a changed line pair.
- `DiffDocument` stores files, hunks, and blocks as column tables;
  `Projection` turns it into unified or side-by-side display rows.

`diffy-ui`'s `diff_view` renders it.
