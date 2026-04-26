# AGENTS.md

Guidance for agents working in `crates/halogen-macros`. The root `AGENTS.md`
still applies; this node covers the proc-macro boundary that is easy to break
with plausible-looking changes.

## Purpose And Scope

`halogen-macros` owns compile-time parsing and lowering for
`#[derive(Store)]` and `view!`.

It does not own runtime reactivity, scene primitives, layout behavior, app
actions, design tokens, or renderer behavior. It emits Rust method chains that
the `halogen` crate and Diffy's UI builders must provide.

## Related Context

- Root Rust and change-hygiene rules: `../../AGENTS.md`
- Runtime UI/reactivity contracts: `../halogen/AGENTS.md`
- Macro syntax examples and reactive notes: `../halogen/ARCHITECTURE.md`

## Core Contracts

- Keep dependencies limited to proc-macro tooling unless there is a strong
  reason: `syn`, `quote`, and `proc-macro2` are the intended dependency set.
- `#[derive(Store)]` only supports non-generic structs with named fields.
  Tuple structs, unit structs, enums, unions, and generics should fail with
  clear compile errors.
- Store leaves become `::halogen::reactive::Signal<T>`. `#[store(flatten)]`
  maps a named `Foo` field to `FooStore`; `#[store(skip)]` omits the field from
  the generated store.
- Generated stores derive `Clone`, `Copy`, and `Debug`, expose `new` and
  `new_default`, and only generate `snapshot()` when no field is skipped.
- `view!` supports optional `scale,`, built-in tags (`div`, `text`, `icon`,
  `spacer`, `fragment`), component tags, `if` / `else if` / `else`, `for`,
  `match`, raw expressions, optional expressions, and spread expressions.
- Reactive attributes use `name={@signal}` and lower to `cx.read(signal)`.
  Call sites must provide a `cx` with the expected `read` method.
- `class="..."` lowers to builder method calls. It is not CSS and must stay
  aligned with Diffy's builder methods.
- Multi-child `if` branches and fragments must spread children into the parent,
  not wrap them in a bare `div()`. Wrapping changes layout and percentage-size
  resolution.
- Component constructor arguments are intentionally ordered by
  `constructor_arg_order`; unknown attributes become builder calls.
- Component slot tags like `Icon`, `Label`, `Body`, `Left`, and `Right` lower to
  value or child builder methods and do not accept attributes.
- Auto-scaling applies only to known spatial attributes when the optional
  `scale` identifier is supplied.

## Usage Patterns

- Prefer extending the parser AST and code generation deliberately over ad hoc
  token-string manipulation.
- Preserve hygienic internal names like `__halogen_children` and `__w`; avoid
  names that can collide with user bindings unless they are already part of the
  macro convention.
- Add or update token-output tests for every syntax, slot, class mapping, or
  lowering change.
- When adding a new component constructor special case, make the ordering
  explicit in `constructor_arg_order` and test missing/duplicate args if the
  behavior is non-obvious.

## Anti-Patterns

- Do not add runtime behavior to this crate. Proc macros should parse, validate,
  and emit code only.
- Do not assume class names have browser semantics.
- Do not reintroduce wrapper nodes for multi-child control flow.
- Do not broaden macro syntax without compile-error tests or representative
  emission tests.
- Do not hide unsupported Rust shapes behind partial generated code; fail
  clearly at compile time.

## Validation

- Focused macro tests: `cargo test -p halogen-macros`
- Store derive integration: `cargo test -p halogen --test store_derive`
- UI lowering fallout: run `cargo test -p halogen` when macro output changes
  exported contracts.

## Maintenance

Keep syntax facts here and runtime facts in `../halogen/AGENTS.md`. If a macro
change requires a new builder method in Diffy, update the app-side API and tests
in the same change.
