//! Compile-and-run integration tests for the `view!` macro.
//!
//! `view!` is duck-typed: it emits calls to whatever `div()`, `text()`,
//! `spacer()`, component constructors, and builder methods are in scope at
//! the call site (in Diffy those live in `src/ui/element.rs`). The `dsl`
//! module below provides a minimal recording implementation of that contract
//! so each lowering rule can be asserted at runtime instead of only as token
//! snapshots (those live in `src/lib.rs` unit tests).

use std::cell::Cell;
use std::rc::Rc;

use halogen::reactive::{Signal, SignalStore};
use halogen_macros::view;

use dsl::*;

#[allow(dead_code)]
mod dsl {
    use std::rc::Rc;

    /// Built element tree node; records the tag, builder calls in order,
    /// the slot it was assigned to (for component child slots), and children.
    pub struct AnyElement {
        pub tag: &'static str,
        pub value: Option<String>,
        pub calls: Vec<String>,
        pub slot: Option<&'static str>,
        pub children: Vec<AnyElement>,
        pub on_click: Option<Rc<dyn Fn()>>,
    }

    impl AnyElement {
        fn new(tag: &'static str) -> Self {
            AnyElement {
                tag,
                value: None,
                calls: Vec::new(),
                slot: None,
                children: Vec::new(),
                on_click: None,
            }
        }
    }

    pub struct Div {
        el: AnyElement,
    }

    pub fn div() -> Div {
        Div {
            el: AnyElement::new("div"),
        }
    }

    impl Div {
        pub fn child(mut self, child: AnyElement) -> Self {
            self.el.children.push(child);
            self
        }

        pub fn optional_child(mut self, child: Option<AnyElement>) -> Self {
            if let Some(child) = child {
                self.el.children.push(child);
            }
            self
        }

        pub fn children(mut self, children: impl IntoIterator<Item = AnyElement>) -> Self {
            self.el.children.extend(children);
            self
        }

        pub fn gap(mut self, value: f32) -> Self {
            self.el.calls.push(format!("gap({value})"));
            self
        }

        pub fn flex_row(mut self) -> Self {
            self.el.calls.push("flex_row".into());
            self
        }

        pub fn flex_grow(mut self) -> Self {
            self.el.calls.push("flex_grow".into());
            self
        }

        pub fn flex_shrink_0(mut self) -> Self {
            self.el.calls.push("flex_shrink_0".into());
            self
        }

        pub fn px_2(mut self) -> Self {
            self.el.calls.push("px_2".into());
            self
        }

        pub fn on_click(mut self, handler: impl Fn() + 'static) -> Self {
            self.el.calls.push("on_click".into());
            self.el.on_click = Some(Rc::new(handler));
            self
        }

        pub fn into_any(self) -> AnyElement {
            self.el
        }
    }

    pub struct Text {
        el: AnyElement,
    }

    pub fn text(content: impl ToString) -> Text {
        let mut el = AnyElement::new("text");
        el.value = Some(content.to_string());
        Text { el }
    }

    impl Text {
        pub fn color(mut self, value: impl ToString) -> Self {
            self.el.calls.push(format!("color({})", value.to_string()));
            self
        }

        pub fn bold(mut self) -> Self {
            self.el.calls.push("bold".into());
            self
        }

        pub fn mono(mut self) -> Self {
            self.el.calls.push("mono".into());
            self
        }

        pub fn into_any(self) -> AnyElement {
            self.el
        }
    }

    pub struct Spacer {
        el: AnyElement,
    }

    pub fn spacer() -> Spacer {
        Spacer {
            el: AnyElement::new("spacer"),
        }
    }

    impl Spacer {
        pub fn into_any(self) -> AnyElement {
            self.el
        }
    }

    /// Component with one constructor arg (`action`, per
    /// `constructor_arg_order`) plus `Icon`/`Label` value slots.
    pub struct Button {
        el: AnyElement,
    }

    impl Button {
        pub fn new(action: &'static str) -> Self {
            let mut el = AnyElement::new("Button");
            el.calls.push(format!("action({action})"));
            Button { el }
        }

        pub fn tooltip(mut self, value: &'static str) -> Self {
            self.el.calls.push(format!("tooltip({value})"));
            self
        }

        pub fn icon(mut self, value: &'static str) -> Self {
            self.el.calls.push(format!("icon({value})"));
            self
        }

        pub fn label(mut self, value: impl ToString) -> Self {
            self.el.calls.push(format!("label({})", value.to_string()));
            self
        }

        pub fn flex_grow(mut self) -> Self {
            self.el.calls.push("flex_grow".into());
            self
        }

        pub fn into_any(self) -> AnyElement {
            self.el
        }
    }

    /// Component with `Left`/`Right` child slots.
    pub struct Toolbar {
        el: AnyElement,
    }

    impl Toolbar {
        pub fn new() -> Self {
            Toolbar {
                el: AnyElement::new("Toolbar"),
            }
        }

        pub fn compact(mut self) -> Self {
            self.el.calls.push("compact".into());
            self
        }

        pub fn left_child(mut self, mut child: AnyElement) -> Self {
            child.slot = Some("left");
            self.el.children.push(child);
            self
        }

        pub fn right_child(mut self, mut child: AnyElement) -> Self {
            child.slot = Some("right");
            self.el.children.push(child);
            self
        }

        pub fn into_any(self) -> AnyElement {
            self.el
        }
    }
}

/// `cx` contract required by `{@sig}` attributes: any type with a
/// `read<T>(Signal<T>) -> T` method, named `cx` at the call site.
struct Cx<'a> {
    store: &'a SignalStore,
}

impl Cx<'_> {
    fn read<T: 'static + Clone>(&self, signal: Signal<T>) -> T {
        self.store.read(signal)
    }
}

#[test]
fn basic_element_emit() {
    let el = view! {
        <div flex_row gap={4.0}>
            <text color="red">"hello"</text>
            <spacer />
        </div>
    };

    assert_eq!(el.tag, "div");
    assert_eq!(el.calls, ["flex_row", "gap(4)"]);
    assert_eq!(el.children.len(), 2);
    assert_eq!(el.children[0].tag, "text");
    assert_eq!(el.children[0].value.as_deref(), Some("hello"));
    assert_eq!(el.children[0].calls, ["color(red)"]);
    assert_eq!(el.children[1].tag, "spacer");
}

#[test]
fn nested_children_expression_forms_and_fragment() {
    fn badge(n: usize) -> AnyElement {
        text(format!("badge-{n}")).into_any()
    }

    let extras = vec![badge(1), badge(2)];
    let present: Option<AnyElement> = Some(badge(3));
    let absent: Option<AnyElement> = None;

    let el = view! {
        <div>
            <div flex_row>
                {badge(0)}
            </div>
            {?present}
            {?absent}
            {...extras}
            <fragment>
                <text>"a"</text>
                <text>"b"</text>
            </fragment>
        </div>
    };

    // nested div + present optional + 2 spread + 2 fragment children, with the
    // fragment flattened into the parent (no wrapper node).
    let kinds: Vec<&str> = el
        .children
        .iter()
        .map(|c| c.value.as_deref().unwrap_or(c.tag))
        .collect();
    assert_eq!(kinds, ["div", "badge-3", "badge-1", "badge-2", "a", "b"]);
    assert_eq!(el.children[0].children[0].value.as_deref(), Some("badge-0"));
}

#[test]
fn if_without_else_is_optional_child() {
    let make = |cond: bool| {
        view! {
            <div>
                if cond { <text>"shown"</text> }
            </div>
        }
    };

    assert_eq!(make(true).children.len(), 1);
    assert_eq!(make(true).children[0].value.as_deref(), Some("shown"));
    assert!(make(false).children.is_empty());
}

#[test]
fn if_else_and_else_if_chain_pick_one_branch() {
    let pick = |a: bool, b: bool| {
        view! {
            <div>
                if a { <text>"a"</text> }
                else if b { <text>"b"</text> }
                else { <text>"c"</text> }
            </div>
        }
    };

    assert_eq!(pick(true, false).children[0].value.as_deref(), Some("a"));
    assert_eq!(pick(false, true).children[0].value.as_deref(), Some("b"));
    assert_eq!(pick(false, false).children[0].value.as_deref(), Some("c"));
}

#[test]
fn else_if_without_final_else_falls_back_to_spacer() {
    let pick = |a: bool, b: bool| {
        view! {
            <div>
                if a { <text>"a"</text> }
                else if b { <text>"b"</text> }
            </div>
        }
    };

    assert_eq!(pick(false, true).children[0].value.as_deref(), Some("b"));
    assert_eq!(pick(false, false).children[0].tag, "spacer");
}

#[test]
fn multi_child_if_spreads_into_parent_without_wrapper() {
    let make = |cond: bool| {
        view! {
            <div>
                <text>"lead"</text>
                if cond {
                    <text>"x"</text>
                    <text>"y"</text>
                }
                <text>"tail"</text>
            </div>
        }
    };

    let on = make(true);
    let values: Vec<&str> = on
        .children
        .iter()
        .map(|c| c.value.as_deref().unwrap_or(c.tag))
        .collect();
    // Branch children flow inline into the parent — no wrapper div node.
    assert_eq!(values, ["lead", "x", "y", "tail"]);
    assert!(on.children.iter().all(|c| c.tag == "text"));

    let off = make(false);
    let values: Vec<&str> = off
        .children
        .iter()
        .map(|c| c.value.as_deref().unwrap_or(c.tag))
        .collect();
    assert_eq!(values, ["lead", "tail"]);
}

#[test]
fn for_loop_flattens_each_iteration() {
    let items = vec!["a", "b", "c"];

    let el = view! {
        <div>
            for (i, item) in items.iter().enumerate() {
                <text>{format!("{i}:{item}")}</text>
            }
        </div>
    };

    let values: Vec<&str> = el
        .children
        .iter()
        .map(|c| c.value.as_deref().unwrap_or_default())
        .collect();
    assert_eq!(values, ["0:a", "1:b", "2:c"]);
}

#[test]
fn match_arms_emit_child() {
    let render = |n: u8| {
        view! {
            <div>
                match n {
                    0 => view! { <text>"zero"</text> },
                    1 => text("one").into_any(),
                    _ => spacer().into_any(),
                }
            </div>
        }
    };

    assert_eq!(render(0).children[0].value.as_deref(), Some("zero"));
    assert_eq!(render(1).children[0].value.as_deref(), Some("one"));
    assert_eq!(render(7).children[0].tag, "spacer");
}

#[test]
fn component_value_slots_and_constructor_args() {
    let el = view! {
        <Button action={"save"} tooltip={"Save file"} class="grow">
            <Icon>{"disk"}</Icon>
            <Label>"Save"</Label>
        </Button>
    };

    assert_eq!(el.tag, "Button");
    // Constructor arg first, then builder calls in attribute order (with the
    // class lowered to its mapped builder method), then slot calls.
    assert_eq!(
        el.calls,
        [
            "action(save)",
            "tooltip(Save file)",
            "flex_grow",
            "icon(disk)",
            "label(Save)",
        ]
    );
}

#[test]
fn component_child_slots_map_to_repeated_builder_calls() {
    let el = view! {
        <Toolbar compact>
            <Left>
                <spacer />
            </Left>
            <Right>
                <text>"r1"</text>
                <text>"r2"</text>
            </Right>
        </Toolbar>
    };

    assert_eq!(el.tag, "Toolbar");
    assert_eq!(el.calls, ["compact"]);
    let slots: Vec<(&str, &str)> = el
        .children
        .iter()
        .map(|c| {
            (
                c.slot.unwrap_or("none"),
                c.value.as_deref().unwrap_or(c.tag),
            )
        })
        .collect();
    assert_eq!(
        slots,
        [("left", "spacer"), ("right", "r1"), ("right", "r2")]
    );
}

#[test]
fn class_attribute_lowers_to_builder_methods() {
    let el = view! { <div class="flex-row grow shrink-0 px-2" /> };
    assert_eq!(el.calls, ["flex_row", "flex_grow", "flex_shrink_0", "px_2"]);

    let el = view! { <text class="font-bold font-mono">"x"</text> };
    assert_eq!(el.calls, ["bold", "mono"]);
}

#[test]
fn event_handler_attribute_binds_closure() {
    let clicks = Rc::new(Cell::new(0u32));
    let sink = clicks.clone();

    let el = view! {
        <div on_click={move || sink.set(sink.get() + 1)}>
            <text>"button"</text>
        </div>
    };

    assert!(el.calls.iter().any(|call| call == "on_click"));
    let handler = el.on_click.as_ref().unwrap();
    handler();
    handler();
    assert_eq!(clicks.get(), 2);
}

#[test]
fn reactive_attribute_reads_through_cx() {
    let store = SignalStore::default();
    let gap = store.create(4.0f32);
    let label = store.create(String::from("alpha"));
    let cx = Cx { store: &store };

    let build = || {
        view! {
            <div gap={@gap}>
                <text color={@label}>"t"</text>
            </div>
        }
    };

    let el = build();
    assert_eq!(el.calls, ["gap(4)"]);
    assert_eq!(el.children[0].calls, ["color(alpha)"]);

    store.write(gap, 12.0);
    store.write(label, String::from("beta"));

    let el = build();
    assert_eq!(el.calls, ["gap(12)"]);
    assert_eq!(el.children[0].calls, ["color(beta)"]);
}
