//! Macro to generate completion-based async functions and blocks. This crate shouldn't be used
//! directly, instead use `completion`.

use proc_macro::TokenStream as TokenStream1;
use proc_macro2::{Group, Span, TokenStream};
use quote::{quote, ToTokens, TokenStreamExt};
use syn::parse::{self, Parse, ParseStream, Parser};
use syn::{
    token, AttrStyle, Attribute, Block, ExprAsync, Path, Signature, Stmt, Token, Visibility,
};

mod function;
mod transform_async;

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
            let tokens = transform_async::transform(async_block, false, &crate_path);
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
                crate_path = Some(input.parse::<Path>()?.into_token_stream());
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
    transform_async::transform(call_site_async(capture_move, stmts), false, &crate_path)
}

#[proc_macro]
#[doc(hidden)]
pub fn completion_stream_inner(input: TokenStream1) -> TokenStream1 {
    let (crate_path, stmts) = match parse_bang_input.parse(input) {
        Ok(r) => r,
        Err(e) => return e.into_compile_error().into(),
    };
    transform_async::transform(call_site_async(true, stmts), true, &crate_path).into()
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
