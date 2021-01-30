//! Macro to generate completion-based async functions and blocks. This crate shouldn't be used
//! directly, instead use `completion`.

use proc_macro::TokenStream as TokenStream1;
use proc_macro2::{Group, Literal, Punct, Span, TokenStream, TokenTree};
use quote::{quote, quote_spanned, ToTokens, TokenStreamExt};
use syn::parse::{self, Parse, ParseStream, Parser};
use syn::punctuated::Punctuated;
use syn::visit_mut::{self, VisitMut};
use syn::{
    token, AttrStyle, Attribute, Block, Expr, ExprAsync, Path, Signature, Stmt, Token, Visibility,
};

mod block;
mod function;
mod stream;

#[proc_macro_attribute]
pub fn completion(attr: TokenStream1, input: TokenStream1) -> TokenStream1 {
    let (CompletionAttr { crate_path, boxed }, input) = match (syn::parse(attr), syn::parse(input))
    {
        (Ok(attr), Ok(input)) => (attr, input),
        (Ok(_), Err(e)) | (Err(e), Ok(_)) => return e.into_compile_error().into(),
        (Err(mut e1), Err(e2)) => {
            e1.combine(e2);
            return e1.into_compile_error().into();
        }
    };
    match input {
        CompletionInput::AsyncFn(f) => function::transform(f, boxed, &crate_path),
        CompletionInput::AsyncBlock(async_block, semi) => {
            let tokens = block::transform(async_block, &crate_path);
            quote!(#tokens #semi)
        }
    }
    .into()
}

struct CompletionAttr {
    crate_path: CratePath,
    boxed: Option<Boxed>,
}
impl Parse for CompletionAttr {
    fn parse(input: ParseStream<'_>) -> parse::Result<Self> {
        let mut crate_path = None;
        let mut boxed = None;

        while !input.is_empty() {
            if input.peek(Token![crate]) {
                if crate_path.is_some() {
                    return Err(input.error("duplicate crate option"));
                }
                input.parse::<Token![crate]>()?;
                input.parse::<Token![=]>()?;
                crate_path = Some(
                    input
                        .parse::<Path>()?
                        .into_token_stream()
                        .into_iter()
                        .map(|mut token| {
                            token.set_span(Span::call_site());
                            token
                        })
                        .collect(),
                );
            } else if input.peek(Token![box]) {
                if boxed.is_some() {
                    return Err(input.error("duplicate boxed option"));
                }
                let span = input.parse::<Token![box]>()?.span;
                let send = input.peek(token::Paren);
                if send {
                    let content;
                    syn::parenthesized!(content in input);
                    content.parse::<Token![?]>()?;
                    syn::custom_keyword!(Send);
                    content.parse::<Send>()?;
                }
                boxed = Some(Boxed { span, send });
            } else {
                return Err(input.error("expected `crate` or `box`"));
            }

            if input.is_empty() {
                break;
            }

            input.parse::<Token![,]>()?;
        }

        Ok(Self {
            crate_path: CratePath::new(crate_path.unwrap_or_else(|| quote!(::completion))),
            boxed,
        })
    }
}

struct Boxed {
    span: Span,
    send: bool,
}

/// Input to the `#[completion]` attribute macro.
enum CompletionInput {
    AsyncFn(AnyFn),
    AsyncBlock(ExprAsync, Option<Token![;]>),
}
impl Parse for CompletionInput {
    fn parse(input: ParseStream<'_>) -> parse::Result<Self> {
        let mut attrs = input.call(Attribute::parse_outer)?;

        Ok(
            if input.peek(Token![async]) && (input.peek2(Token![move]) || input.peek2(token::Brace))
            {
                let mut block: ExprAsync = input.parse()?;
                block.attrs.append(&mut attrs);
                CompletionInput::AsyncBlock(block, input.parse()?)
            } else {
                let mut f: AnyFn = input.parse()?;
                f.attrs.append(&mut attrs);
                CompletionInput::AsyncFn(f)
            },
        )
    }
}

/// Any kind of function.
struct AnyFn {
    attrs: Vec<Attribute>,
    vis: Visibility,
    sig: Signature,
    block: Option<Block>,
    semi_token: Option<Token![;]>,
}
impl Parse for AnyFn {
    fn parse(input: ParseStream<'_>) -> parse::Result<Self> {
        let mut attrs = input.call(Attribute::parse_outer)?;
        let vis: Visibility = input.parse()?;
        let sig: Signature = input.parse()?;

        let (block, semi_token) = if input.peek(Token![;]) {
            (None, Some(input.parse::<Token![;]>()?))
        } else {
            let content;
            let brace_token = syn::braced!(content in input);
            attrs.append(&mut content.call(Attribute::parse_inner)?);
            let stmts = content.call(Block::parse_within)?;
            (Some(Block { brace_token, stmts }), None)
        };

        Ok(Self {
            attrs,
            vis,
            sig,
            block,
            semi_token,
        })
    }
}
impl ToTokens for AnyFn {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.append_all(
            self.attrs
                .iter()
                .filter(|attr| matches!(attr.style, AttrStyle::Outer)),
        );
        self.vis.to_tokens(tokens);
        self.sig.to_tokens(tokens);
        if let Some(block) = &self.block {
            block.brace_token.surround(tokens, |tokens| {
                tokens.append_all(
                    self.attrs
                        .iter()
                        .filter(|attr| matches!(attr.style, AttrStyle::Inner(_))),
                );
                tokens.append_all(&block.stmts);
            });
        }
        if let Some(semi_token) = &self.semi_token {
            semi_token.to_tokens(tokens);
        }
    }
}

#[proc_macro]
#[doc(hidden)]
pub fn completion_async_inner(input: TokenStream1) -> TokenStream1 {
    completion_async_inner2(input.into(), false).into()
}
#[proc_macro]
#[doc(hidden)]
pub fn completion_async_move_inner(input: TokenStream1) -> TokenStream1 {
    completion_async_inner2(input.into(), true).into()
}

fn completion_async_inner2(input: TokenStream, capture_move: bool) -> TokenStream {
    let (crate_path, stmts) = match parse_bang_input.parse2(input) {
        Ok(input) => input,
        Err(e) => return e.into_compile_error(),
    };
    block::transform(call_site_async(capture_move, stmts), &crate_path)
}

#[proc_macro]
#[doc(hidden)]
pub fn completion_stream_inner(input: TokenStream1) -> TokenStream1 {
    let (crate_path, stmts) = match parse_bang_input.parse(input) {
        Ok(r) => r,
        Err(e) => return e.into_compile_error().into(),
    };
    stream::transform(call_site_async(true, stmts), &crate_path).into()
}

fn parse_bang_input(input: ParseStream<'_>) -> parse::Result<(CratePath, Vec<Stmt>)> {
    let crate_path = CratePath::new(input.parse::<Group>().unwrap().stream());
    let item = Block::parse_within(input)?;
    Ok((crate_path, item))
}

/// Create an async block at the call site.
fn call_site_async(capture_move: bool, stmts: Vec<Stmt>) -> ExprAsync {
    ExprAsync {
        attrs: Vec::new(),
        async_token: Token![async](Span::call_site()),
        capture: if capture_move {
            Some(Token![move](Span::call_site()))
        } else {
            None
        },
        block: Block {
            brace_token: token::Brace {
                span: Span::call_site(),
            },
            stmts,
        },
    }
}

struct CratePath {
    inner: TokenStream,
}
impl CratePath {
    fn new(inner: TokenStream) -> Self {
        Self { inner }
    }
    fn with_span(&self, span: Span) -> impl ToTokens + '_ {
        struct CratePathWithSpan<'a>(&'a TokenStream, Span);

        impl ToTokens for CratePathWithSpan<'_> {
            fn to_tokens(&self, tokens: &mut TokenStream) {
                tokens.extend(self.0.clone().into_iter().map(|mut token| {
                    token.set_span(token.span().located_at(self.1));
                    token
                }));
            }
        }

        CratePathWithSpan(&self.inner, span)
    }
}

/// Transform the top level of a list of statements.
fn transform_top_level(stmts: &mut [Stmt], crate_path: &CratePath, f: impl FnMut(&mut Expr)) {
    struct Visitor<'a, F> {
        crate_path: &'a CratePath,
        f: F,
    }

    impl<F: FnMut(&mut Expr)> VisitMut for Visitor<'_, F> {
        fn visit_expr_mut(&mut self, expr: &mut Expr) {
            match expr {
                Expr::Async(_) | Expr::Closure(_) => {
                    // Don't do anything, we don't want to touch inner async blocks or closures.
                }
                Expr::Macro(expr_macro) => {
                    // Normally we don't transform the bodies of macros as they could do anything
                    // with the tokens they're given. However we special-case standard library
                    // macros to allow.

                    const SPECIAL_MACROS: &[&str] = &[
                        "assert",
                        "assert_eq",
                        "assert_ne",
                        "dbg",
                        "debug_assert",
                        "debug_assert_eq",
                        "debug_assert_ne",
                        "eprint",
                        "eprintln",
                        "format",
                        "format_args",
                        "matches",
                        "panic",
                        "print",
                        "println",
                        "todo",
                        "unimplemented",
                        "unreachable",
                        "vec",
                        "write",
                        "writeln",
                    ];

                    let mut is_trusted =
                        token_stream_starts_with(expr_macro.mac.path.to_token_stream(), {
                            let crate_path = self.crate_path.with_span(Span::call_site());
                            quote!(#crate_path::__special_macros::)
                        });

                    if !is_trusted
                        && SPECIAL_MACROS
                            .iter()
                            .any(|name| expr_macro.mac.path.is_ident(name))
                    {
                        let macro_ident = expr_macro.mac.path.get_ident().unwrap();
                        let crate_path = self.crate_path.with_span(macro_ident.span());
                        let path = quote_spanned!(macro_ident.span()=> #crate_path::__special_macros::#macro_ident);
                        expr_macro.mac.path = syn::parse2(path).unwrap();
                        is_trusted = true;
                    }

                    if is_trusted {
                        let last_segment = expr_macro.mac.path.segments.last().unwrap();

                        match &*last_segment.ident.to_string() {
                            "matches" => {
                                let res =
                                    expr_macro.mac.parse_body_with(|tokens: ParseStream<'_>| {
                                        let expr = tokens.parse::<Expr>()?;
                                        let rest = tokens.parse::<TokenStream>()?;
                                        Ok((expr, rest))
                                    });
                                if let Ok((mut scrutinee, rest)) = res {
                                    self.visit_expr_mut(&mut scrutinee);
                                    expr_macro.mac.tokens = scrutinee.into_token_stream();
                                    expr_macro.mac.tokens.extend(rest.into_token_stream());
                                }
                            }
                            _ => {
                                let res = expr_macro
                                    .mac
                                    .parse_body_with(<Punctuated<_, Token![,]>>::parse_terminated);
                                if let Ok(mut exprs) = res {
                                    for expr in &mut exprs {
                                        self.visit_expr_mut(expr);
                                    }
                                    expr_macro.mac.tokens = exprs.into_token_stream();
                                }
                            }
                        }
                    }
                }
                _ => {
                    visit_mut::visit_expr_mut(self, expr);
                }
            }
            (self.f)(expr);
        }
        fn visit_item_mut(&mut self, _: &mut syn::Item) {
            // Don't do anything, we don't want to touch inner items.
        }
    }

    let mut visitor = Visitor { crate_path, f };
    for stmt in stmts {
        visitor.visit_stmt_mut(stmt);
    }
}

fn token_stream_starts_with(tokens: TokenStream, prefix: TokenStream) -> bool {
    let mut tokens = tokens.into_iter();

    for prefix_token in prefix {
        let token = match tokens.next() {
            Some(token) => token,
            None => return false,
        };
        if !token_tree_eq(&prefix_token, &token) {
            return false;
        }
    }

    true
}

fn token_stream_eq(lhs: TokenStream, rhs: TokenStream) -> bool {
    lhs.into_iter()
        .zip(rhs)
        .all(|(lhs, rhs)| token_tree_eq(&lhs, &rhs))
}
fn token_tree_eq(lhs: &TokenTree, rhs: &TokenTree) -> bool {
    match (lhs, rhs) {
        (TokenTree::Group(lhs), TokenTree::Group(rhs)) => group_eq(lhs, rhs),
        (TokenTree::Ident(lhs), TokenTree::Ident(rhs)) => lhs == rhs,
        (TokenTree::Punct(lhs), TokenTree::Punct(rhs)) => punct_eq(lhs, rhs),
        (TokenTree::Literal(lhs), TokenTree::Literal(rhs)) => literal_eq(lhs, rhs),
        (_, _) => false,
    }
}
fn group_eq(lhs: &Group, rhs: &Group) -> bool {
    lhs.delimiter() == rhs.delimiter() && token_stream_eq(lhs.stream(), rhs.stream())
}
fn punct_eq(lhs: &Punct, rhs: &Punct) -> bool {
    lhs.as_char() == rhs.as_char() && lhs.spacing() == rhs.spacing()
}
fn literal_eq(lhs: &Literal, rhs: &Literal) -> bool {
    lhs.to_string() == rhs.to_string()
}

struct OuterAttrs<'a>(&'a [Attribute]);
impl ToTokens for OuterAttrs<'_> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.append_all(
            self.0
                .iter()
                .filter(|attr| matches!(attr.style, AttrStyle::Outer)),
        )
    }
}
