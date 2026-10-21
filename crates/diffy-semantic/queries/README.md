# Semantic queries

`<language>/semantic.scm` tells the semantic diff how to lower a grammar's
tree. `syntax-pack build` ships it in the language's pack, and the app
compiles in the copies here, which win over a pack's.

| Capture | On | Meaning |
|---|---|---|
| `@text` | node | Diffs as one run of text, compared word by word |
| `@delimiter` | token | Brackets a list; `(` `[` `{` `<` pair with their mirror, any other token with itself |
| `@item.<label>` | node | A declaration, labelled `<label>` in the outline (`fn`, `type`, `module`, ...) |
| `@item.name` | node | The item's name; required |
| `@item.key` | node | Extends the item's identity from the name to cover this node (`impl Trait for Type`) |
| `@item.unit` | node | A parent that moves with the item it wraps (Python `decorated_definition`) |
| `@item.lead` | node | A sibling that, directly before an item, belongs to it (attributes, decorators) |

`@text`, `@delimiter`, `@item.unit` and `@item.lead` apply to every node of
the captured kind, whatever the rest of the pattern says. Item patterns run
as a query over each tree.

Each of text, delimiters and items not in `semantic.scm` is derived from the
pack:

- text: kinds captured as `@string*` or `@character*` at the root of an
  unconditional `highlights.scm` pattern;
- delimiters: `()`, `[]` and `{}` when the grammar has the tokens, and `<>`
  when `highlights.scm` captures both as `@punctuation.bracket`;
- items: the parents of `@local.definition.function`/`method` (`fn`),
  `type`/`enum` (`type`) and `namespace`/`module` (`module`) in
  `locals.scm`.
