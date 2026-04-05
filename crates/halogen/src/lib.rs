use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::ext::IdentExt;
use syn::{braced, Expr, ExprIf, ExprMatch, Ident, LitStr, Pat, Result, Token};

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
        ChildMode::Child(t) | ChildMode::Optional(t) | ChildMode::Spread(t) => t.into(),
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
    Component(syn::Path),
}

enum Attr {
    Flag(Ident),
    KeyValue(Ident, Expr),
    Class(LitStr),
    IfAttr(Ident, ExprIf),
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
        Tag::Component(p) => {
            let last = p.segments.last().map(|s| s.ident.to_string()).unwrap_or_default();
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

const SPATIAL_ATTRS: &[&str] = &[
    "gap", "gap_x", "gap_y",
    "p", "px", "py", "pt", "pb", "pl", "pr",
    "rounded",
];

impl EmitCtx {
    fn emit_node(&self, node: &Node) -> ChildMode {
        match node {
            Node::Element(el) => ChildMode::Child(self.emit_element(el)),
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
        let body = self.emit_children_fragment(&fl.body_children);
        quote! {
            (#iter).into_iter().map(|#pat| #body).collect::<Vec<_>>()
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

    fn emit_element(&self, el: &ElementNode) -> TokenStream2 {
        match &el.tag {
            Tag::Div => self.emit_div(el),
            Tag::Text => self.emit_text(el),
            Tag::Icon => self.emit_icon(el),
            Tag::Spacer => quote! { spacer() },
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

        for attr in &el.attrs {
            match attr {
                Attr::KeyValue(name, expr) if name == "svg" => svg_expr = Some(expr.clone()),
                Attr::KeyValue(name, expr) if name == "size" => size_expr = Some(expr.clone()),
                other => extra.push(other),
            }
        }

        let svg = svg_expr.map(|e| quote! { #e }).unwrap_or_else(|| quote! { "" });
        let size = size_expr
            .map(|e| quote! { #e })
            .unwrap_or_else(|| quote! { 16.0 });

        let mut chain = quote! { svg_icon(#svg, #size) };
        for attr in &extra {
            match attr {
                Attr::KeyValue(name, expr) => {
                    chain = quote! { #chain.#name(#expr) };
                }
                Attr::Flag(name) => {
                    chain = quote! { #chain.#name() };
                }
                Attr::Class(_) | Attr::IfAttr(_, _) => {}
            }
        }

        quote! { #chain.into_any() }
    }

    fn emit_component(&self, path: &syn::Path, el: &ElementNode) -> TokenStream2 {
        let mut required_args = Vec::new();
        let mut builder_calls = Vec::new();

        for attr in &el.attrs {
            match attr {
                Attr::KeyValue(name, expr) => {
                    if is_constructor_arg(name) {
                        required_args.push(quote! { #expr });
                    } else {
                        builder_calls.push(quote! { .#name(#expr) });
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
            }
        }

        let mut chain = if required_args.is_empty() {
            quote! { #path::new() }
        } else {
            quote! { #path::new(#(#required_args),*) }
        };

        for call in &builder_calls {
            chain = quote! { #chain #call };
        }

        for child in &el.children {
            chain = self.append_child(chain, child);
        }

        quote! { #chain.into_any() }
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
                if self.should_autoscale(name) {
                    let scale = self.scale.as_ref().unwrap();
                    quote! { #chain.#name((#expr * #scale).round()) }
                } else {
                    quote! { #chain.#name(#expr) }
                }
            }
            Attr::Class(lit) => {
                let calls = self.class_to_calls(lit);
                quote! { #chain #(#calls)* }
            }
            Attr::IfAttr(name, if_expr) => {
                self.emit_if_attr(chain, name, if_expr)
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
            Attr::Class(lit) => {
                let calls = self.class_to_calls(lit);
                quote! { #chain #(#calls)* }
            }
            Attr::IfAttr(name, if_expr) => {
                self.emit_if_attr(chain, name, if_expr)
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

fn is_constructor_arg(name: &Ident) -> bool {
    let s = name.to_string();
    matches!(s.as_str(), "action" | "label" | "title" | "value" | "status")
}
