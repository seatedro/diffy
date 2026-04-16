# halogen

View macro + reactive signal store for building UIs.

## Reactive

`halogen::reactive` is a fine-grained reactive system modeled on SolidJS.
`Signal<T>` is an 8-byte `Copy` handle into a `SignalStore` arena; values
live in slots indexed by `SignalId { index, generation }`. Reads that
happen inside an observer scope are automatically tracked; writes mark
dependents for lazy recomputation.

### Three-state propagation

Every slot in the store is `Clean`, `Check`, or `Dirty`.

- **Clean** — value is up-to-date.
- **Check** — a transitive source changed; value may or may not still
  be correct. Resolved on next read: re-evaluates memo fns, compares to
  prior value, transitions to `Clean` if equal or `Dirty` if different.
- **Dirty** — value is stale. `mark_source_dirty` transitions to
  `Dirty` immediately for the source and BFS-marks all transitive
  subscribers as `Check`.

`any_dirty()` returns true if any slot has been written since the last
`clear_dirty()`. Typical frame loop: read `any_dirty()` to decide
whether to redraw, render, then call `clear_dirty()` so the next frame
starts Clean.

### Memo equality short-circuit

`create_memo` takes `Fn(&SignalStore) -> T` for `T: Clone + PartialEq`.
If a recomputed memo value equals its previous value, dependents stay
`Clean` and don't re-read it. This is the primary perf lever: upstream
writes cascade down the graph only as far as they change anything.

### Store derive

`#[derive(Store)]` on a struct with scalar fields generates a
`FooStore` parallel struct where every field becomes `Signal<T>`, plus
`FooStore::new(&store, Foo { ... })` and `FooStore::new_default(&store)`.
`#[store(flatten)]` on a nested struct field inlines its fields into the
parent's store. Use this to split big `State` structs into
fine-grained signals without hand-writing every field.

## Sharp edges

- **Re-entrant writes panic.** Writing to a signal while already inside
  a mutable borrow of the store (e.g. inside a `.update()` closure that
  reaches back into the store) triggers `RefCell::borrow_mut().expect(...)`.
  We don't have a reactive effect system today; when we add one, effects
  that write during their own execution will need to defer via a
  pending-writes queue.
- **`set_if_changed` requires `PartialEq`.** Self-documenting compile
  error if someone derives `Store` on a type missing `PartialEq` and a
  caller reaches for the dedupe-on-write path.
- **Memos with unstable outputs.** `create_memo` requires `PartialEq`
  precisely because the equality short-circuit is load-bearing. A memo
  whose compute fn returns `Vec<T>` etc. that ~always differs will
  invalidate every frame — prefer memoizing scalar or small-hashable
  derived values.

## View macro

See `halogen_macros::view` for full syntax. In brief:

```rust
view! { scale,                                   // optional auto-scale factor
    <div class="flex-row" gap={Sp::SM} bg={tc.surface}>
        <text color={tc.text}>{label}</text>
        if let Some(x) = opt { <div>...</div> }  // conditional children
        for item in items { <div>{item.name}</div> }
        {raw_expr}                               // inline expression
        {?option_expr}                           // optional child
        {...vec_expr}                            // spread children
    </div>
}
```

### `{@sig}` reactive attribute

`name={@sig}` expands to `.name(cx.read(sig))`. Requires an
`ElementContext` (or any type with a `read<T>(Signal<T>) -> T` method)
named `cx` in scope at the call site.
