use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{Expr, ExprIf, ExprMatch, Ident, LitStr, Pat, Result, Token, braced};

// ---------------------------------------------------------------------------
// #[derive(Store)] — generate a parallel `XStore` struct where each field is a
// `Signal<T>` handle (or a nested `YStore` for `#[store(flatten)]` fields).
// ---------------------------------------------------------------------------

#[proc_macro_derive(Store, attributes(store))]
pub fn derive_store(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);
    match derive_store_impl(input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

enum FieldKind {
    Leaf,
    Flatten,
    Skip,
}

fn parse_store_field_attr(attrs: &[syn::Attribute]) -> Result<FieldKind> {
    let mut kind: Option<(FieldKind, proc_macro2::Span)> = None;
    for attr in attrs {
        if !attr.path().is_ident("store") {
            continue;
        }
        attr.parse_nested_meta(|m| {
            let (new_kind, label) = if m.path.is_ident("flatten") {
                (FieldKind::Flatten, "flatten")
            } else if m.path.is_ident("skip") {
                (FieldKind::Skip, "skip")
            } else {
                return Err(m.error(
                    "unknown `#[store(...)]` attribute; supported: `flatten`, `skip`",
                ));
            };
            if let Some((_, prev_span)) = kind {
                let mut err = syn::Error::new(
                    m.path.span(),
                    format!("conflicting `#[store({label})]` — field already has a store attribute"),
                );
                err.combine(syn::Error::new(
                    prev_span,
                    "previous `#[store(...)]` attribute here",
                ));
                return Err(err);
            }
            kind = Some((new_kind, m.path.span()));
            Ok(())
        })?;
    }
    Ok(kind.map(|(k, _)| k).unwrap_or(FieldKind::Leaf))
}

fn flatten_store_type(ty: &syn::Type) -> Result<syn::Type> {
    let syn::Type::Path(tp) = ty else {
        return Err(syn::Error::new_spanned(
            ty,
            "`#[store(flatten)]` expects a named struct type (e.g. `Foo` or `path::to::Foo`); \
             arrays, tuples, references, and generics are not supported",
        ));
    };
    let mut new_path = tp.clone();
    let last = new_path.path.segments.last_mut().unwrap();
    last.ident = Ident::new(&format!("{}Store", last.ident), last.ident.span());
    Ok(syn::Type::Path(new_path))
}

fn derive_store_impl(input: syn::DeriveInput) -> Result<TokenStream2> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "`#[derive(Store)]` cannot be applied to a generic type yet; \
             consider wrapping concrete instantiations instead",
        ));
    }

    let name = &input.ident;
    let store_name = Ident::new(&format!("{name}Store"), name.span());
    let vis = &input.vis;

    let fields = match &input.data {
        syn::Data::Struct(s) => match &s.fields {
            syn::Fields::Named(n) => &n.named,
            syn::Fields::Unnamed(_) => {
                return Err(syn::Error::new_spanned(
                    &input.ident,
                    "`#[derive(Store)]` requires a struct with named fields; \
                     tuple structs are not supported",
                ));
            }
            syn::Fields::Unit => {
                return Err(syn::Error::new_spanned(
                    &input.ident,
                    "`#[derive(Store)]` on a unit struct is meaningless — no fields to store",
                ));
            }
        },
        syn::Data::Enum(_) => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "`#[derive(Store)]` does not support enums; \
                 consider making the enum a leaf field of a struct that derives `Store`",
            ));
        }
        syn::Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "`#[derive(Store)]` does not support unions",
            ));
        }
    };

    let mut decls = Vec::new();
    let mut inits = Vec::new();
    let mut snapshot_fields = Vec::new();
    let mut any_skipped = false;

    for field in fields {
        let fname = field.ident.as_ref().unwrap();
        let fty = &field.ty;
        let fvis = &field.vis;

        match parse_store_field_attr(&field.attrs)? {
            FieldKind::Skip => {
                any_skipped = true;
            }
            FieldKind::Leaf => {
                decls.push(quote! {
                    #fvis #fname: ::halogen::reactive::Signal<#fty>,
                });
                inits.push(quote! {
                    #fname: store.create(initial.#fname),
                });
                snapshot_fields.push(quote! {
                    #fname: store.read(self.#fname),
                });
            }
            FieldKind::Flatten => {
                let store_ty = flatten_store_type(fty)?;
                decls.push(quote! {
                    #fvis #fname: #store_ty,
                });
                inits.push(quote! {
                    #fname: <#store_ty>::new(store, initial.#fname),
                });
                snapshot_fields.push(quote! {
                    #fname: self.#fname.snapshot(store),
                });
            }
        }
    }

    // `snapshot()` can only reconstruct the original when every field is
    // represented in the store. If any field is `#[store(skip)]`, we can't
    // fill it in — omit the method.
    let snapshot_impl = if any_skipped {
        quote! {}
    } else {
        quote! {
            /// Read every signal and reconstruct the original plain struct.
            pub fn snapshot(&self, store: &::halogen::reactive::SignalStore) -> #name {
                #name {
                    #(#snapshot_fields)*
                }
            }
        }
    };

    // `new_default` is only callable when the original struct is `Default`.
    // The `where` bound lets the impl compile even when it isn't.
    let new_default_impl = quote! {
        /// Create a store initialized from `Original::default()`.
        pub fn new_default(store: &::halogen::reactive::SignalStore) -> Self
        where
            #name: Default,
        {
            Self::new(store, <#name as Default>::default())
        }
    };

    Ok(quote! {
        #[derive(Clone, Copy, Debug)]
        #vis struct #store_name {
            #(#decls)*
        }

        impl #store_name {
            /// Create a new store by consuming an initial value, allocating
            /// signals for each leaf field in the provided `SignalStore`.
            pub fn new(
                store: &::halogen::reactive::SignalStore,
                initial: #name,
            ) -> Self {
                Self {
                    #(#inits)*
                }
            }

            #new_default_impl
            #snapshot_impl
        }
    })
}

#[proc_macro]
pub fn view(input: TokenStream) -> TokenStream {
    let view_input: ViewInput = match syn::parse(input) {
        Ok(v) => v,
        Err(e) => return e.to_compile_error().into(),
    };
    let ctx = EmitCtx {
        scale: view_input.scale,
    };
    match ctx.emit_node(&view_input.root) {
        ChildMode::Child(t) | ChildMode::Optional(t) => t.into(),
        ChildMode::Spread(t) => quote! { div().children(#t).into_any() }.into(),
    }
}

// ---------------------------------------------------------------------------
// Top-level input: optional `scale,` then a node
// ---------------------------------------------------------------------------

struct ViewInput {
    scale: Option<Ident>,
    root: Node,
}

impl Parse for ViewInput {
    fn parse(input: ParseStream) -> Result<Self> {
        let scale = if input.peek(Ident::peek_any) && input.peek2(Token![,]) {
            let fork = input.fork();
            let ident: Ident = fork.parse()?;
            if ident == "scale" || !ident.to_string().starts_with('<') {
                let ident: Ident = input.parse()?;
                input.parse::<Token![,]>()?;
                Some(ident)
            } else {
                None
            }
        } else {
            None
        };
        let root = input.parse()?;
        Ok(ViewInput { scale, root })
    }
}

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------

enum Node {
    Element(ElementNode),
    Expr(Expr),
    OptionalExpr(Expr),
    SpreadExpr(Expr),
    Text(LitStr),
    IfChain(IfChainNode),
    ForLoop(ForLoopNode),
    MatchExpr(MatchNode),
}

struct IfChainNode {
    cond: Expr,
    then_children: Vec<Node>,
    else_if: Option<Box<IfChainNode>>,
    else_children: Option<Vec<Node>>,
}

struct ForLoopNode {
    pat: Pat,
    iter: Expr,
    body_children: Vec<Node>,
}

struct MatchNode {
    expr: ExprMatch,
}

struct ElementNode {
    tag: Tag,
    attrs: Vec<Attr>,
    children: Vec<Node>,
}

#[derive(Clone)]
enum Tag {
    Div,
    Text,
    Icon,
    Spacer,
    Fragment,
    Component(syn::Path),
}

enum Attr {
    Flag(Ident),
    KeyValue(Ident, Expr),
    /// `name={@sig}` — value is a Signal/Memo, emitted as `cx.read(sig)`.
    /// Requires `cx` to be in scope at the view! call site.
    ReactiveKeyValue(Ident, Expr),
    Class(LitStr),
    IfAttr(Ident, ExprIf),
    When(Expr, Vec<Attr>),
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

impl Parse for Node {
    fn parse(input: ParseStream) -> Result<Self> {
        if input.peek(Token![<]) {
            Ok(Node::Element(input.parse()?))
        } else if input.peek(Token![if]) {
            Ok(Node::IfChain(input.parse()?))
        } else if input.peek(Token![for]) {
            Ok(Node::ForLoop(input.parse()?))
        } else if input.peek(Token![match]) {
            let expr: ExprMatch = input.parse()?;
            Ok(Node::MatchExpr(MatchNode { expr }))
        } else if input.peek(syn::token::Brace) {
            let content;
            braced!(content in input);
            if content.peek(Token![?]) {
                content.parse::<Token![?]>()?;
                let expr: Expr = content.parse()?;
                Ok(Node::OptionalExpr(expr))
            } else if content.peek(Token![...]) {
                content.parse::<Token![...]>()?;
                let expr: Expr = content.parse()?;
                Ok(Node::SpreadExpr(expr))
            } else {
                let expr: Expr = content.parse()?;
                Ok(Node::Expr(expr))
            }
        } else if input.peek(LitStr) {
            Ok(Node::Text(input.parse()?))
        } else {
            Err(input.error("expected <element>, if, for, match, {expr}, or \"string\""))
        }
    }
}

impl Parse for IfChainNode {
    fn parse(input: ParseStream) -> Result<Self> {
        input.parse::<Token![if]>()?;

        let cond: Expr = Expr::parse_without_eager_brace(input)?;

        let then_brace;
        braced!(then_brace in input);
        let then_children = parse_children(&then_brace)?;

        let (else_if, else_children) = if input.peek(Token![else]) {
            input.parse::<Token![else]>()?;
            if input.peek(Token![if]) {
                let nested: IfChainNode = input.parse()?;
                (Some(Box::new(nested)), None)
            } else {
                let else_brace;
                braced!(else_brace in input);
                let children = parse_children(&else_brace)?;
                (None, Some(children))
            }
        } else {
            (None, None)
        };

        Ok(IfChainNode {
            cond,
            then_children,
            else_if,
            else_children,
        })
    }
}

impl Parse for ForLoopNode {
    fn parse(input: ParseStream) -> Result<Self> {
        input.parse::<Token![for]>()?;
        let pat = Pat::parse_multi_with_leading_vert(input)?;
        input.parse::<Token![in]>()?;
        let iter: Expr = Expr::parse_without_eager_brace(input)?;

        let body_brace;
        braced!(body_brace in input);
        let body_children = parse_children(&body_brace)?;

        Ok(ForLoopNode {
            pat,
            iter,
            body_children,
        })
    }
}

fn parse_children(input: ParseStream) -> Result<Vec<Node>> {
    let mut children = Vec::new();
    while !input.is_empty() {
        children.push(input.parse::<Node>()?);
    }
    Ok(children)
}

impl Parse for ElementNode {
    fn parse(input: ParseStream) -> Result<Self> {
        input.parse::<Token![<]>()?;

        let tag = parse_tag(input)?;

        let mut attrs = Vec::new();
        while !input.peek(Token![>]) && !input.peek(Token![/]) {
            attrs.push(input.parse::<Attr>()?);
        }

        if input.peek(Token![/]) {
            input.parse::<Token![/]>()?;
            input.parse::<Token![>]>()?;
            return Ok(ElementNode {
                tag,
                attrs,
                children: Vec::new(),
            });
        }

        input.parse::<Token![>]>()?;

        let mut children = Vec::new();
        while !is_closing_tag(input) {
            children.push(input.parse::<Node>()?);
        }

        parse_closing_tag(input, &tag)?;

        Ok(ElementNode {
            tag,
            attrs,
            children,
        })
    }
}

fn parse_tag(input: ParseStream) -> Result<Tag> {
    let ident: Ident = input.parse()?;
    match ident.to_string().as_str() {
        "div" => Ok(Tag::Div),
        "text" => Ok(Tag::Text),
        "icon" => Ok(Tag::Icon),
        "spacer" => Ok(Tag::Spacer),
        "fragment" => Ok(Tag::Fragment),
        _ => {
            let mut path = syn::Path::from(ident);
            while input.peek(Token![::]) {
                input.parse::<Token![::]>()?;
                let seg: Ident = input.parse()?;
                path.segments.push(seg.into());
            }
            Ok(Tag::Component(path))
        }
    }
}

fn is_closing_tag(input: ParseStream) -> bool {
    input.peek(Token![<]) && input.peek2(Token![/])
}

fn parse_closing_tag(input: ParseStream, open_tag: &Tag) -> Result<()> {
    input.parse::<Token![<]>()?;
    input.parse::<Token![/]>()?;
    let close_ident: Ident = input.parse()?;
    let expected = match open_tag {
        Tag::Div => "div",
        Tag::Text => "text",
        Tag::Icon => "icon",
        Tag::Spacer => "spacer",
        Tag::Fragment => "fragment",
        Tag::Component(p) => {
            let last = p
                .segments
                .last()
                .map(|s| s.ident.to_string())
                .unwrap_or_default();
            if close_ident != last.as_str() {
                return Err(syn::Error::new(
                    close_ident.span(),
                    format!("expected closing tag `{last}`, found `{close_ident}`"),
                ));
            }
            input.parse::<Token![>]>()?;
            return Ok(());
        }
    };
    if close_ident != expected {
        return Err(syn::Error::new(
            close_ident.span(),
            format!("expected closing tag `{expected}`, found `{close_ident}`"),
        ));
    }
    input.parse::<Token![>]>()?;
    Ok(())
}

impl Parse for Attr {
    fn parse(input: ParseStream) -> Result<Self> {
        // @when {condition} { attr1 attr2=val ... }
        if input.peek(Token![@]) {
            input.parse::<Token![@]>()?;
            let kw: Ident = input.parse()?;
            if kw != "when" {
                return Err(syn::Error::new(kw.span(), "expected `when` after `@`"));
            }
            let cond_content;
            braced!(cond_content in input);
            let cond: Expr = cond_content.parse()?;
            let attrs_content;
            braced!(attrs_content in input);
            let mut attrs = Vec::new();
            while !attrs_content.is_empty() {
                attrs.push(attrs_content.parse::<Attr>()?);
            }
            return Ok(Attr::When(cond, attrs));
        }

        let name: Ident = input.parse()?;
        if name == "class" {
            input.parse::<Token![=]>()?;
            let lit: LitStr = input.parse()?;
            Ok(Attr::Class(lit))
        } else if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            if input.peek(syn::token::Brace) {
                let content;
                braced!(content in input);
                if content.peek(Token![if]) {
                    let if_expr: ExprIf = content.parse()?;
                    Ok(Attr::IfAttr(name, if_expr))
                } else if content.peek(Token![@]) {
                    content.parse::<Token![@]>()?;
                    let expr: Expr = content.parse()?;
                    Ok(Attr::ReactiveKeyValue(name, expr))
                } else {
                    let expr: Expr = content.parse()?;
                    Ok(Attr::KeyValue(name, expr))
                }
            } else if input.peek(LitStr) {
                let lit: LitStr = input.parse()?;
                let expr: Expr = syn::parse_quote!(#lit);
                Ok(Attr::KeyValue(name, expr))
            } else {
                let expr: Expr = input.parse()?;
                Ok(Attr::KeyValue(name, expr))
            }
        } else {
            Ok(Attr::Flag(name))
        }
    }
}

// ---------------------------------------------------------------------------
// Code generation
// ---------------------------------------------------------------------------

struct EmitCtx {
    scale: Option<Ident>,
}

enum ChildMode {
    Child(TokenStream2),
    Optional(TokenStream2),
    Spread(TokenStream2),
}

#[derive(Clone, Copy)]
enum ComponentSlotKind {
    Value,
    Child,
}

struct ComponentSlot {
    method: Ident,
    kind: ComponentSlotKind,
}

const SPATIAL_ATTRS: &[&str] = &[
    "gap", "gap_x", "gap_y", "p", "px", "py", "pt", "pb", "pl", "pr", "rounded",
];

impl EmitCtx {
    fn emit_node(&self, node: &Node) -> ChildMode {
        match node {
            Node::Element(el) => match el.tag {
                Tag::Fragment => ChildMode::Spread(self.emit_children_spread(&el.children)),
                _ => ChildMode::Child(self.emit_element(el)),
            },
            Node::Expr(expr) => ChildMode::Child(quote! { #expr }),
            Node::OptionalExpr(expr) => ChildMode::Optional(quote! { #expr }),
            Node::SpreadExpr(expr) => ChildMode::Spread(quote! { #expr }),
            Node::Text(lit) => ChildMode::Child(quote! { #lit }),
            Node::IfChain(chain) => self.emit_if_chain(chain),
            Node::ForLoop(fl) => ChildMode::Spread(self.emit_for_loop(fl)),
            Node::MatchExpr(m) => ChildMode::Child(self.emit_match(m)),
        }
    }

    fn emit_if_chain(&self, chain: &IfChainNode) -> ChildMode {
        let cond = &chain.cond;
        let then_body = self.emit_children_fragment(&chain.then_children);

        let has_else = chain.else_if.is_some() || chain.else_children.is_some();

        if !has_else {
            let tokens = quote! {
                if #cond { Some(#then_body) } else { None }
            };
            ChildMode::Optional(tokens)
        } else {
            let else_branch = if let Some(ref else_if) = chain.else_if {
                match self.emit_if_chain(else_if) {
                    ChildMode::Optional(t) => {
                        quote! { else { (#t).unwrap_or_else(|| spacer().into_any()) } }
                    }
                    ChildMode::Child(t) | ChildMode::Spread(t) => quote! { else { #t } },
                }
            } else if let Some(ref else_children) = chain.else_children {
                let else_body = self.emit_children_fragment(else_children);
                quote! { else { #else_body } }
            } else {
                unreachable!()
            };

            ChildMode::Child(quote! {
                if #cond { #then_body } #else_branch
            })
        }
    }

    fn emit_for_loop(&self, fl: &ForLoopNode) -> TokenStream2 {
        let pat = &fl.pat;
        let iter = &fl.iter;
        let body = self.emit_children_spread(&fl.body_children);
        quote! {
            (#iter)
                .into_iter()
                .flat_map(|#pat| #body)
                .collect::<Vec<_>>()
        }
    }

    fn emit_match(&self, m: &MatchNode) -> TokenStream2 {
        let expr = &m.expr;
        quote! { #expr }
    }

    fn emit_children_fragment(&self, children: &[Node]) -> TokenStream2 {
        if children.len() == 1 {
            match self.emit_node(&children[0]) {
                ChildMode::Child(t) | ChildMode::Optional(t) | ChildMode::Spread(t) => t,
            }
        } else {
            let mut chain = quote! { div() };
            for child in children {
                chain = self.append_child(chain, child);
            }
            quote! { #chain.into_any() }
        }
    }

    fn emit_children_spread(&self, children: &[Node]) -> TokenStream2 {
        let mut stmts = Vec::new();
        for child in children {
            let stmt = match self.emit_node(child) {
                ChildMode::Child(tokens) => quote! {
                    __halogen_children.push(#tokens);
                },
                ChildMode::Optional(tokens) => quote! {
                    if let Some(__halogen_child) = (#tokens) {
                        __halogen_children.push(__halogen_child);
                    }
                },
                ChildMode::Spread(tokens) => quote! {
                    __halogen_children.extend(#tokens);
                },
            };
            stmts.push(stmt);
        }

        quote! {{
            let mut __halogen_children = Vec::new();
            #(#stmts)*
            __halogen_children
        }}
    }

    fn emit_element(&self, el: &ElementNode) -> TokenStream2 {
        match &el.tag {
            Tag::Div => self.emit_div(el),
            Tag::Text => self.emit_text(el),
            Tag::Icon => self.emit_icon(el),
            Tag::Spacer => quote! { spacer() },
            Tag::Fragment => unreachable!("fragments are handled in emit_node"),
            Tag::Component(path) => self.emit_component(path, el),
        }
    }

    fn emit_div(&self, el: &ElementNode) -> TokenStream2 {
        let mut chain = quote! { div() };

        for attr in &el.attrs {
            chain = self.emit_styled_attr(chain, attr);
        }

        for child in &el.children {
            chain = self.append_child(chain, child);
        }

        quote! { #chain.into_any() }
    }

    fn append_child(&self, chain: TokenStream2, child: &Node) -> TokenStream2 {
        match self.emit_node(child) {
            ChildMode::Child(tokens) => quote! { #chain.child(#tokens) },
            ChildMode::Optional(tokens) => quote! { #chain.optional_child(#tokens) },
            ChildMode::Spread(tokens) => quote! { #chain.children(#tokens) },
        }
    }

    fn emit_text(&self, el: &ElementNode) -> TokenStream2 {
        let content = el.children.first().map(|c| match c {
            Node::Expr(e) | Node::OptionalExpr(e) | Node::SpreadExpr(e) => quote! { #e },
            Node::Text(lit) => quote! { #lit },
            _ => quote! { "" },
        });

        let ctor = match content {
            Some(c) => quote! { text(#c) },
            None => quote! { text("") },
        };

        let mut chain = ctor;
        for attr in &el.attrs {
            chain = self.emit_text_attr(chain, attr);
        }

        quote! { #chain.into_any() }
    }

    fn emit_icon(&self, el: &ElementNode) -> TokenStream2 {
        let mut svg_expr = None;
        let mut size_expr = None;
        let mut extra = Vec::new();

        let mut svg_reactive = false;
        let mut size_reactive = false;
        for attr in &el.attrs {
            match attr {
                Attr::KeyValue(name, expr) if name == "svg" => svg_expr = Some(expr.clone()),
                Attr::KeyValue(name, expr) if name == "size" => size_expr = Some(expr.clone()),
                Attr::ReactiveKeyValue(name, expr) if name == "svg" => {
                    svg_expr = Some(expr.clone());
                    svg_reactive = true;
                }
                Attr::ReactiveKeyValue(name, expr) if name == "size" => {
                    size_expr = Some(expr.clone());
                    size_reactive = true;
                }
                other => extra.push(other),
            }
        }

        let svg = match (svg_expr, svg_reactive) {
            (Some(e), true) => quote! { cx.read(#e) },
            (Some(e), false) => quote! { #e },
            (None, _) => quote! { "" },
        };
        let size = match (size_expr, size_reactive) {
            (Some(e), true) => quote! { cx.read(#e) },
            (Some(e), false) => quote! { #e },
            (None, _) => quote! { 16.0 },
        };

        let mut chain = quote! { svg_icon(#svg, #size) };
        for attr in &extra {
            match attr {
                Attr::KeyValue(name, expr) => {
                    chain = quote! { #chain.#name(#expr) };
                }
                Attr::ReactiveKeyValue(name, expr) => {
                    chain = quote! { #chain.#name(cx.read(#expr)) };
                }
                Attr::Flag(name) => {
                    chain = quote! { #chain.#name() };
                }
                Attr::Class(_) | Attr::IfAttr(_, _) | Attr::When(_, _) => {}
            }
        }

        quote! { #chain.into_any() }
    }

    fn emit_component(&self, path: &syn::Path, el: &ElementNode) -> TokenStream2 {
        let constructor_arg_names = constructor_arg_order(path);
        let mut required_args = vec![None; constructor_arg_names.len()];
        let mut builder_calls = Vec::new();
        let mut errors = Vec::new();

        for attr in &el.attrs {
            match attr {
                Attr::KeyValue(name, expr) => {
                    if let Some(index) = constructor_arg_index(path, name) {
                        if required_args[index].is_some() {
                            errors.push(
                                syn::Error::new_spanned(
                                    name,
                                    format!(
                                        "duplicate constructor arg `{}` for component `{}`",
                                        name,
                                        path.segments
                                            .last()
                                            .map(|segment| segment.ident.to_string())
                                            .unwrap_or_default()
                                    ),
                                )
                                .to_compile_error(),
                            );
                        } else {
                            required_args[index] = Some(quote! { #expr });
                        }
                    } else {
                        builder_calls.push(quote! { .#name(#expr) });
                    }
                }
                Attr::ReactiveKeyValue(name, expr) => {
                    if let Some(index) = constructor_arg_index(path, name) {
                        if required_args[index].is_some() {
                            errors.push(
                                syn::Error::new_spanned(
                                    name,
                                    format!(
                                        "duplicate constructor arg `{}` for component `{}`",
                                        name,
                                        path.segments
                                            .last()
                                            .map(|segment| segment.ident.to_string())
                                            .unwrap_or_default()
                                    ),
                                )
                                .to_compile_error(),
                            );
                        } else {
                            required_args[index] = Some(quote! { cx.read(#expr) });
                        }
                    } else {
                        builder_calls.push(quote! { .#name(cx.read(#expr)) });
                    }
                }
                Attr::Flag(name) => {
                    builder_calls.push(quote! { .#name() });
                }
                Attr::Class(lit) => {
                    for call in self.class_to_calls(lit) {
                        builder_calls.push(call);
                    }
                }
                Attr::IfAttr(_, _) => {}
                Attr::When(_, _) => {} // handled in second pass below
            }
        }

        for (index, arg_name) in constructor_arg_names.iter().enumerate() {
            if required_args[index].is_none() {
                errors.push(
                    syn::Error::new_spanned(
                        path,
                        format!(
                            "missing constructor arg `{}` for component `{}`",
                            arg_name,
                            path.segments
                                .last()
                                .map(|segment| segment.ident.to_string())
                                .unwrap_or_default()
                        ),
                    )
                    .to_compile_error(),
                );
            }
        }

        if !errors.is_empty() {
            return quote! {{
                #(#errors)*
                unreachable!()
            }};
        }

        let mut chain = if required_args.is_empty() {
            quote! { #path::new() }
        } else {
            let required_args = required_args.into_iter().flatten();
            quote! { #path::new(#(#required_args),*) }
        };

        for call in &builder_calls {
            chain = quote! { #chain #call };
        }

        // Apply @when directives
        for attr in &el.attrs {
            if let Attr::When(cond, inner_attrs) = attr {
                let mut inner = quote! { __w };
                for a in inner_attrs {
                    match a {
                        Attr::Flag(name) => {
                            inner = quote! { #inner.#name() };
                        }
                        Attr::KeyValue(name, expr) => {
                            inner = quote! { #inner.#name(#expr) };
                        }
                        Attr::ReactiveKeyValue(name, expr) => {
                            inner = quote! { #inner.#name(cx.read(#expr)) };
                        }
                        Attr::Class(lit) => {
                            for call in self.class_to_calls(lit) {
                                inner = quote! { #inner #call };
                            }
                        }
                        _ => {}
                    }
                }
                chain = quote! { { let __w = #chain; if #cond { #inner } else { __w } } };
            }
        }

        for child in &el.children {
            chain = self.emit_component_child(chain, child);
        }

        quote! { #chain.into_any() }
    }

    fn emit_component_child(&self, chain: TokenStream2, child: &Node) -> TokenStream2 {
        let Node::Element(slot_el) = child else {
            return self.append_child(chain, child);
        };
        let Some(slot) = component_slot(&slot_el.tag) else {
            return self.append_child(chain, child);
        };

        if !slot_el.attrs.is_empty() {
            let err = syn::Error::new_spanned(
                tag_tokens(&slot_el.tag),
                "slot tags do not support attributes; use the parent component attributes for builder methods",
            )
            .to_compile_error();
            return quote! {{ #err #chain }};
        }

        match slot.kind {
            ComponentSlotKind::Value => {
                self.emit_component_value_slot(chain, &slot.method, &slot_el.children, &slot_el.tag)
            }
            ComponentSlotKind::Child => {
                self.emit_component_child_slot(chain, &slot.method, &slot_el.children)
            }
        }
    }

    fn emit_component_value_slot(
        &self,
        chain: TokenStream2,
        method: &Ident,
        children: &[Node],
        tag: &Tag,
    ) -> TokenStream2 {
        let [child] = children else {
            let err = syn::Error::new_spanned(
                tag_tokens(tag),
                "value slot tags require exactly one child expression or node",
            )
            .to_compile_error();
            return quote! {{ #err #chain }};
        };

        match self.emit_node(child) {
            ChildMode::Child(tokens) => quote! { #chain.#method(#tokens) },
            ChildMode::Optional(tokens) => quote! {{
                let __halogen_slot = #chain;
                if let Some(__halogen_value) = (#tokens) {
                    __halogen_slot.#method(__halogen_value)
                } else {
                    __halogen_slot
                }
            }},
            ChildMode::Spread(_) => {
                let err = syn::Error::new_spanned(
                    tag_tokens(tag),
                    "value slot tags do not support spread children",
                )
                .to_compile_error();
                quote! {{ #err #chain }}
            }
        }
    }

    fn emit_component_child_slot(
        &self,
        mut chain: TokenStream2,
        method: &Ident,
        children: &[Node],
    ) -> TokenStream2 {
        for child in children {
            chain = match self.emit_node(child) {
                ChildMode::Child(tokens) => quote! { #chain.#method(#tokens) },
                ChildMode::Optional(tokens) => quote! {{
                    let __halogen_slot = #chain;
                    if let Some(__halogen_child) = (#tokens) {
                        __halogen_slot.#method(__halogen_child)
                    } else {
                        __halogen_slot
                    }
                }},
                ChildMode::Spread(tokens) => quote! {
                    (#tokens)
                        .into_iter()
                        .fold(#chain, |__halogen_slot, __halogen_child| {
                            __halogen_slot.#method(__halogen_child)
                        })
                },
            };
        }
        chain
    }

    // -----------------------------------------------------------------------
    // class="..." expansion
    // -----------------------------------------------------------------------

    fn class_to_calls(&self, lit: &LitStr) -> Vec<TokenStream2> {
        let value = lit.value();
        let mut calls = Vec::new();
        for class in value.split_whitespace() {
            calls.push(self.class_token_to_call(class, lit.span()));
        }
        calls
    }

    fn class_token_to_call(&self, class: &str, span: Span) -> TokenStream2 {
        let mapped = match class {
            "shrink-0" => "flex_shrink_0",
            "shrink" => "flex_shrink_0",
            "grow" => "flex_grow",
            "grow-0" => "flex_grow",
            "font-bold" => "bold",
            "font-semibold" => "semibold",
            "font-medium" => "medium",
            "font-mono" => "mono",
            other => {
                let method = Ident::new(&other.replace('-', "_"), span);
                return quote! { .#method() };
            }
        };
        let method = Ident::new(mapped, span);
        quote! { .#method() }
    }

    // -----------------------------------------------------------------------
    // Attribute emission with auto-scale
    // -----------------------------------------------------------------------

    fn emit_styled_attr(&self, chain: TokenStream2, attr: &Attr) -> TokenStream2 {
        match attr {
            Attr::Flag(name) => quote! { #chain.#name() },
            Attr::KeyValue(name, expr) => {
                if let Expr::Tuple(tup) = expr {
                    let elems = &tup.elems;
                    quote! { #chain.#name(#elems) }
                } else if self.should_autoscale(name) {
                    let scale = self.scale.as_ref().unwrap();
                    quote! { #chain.#name((#expr * #scale).round()) }
                } else {
                    quote! { #chain.#name(#expr) }
                }
            }
            Attr::ReactiveKeyValue(name, expr) => {
                if self.should_autoscale(name) {
                    let scale = self.scale.as_ref().unwrap();
                    quote! { #chain.#name((cx.read(#expr) * #scale).round()) }
                } else {
                    quote! { #chain.#name(cx.read(#expr)) }
                }
            }
            Attr::Class(lit) => {
                let calls = self.class_to_calls(lit);
                quote! { #chain #(#calls)* }
            }
            Attr::IfAttr(name, if_expr) => self.emit_if_attr(chain, name, if_expr),
            Attr::When(cond, attrs) => {
                let mut inner = quote! { __w };
                for a in attrs {
                    inner = self.emit_styled_attr(inner, a);
                }
                quote! { { let __w = #chain; if #cond { #inner } else { __w } } }
            }
        }
    }

    fn emit_if_attr(&self, chain: TokenStream2, name: &Ident, if_expr: &ExprIf) -> TokenStream2 {
        let cond = &if_expr.cond;
        let then_stmts = &if_expr.then_branch.stmts;
        let then_val = if then_stmts.len() == 1 {
            let stmt = &then_stmts[0];
            quote! { #stmt }
        } else {
            quote! { { #(#then_stmts)* } }
        };

        let scaled_then = if self.should_autoscale(name) {
            let scale = self.scale.as_ref().unwrap();
            quote! { (#then_val * #scale).round() }
        } else {
            then_val.clone()
        };

        match &if_expr.else_branch {
            Some((_, else_expr)) => {
                let unwrapped_else = unwrap_block_expr(else_expr);
                let scaled_else = if self.should_autoscale(name) {
                    let scale = self.scale.as_ref().unwrap();
                    quote! { (#unwrapped_else * #scale).round() }
                } else {
                    quote! { #unwrapped_else }
                };
                quote! { #chain.#name(if #cond { #scaled_then } else { #scaled_else }) }
            }
            None => {
                quote! { { let __t = #chain; if #cond { __t.#name(#scaled_then) } else { __t } } }
            }
        }
    }

    fn emit_text_attr(&self, chain: TokenStream2, attr: &Attr) -> TokenStream2 {
        match attr {
            Attr::Flag(name) => quote! { #chain.#name() },
            Attr::KeyValue(name, expr) => quote! { #chain.#name(#expr) },
            Attr::ReactiveKeyValue(name, expr) => quote! { #chain.#name(cx.read(#expr)) },
            Attr::Class(lit) => {
                let calls = self.class_to_calls(lit);
                quote! { #chain #(#calls)* }
            }
            Attr::IfAttr(name, if_expr) => self.emit_if_attr(chain, name, if_expr),
            Attr::When(cond, attrs) => {
                let mut inner = quote! { __w };
                for a in attrs {
                    inner = self.emit_text_attr(inner, a);
                }
                quote! { { let __w = #chain; if #cond { #inner } else { __w } } }
            }
        }
    }

    fn should_autoscale(&self, name: &Ident) -> bool {
        if self.scale.is_none() {
            return false;
        }
        let s = name.to_string();
        SPATIAL_ATTRS.contains(&s.as_str())
    }
}

fn unwrap_block_expr(expr: &Expr) -> TokenStream2 {
    if let Expr::Block(block) = expr {
        if block.block.stmts.len() == 1 {
            let stmt = &block.block.stmts[0];
            return quote! { #stmt };
        }
    }
    quote! { #expr }
}

fn constructor_arg_order(path: &syn::Path) -> &'static [&'static str] {
    let Some(last) = path.segments.last() else {
        return &[];
    };
    match last.ident.to_string().as_str() {
        "Button" => &["action"],
        "DropdownItem" => &["label", "action"],
        "TabItem" => &["label", "action"],
        "SegmentedItem" => &["label", "action", "selected"],
        "Modal" => &[
            "title",
            "subtitle",
            "icon",
            "max_width",
            "window_width",
            "window_height",
        ],
        _ => &[],
    }
}

fn constructor_arg_index(path: &syn::Path, name: &Ident) -> Option<usize> {
    let name = name.to_string();
    constructor_arg_order(path)
        .iter()
        .position(|candidate| *candidate == name)
}

fn component_slot(tag: &Tag) -> Option<ComponentSlot> {
    let Tag::Component(path) = tag else {
        return None;
    };
    if path.leading_colon.is_some() || path.segments.len() != 1 {
        return None;
    }

    let ident = &path.segments[0].ident;
    let (method, kind) = match ident.to_string().as_str() {
        "Icon" => ("icon", ComponentSlotKind::Value),
        "Label" => ("label", ComponentSlotKind::Value),
        "Tooltip" => ("tooltip", ComponentSlotKind::Value),
        "Description" => ("description", ComponentSlotKind::Value),
        "Count" => ("count", ComponentSlotKind::Value),
        "Shortcut" => ("shortcut", ComponentSlotKind::Value),
        "Body" => ("body_child", ComponentSlotKind::Child),
        "Footer" => ("footer_child", ComponentSlotKind::Child),
        "Left" => ("left_child", ComponentSlotKind::Child),
        "Right" => ("right_child", ComponentSlotKind::Child),
        _ => return None,
    };

    Some(ComponentSlot {
        method: Ident::new(method, ident.span()),
        kind,
    })
}

fn tag_tokens(tag: &Tag) -> TokenStream2 {
    match tag {
        Tag::Div => quote! { div },
        Tag::Text => quote! { text },
        Tag::Icon => quote! { icon },
        Tag::Spacer => quote! { spacer },
        Tag::Fragment => quote! { fragment },
        Tag::Component(path) => quote! { #path },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emit(input: &str) -> String {
        let view_input: ViewInput = syn::parse_str(input).expect("parse view input");
        let ctx = EmitCtx {
            scale: view_input.scale,
        };
        match ctx.emit_node(&view_input.root) {
            ChildMode::Child(tokens) | ChildMode::Optional(tokens) | ChildMode::Spread(tokens) => {
                tokens.to_string()
            }
        }
    }

    #[test]
    fn component_slots_lower_to_builder_calls() {
        let actual = emit(
            r#"
            <Button action={Action::ShowWorkingTree} active={is_active} tooltip={"Show working tree changes"}>
                <Icon>{lucide::FOLDER_GIT}</Icon>
                <Label>{"Working tree"}</Label>
            </Button>
            "#,
        );

        let expected = quote! {
            Button::new(Action::ShowWorkingTree)
                .active(is_active)
                .tooltip("Show working tree changes")
                .icon(lucide::FOLDER_GIT)
                .label("Working tree")
                .into_any()
        }
        .to_string();

        assert_eq!(actual, expected);
    }

    #[test]
    fn child_slots_expand_to_repeated_child_builder_calls() {
        let actual = emit(
            r#"
            <Toolbar>
                <Left>
                    <Button action={Action::ToggleSidebar} />
                    if show_search {
                        <Button action={Action::CloseSearch} />
                    }
                </Left>
                <Right>
                    for action in actions {
                        <Button action={action} />
                    }
                </Right>
            </Toolbar>
            "#,
        );

        let expected = quote! {
            ((actions)
                .into_iter()
                .flat_map(|action| {
                    let mut __halogen_children = Vec::new();
                    __halogen_children.push(Button::new(action).into_any());
                    __halogen_children
                })
                .collect::<Vec<_>>())
                .into_iter()
                .fold(
                    {
                        let __halogen_slot = Toolbar::new()
                            .left_child(Button::new(Action::ToggleSidebar).into_any());
                        if let Some(__halogen_child) =
                            (if show_search {
                                Some(Button::new(Action::CloseSearch).into_any())
                            } else {
                                None
                            })
                        {
                            __halogen_slot.left_child(__halogen_child)
                        } else {
                            __halogen_slot
                        }
                    },
                    |__halogen_slot, __halogen_child| {
                        __halogen_slot.right_child(__halogen_child)
                    }
                )
                .into_any()
        }
        .to_string();

        assert_eq!(actual, expected);
    }

    #[test]
    fn non_slot_component_children_still_use_child() {
        let actual = emit(
            r#"
            <Parent>
                <Avatar />
                <Button action={Action::ToggleSidebar} />
            </Parent>
            "#,
        );

        let expected = quote! {
            Parent::new()
                .child(Avatar::new().into_any())
                .child(Button::new(Action::ToggleSidebar).into_any())
                .into_any()
        }
        .to_string();

        assert_eq!(actual, expected);
    }

    #[test]
    fn modal_constructor_args_are_ordered_by_signature() {
        let actual = emit(
            r#"
            <Modal window_height={height}
                   title={"Keyboard Shortcuts"}
                   icon={lucide::COMMAND}
                   max_width={max_width}
                   subtitle={"Press ? to dismiss"}
                   window_width={width}
                   gap={Sp::XL}>
                <Body>{body}</Body>
                <Footer>
                    <Button action={Action::CloseOverlay} />
                </Footer>
            </Modal>
            "#,
        );

        let expected = quote! {
            Modal::new(
                "Keyboard Shortcuts",
                "Press ? to dismiss",
                lucide::COMMAND,
                max_width,
                width,
                height
            )
            .gap(Sp::XL)
            .body_child(body)
            .footer_child(Button::new(Action::CloseOverlay).into_any())
            .into_any()
        }
        .to_string();

        assert_eq!(actual, expected);
    }

    #[test]
    fn for_loops_flatten_multiple_children() {
        let actual = emit(
            r#"
            <div>
                for item in items {
                    if show_separators {
                        <text>{"|"}</text>
                    }
                    <text>{item}</text>
                }
            </div>
            "#,
        );

        let expected = quote! {
            div()
                .children(
                    (items)
                        .into_iter()
                        .flat_map(|item| {
                            let mut __halogen_children = Vec::new();
                            if let Some(__halogen_child) =
                                (if show_separators {
                                    Some(text("|").into_any())
                                } else {
                                    None
                                })
                            {
                                __halogen_children.push(__halogen_child);
                            }
                            __halogen_children.push(text(item).into_any());
                            __halogen_children
                        })
                        .collect::<Vec<_>>()
                )
                .into_any()
        }
        .to_string();

        assert_eq!(actual, expected);
    }

    #[test]
    fn fragment_flattens_children_into_parent() {
        let actual = emit(
            r#"
            <div>
                <fragment>
                    <text>{"left"}</text>
                    <text>{"right"}</text>
                </fragment>
            </div>
            "#,
        );

        let expected = quote! {
            div()
                .children({
                    let mut __halogen_children = Vec::new();
                    __halogen_children.push(text("left").into_any());
                    __halogen_children.push(text("right").into_any());
                    __halogen_children
                })
                .into_any()
        }
        .to_string();

        assert_eq!(actual, expected);
    }
}
