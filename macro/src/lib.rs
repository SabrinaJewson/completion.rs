//! Macro to generate completion-based async functions and blocks. This crate shouldn't be used
//! directly, instead use `completion`.

use std::mem;

use proc_macro::TokenStream as TokenStream1;

use proc_macro2::{Delimiter, Group, Ident, Span, TokenStream};
use quote::{quote, quote_spanned, ToTokens, TokenStreamExt};
use syn::parse::{self, Parse, ParseStream, Parser};
use syn::parse_quote;
use syn::punctuated::Punctuated;
use syn::visit_mut::{self, VisitMut};
use syn::MetaNameValue;
use syn::Token;
use syn::{Block, Lifetime, Lit, Receiver, ReturnType, Stmt};
use syn::{Expr, ExprAsync};
use syn::{GenericParam, Generics, LifetimeDef};
use syn::{Item, ItemFn};
use syn::{Type, TypeReference};

#[proc_macro_attribute]
pub fn completion(attr: TokenStream1, input: TokenStream1) -> TokenStream1 {
    match completion2(attr.into(), input.into()) {
        Ok(tokens) => tokens,
        Err(e) => e.into_compile_error(),
    }
    .into()
}

fn completion2(attr: TokenStream, input: TokenStream) -> parse::Result<TokenStream> {
    let opts = Opts::parse_attr.parse2(attr)?;
    let input = syn::parse2(input)?;

    Ok(match input {
        CompletionInput::AsyncFn(f) => transform_async_fn(&opts, f),
        CompletionInput::AsyncBlock(async_block, semi) => {
            let crate_path = &opts.crate_path;

            let stmts = transform_stmts(&opts, None, async_block.block.stmts);
            let async_token = async_block.async_token;
            let capture = async_block.capture;

            quote! {
                #crate_path::__make_completion_future(#async_token #capture {
                    #stmts
                })
                #semi
            }
        }
    })
}

#[test]
fn test_completion() {
    assert_eq!(
        completion2(
            TokenStream::new(),
            quote! {
                async { fut.await }
            }
        )
        .unwrap()
        .to_string(),
        quote! {
            ::completion::__make_completion_future(async {
                #[allow(unused_imports)]
                use ::completion::__CompletionFutureIntoFutureUnsafe;

                ::completion::__FutureOrCompletionFuture(fut).__into_future_unsafe().await
            })
        }
        .to_string(),
    );
    assert_eq!(
        completion2(
            TokenStream::new(),
            quote! {
                async fn foo() {
                    |_| fut.await;
                }
            }
        )
        .unwrap()
        .to_string(),
        quote! {
            fn foo<'__completion_future>() -> impl ::completion::CompletionFuture<Output = ()> + '__completion_future {
                ::completion::__make_completion_future(async move {
                    #[allow(unused_imports)]
                    use ::completion::__CompletionFutureIntoFutureUnsafe;

                    |_| fut.await;
                })
            }
        }
        .to_string(),
    );
    assert_eq!(
        completion2(
            quote!(crate = "::crate::path"),
            quote! {
                pub(super) async fn do_stuff<T: Clone>(&mut self, x: &&T) -> Vec<u8> {
                    fut.await;
                }
            }
        )
        .unwrap()
        .to_string(),
        quote! {
            pub(super) fn do_stuff<'__completion_future, '__life2, '__life1, '__life0, T: Clone>(
                &'__life0 mut self,
                x: &'__life1 &'__life2 T
            ) -> impl ::crate::path::CompletionFuture<Output = Vec<u8> > + '__completion_future
            where
                '__life2: '__completion_future,
                '__life1: '__completion_future,
                '__life0: '__completion_future,
                T: '__completion_future,
                Self: '__completion_future
            {
                ::crate::path::__make_completion_future(async move {
                    #[allow(unused_imports)]
                    use ::crate::path::__CompletionFutureIntoFutureUnsafe;

                    ::crate::path::__FutureOrCompletionFuture(fut).__into_future_unsafe().await;
                })
            }
        }
        .to_string(),
    );
}

/// Input to the `#[completion]` attribute macro.
enum CompletionInput {
    AsyncFn(ItemFn),
    AsyncBlock(ExprAsync, Option<Token![;]>),
}
impl Parse for CompletionInput {
    fn parse(input: ParseStream<'_>) -> parse::Result<Self> {
        let statement: Stmt = input.parse()?;

        Ok(match statement {
            Stmt::Item(Item::Fn(f)) if f.sig.asyncness.is_some() => Self::AsyncFn(f),
            Stmt::Expr(Expr::Async(e)) => Self::AsyncBlock(e, None),
            Stmt::Semi(Expr::Async(e), semi) => Self::AsyncBlock(e, Some(semi)),
            _ => {
                return Err(parse::Error::new_spanned(
                    statement,
                    "Expected async function or block",
                ))
            }
        })
    }
}

/// Transform an `async fn` to a completion async fn.
fn transform_async_fn(opts: &Opts, mut f: ItemFn) -> TokenStream {
    // Remove the `async` from the function's signature.
    let async_span = f.sig.asyncness.take().unwrap().span;

    // Unelide the lifetimes.
    let mut unelider = Unelider::new(&mut f.sig.generics);
    for arg in &mut f.sig.inputs {
        unelider.visit_fn_arg_mut(arg);
    }

    let ret_lifetime = Lifetime::new("'__completion_future", Span::mixed_site());

    // Add `T: '__completion_future` or `'a: '__completion_future` for every generic parameter.
    let where_clause = f
        .sig
        .generics
        .where_clause
        .get_or_insert_with(|| parse_quote!(where));
    for param in &f.sig.generics.params {
        where_clause.predicates.push(match param {
            GenericParam::Type(ty_param) => {
                let ty = &ty_param.ident;
                parse_quote!(#ty: #ret_lifetime)
            }
            GenericParam::Lifetime(def) => {
                let lifetime = &def.lifetime;
                parse_quote!(#lifetime: #ret_lifetime)
            }
            _ => continue,
        });
    }

    // Add `Self: '__completion_future` if the parameters include `Self`.
    let mut has_self = HasSelf(false);
    for input in &mut f.sig.inputs {
        has_self.visit_fn_arg_mut(input);
    }
    if has_self.0 {
        where_clause
            .predicates
            .push(parse_quote!(Self: #ret_lifetime));
    }

    // Add the `'__completion_future` lifetime.
    f.sig
        .generics
        .params
        .insert(0, lifetime_generic(ret_lifetime.clone()));

    // Change the return type to be `impl CompletionFuture<Output = ...> + '__completion_future`.
    let (rarrow, ret_ty) = match f.sig.output {
        ReturnType::Default => (Token![->](async_span), quote_spanned!(async_span=> ())),
        ReturnType::Type(rarrow, ty) => (rarrow, ty.into_token_stream()),
    };

    let crate_path = SetSpanLocation(&opts.crate_path, async_span);
    let ret_ty = quote_spanned! {async_span=>
        impl #crate_path::CompletionFuture<Output = #ret_ty> + #ret_lifetime
    };
    f.sig.output = ReturnType::Type(rarrow, Box::new(Type::Verbatim(ret_ty)));

    // Transform the function body and put it in an async block
    let stmts = transform_stmts(&opts, None, mem::take(&mut f.block.stmts));
    f.block.stmts = vec![Stmt::Expr(Expr::Verbatim(quote_spanned! {async_span=>
        #crate_path::__make_completion_future(async move {
            #stmts
        })
    }))];

    f.into_token_stream()
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
    let (opts, stmts) = match Opts::parse_bang.parse2(input) {
        Ok(r) => r,
        Err(e) => return e.into_compile_error(),
    };

    let stmts = transform_stmts(&opts, None, stmts);
    let move_token = if capture_move {
        Some(<Token![move]>::default())
    } else {
        None
    };
    let crate_path = &opts.crate_path;
    quote! {
        #crate_path::__make_completion_future(async #move_token {
            #stmts
        })
    }
}

#[proc_macro]
#[doc(hidden)]
pub fn completion_stream_inner(input: TokenStream1) -> TokenStream1 {
    completion_stream_inner2(input.into()).into()
}

fn completion_stream_inner2(input: TokenStream) -> TokenStream {
    let (opts, stmts) = match Opts::parse_bang.parse2(input) {
        Ok(r) => r,
        Err(e) => return e.into_compile_error(),
    };

    let yielder = Ident::new("__yielder", Span::mixed_site());
    let stmts = transform_stmts(&opts, Some(yielder.clone()), stmts);

    let crate_path = &opts.crate_path;
    quote! {
        #crate_path::__AsyncStream::new(move |mut #yielder| async move {
            #stmts
        })
    }
}

struct Opts {
    /// The path to `completion`.
    crate_path: TokenStream,
}
impl Opts {
    fn parse_bang(input: ParseStream<'_>) -> parse::Result<(Self, Vec<Stmt>)> {
        let crate_path = input.parse::<Group>().unwrap().stream();
        let item = Block::parse_within(input)?;
        Ok((Self { crate_path }, item))
    }
    fn parse_attr(input: ParseStream<'_>) -> parse::Result<Self> {
        let mut crate_path = None;

        while !input.is_empty() {
            let nv: MetaNameValue = input.parse()?;

            if nv.path.is_ident("crate") {
                if crate_path.is_some() {
                    return Err(parse::Error::new_spanned(nv, "duplicate crate attribute"));
                }
                crate_path = Some(match nv.lit {
                    Lit::Str(s) => s.parse()?,
                    _ => {
                        return Err(parse::Error::new_spanned(
                            nv.lit,
                            "must be a string literal",
                        ))
                    }
                });
            } else {
                return Err(parse::Error::new_spanned(
                    &nv.path,
                    format!(
                        "unknown attribute `{}`; expected `crate`",
                        nv.path.to_token_stream()
                    ),
                ));
            }

            if input.is_empty() {
                break;
            }

            input.parse::<Token![,]>()?;
        }

        Ok(Self {
            crate_path: crate_path.unwrap_or_else(|| quote!(::completion)),
        })
    }
}

/// Transforms the statements inside an async block into the statements of an async block that can
/// await completion futures and yield if it's a stream.
fn transform_stmts(opts: &Opts, yielder: Option<Ident>, stmts: Vec<Stmt>) -> TokenStream {
    let crate_path = &opts.crate_path;
    let mut tokens = quote! {
        #[allow(unused_imports)]
        use #crate_path::__CompletionFutureIntoFutureUnsafe;
    };
    let mut transformer = Transformer {
        opts,
        yielder,
        yielded: false,
    };

    for mut stmt in stmts {
        transformer.visit_stmt_mut(&mut stmt);
        stmt.to_tokens(&mut tokens);
    }

    if let (Some(yielder), false) = (transformer.yielder, transformer.yielded) {
        tokens = quote! {
            if false {
                #yielder.send(()).await;
            }
            #tokens
        };
    }

    tokens
}

/// Allows completion futures to be awaited and yields to be used inside streams.
struct Transformer<'a> {
    opts: &'a Opts,
    yielder: Option<Ident>,
    yielded: bool,
}
impl<'a> VisitMut for Transformer<'a> {
    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        match expr {
            Expr::Async(_) | Expr::Closure(_) => {
                // Don't do anything, we don't want to touch inner async blocks or closures.
            }
            Expr::Await(expr_await) => {
                self.visit_expr_mut(&mut expr_await.base);

                let base = &expr_await.base;
                let await_span = expr_await.await_token.span;
                let crate_path = SetSpanLocation(&self.opts.crate_path, await_span);

                *expr = Expr::Verbatim(quote_spanned! {await_span=>
                    #crate_path::__FutureOrCompletionFuture(#base).__into_future_unsafe().await
                });
            }
            Expr::Macro(expr_macro) => {
                // Normally we don't transform the inner expressions in macros as they could for
                // example put the inner code in a function, which would make this macro unsound.
                // However, we transform the bodies of some macros by special-casing them.
                const SPECIAL_MACROS: &[&str] = &[
                    "assert",
                    "assert_eq",
                    "assert_ne",
                    #[cfg(feature = "std")]
                    "dbg",
                    "debug_assert",
                    "debug_assert_eq",
                    "debug_assert_ne",
                    #[cfg(feature = "std")]
                    "eprint",
                    #[cfg(feature = "std")]
                    "eprintln",
                    #[cfg(feature = "alloc")]
                    "format",
                    "format_args",
                    "matches",
                    "panic",
                    #[cfg(feature = "std")]
                    "print",
                    #[cfg(feature = "std")]
                    "println",
                    "todo",
                    "unimplemented",
                    "unreachable",
                    #[cfg(feature = "alloc")]
                    "vec",
                    "write",
                    "writeln",
                ];

                if let Some(&macro_name) = SPECIAL_MACROS
                    .iter()
                    .find(|name| expr_macro.mac.path.is_ident(name))
                {
                    let succeeded = if macro_name == "matches" {
                        let res = expr_macro.mac.parse_body_with(|tokens: ParseStream<'_>| {
                            let expr = tokens.parse::<Expr>()?;
                            let rest = tokens.parse::<TokenStream>()?;
                            Ok((expr, rest))
                        });
                        if let Ok((mut scrutinee, rest)) = res {
                            self.visit_expr_mut(&mut scrutinee);
                            expr_macro.mac.tokens = scrutinee.into_token_stream();
                            expr_macro.mac.tokens.extend(rest.into_token_stream());
                            true
                        } else {
                            false
                        }
                    } else {
                        let res = expr_macro
                            .mac
                            .parse_body_with(<Punctuated<_, Token![,]>>::parse_terminated);
                        if let Ok(mut exprs) = res {
                            for expr in &mut exprs {
                                self.visit_expr_mut(expr);
                            }
                            expr_macro.mac.tokens = exprs.into_token_stream();
                            true
                        } else {
                            false
                        }
                    };

                    if succeeded {
                        let span = expr_macro.mac.path.get_ident().unwrap().span();
                        let lib = if cfg!(feature = "std") {
                            "std"
                        } else if cfg!(feature = "alloc") && matches!(macro_name, "format" | "vec")
                        {
                            "alloc"
                        } else {
                            "core"
                        };
                        let lib = Ident::new(lib, span);
                        let macro_ident = Ident::new(macro_name, span);

                        expr_macro.mac.path = parse_quote!(#lib::#macro_ident);
                    }
                }
            }
            Expr::Try(expr_try) => {
                if let Some(yielder) = &self.yielder {
                    let question_span = expr_try.question_token.spans[0];
                    let crate_path = SetSpanLocation(&self.opts.crate_path, question_span);
                    let base = &expr_try.expr;

                    *expr = Expr::Verbatim(quote_spanned! {question_span=>
                        match #crate_path::__Try::into_result(#base) {
                            ::core::result::Result::Ok(val) => val,
                            ::core::result::Result::Err(e) => {
                                #[allow(clippy::useless_conversion)]
                                #yielder.send(#crate_path::__Try::from_error(::core::convert::From::from(e))).await;
                                return;
                            }
                        }
                    });
                } else {
                    self.visit_expr_try_mut(expr_try);
                }
            }
            Expr::Yield(expr_yield) => {
                self.yielded = true;

                if let Some(base) = &mut expr_yield.expr {
                    self.visit_expr_mut(base);
                }

                let yield_span = expr_yield.yield_token.span;

                let yielder = match &self.yielder {
                    Some(yielder) => yielder,
                    None => {
                        let base = &expr_yield.expr;
                        *expr = Expr::Verbatim(quote_spanned! {yield_span=> {
                            ::core::compile_error!("`yield` can only be used inside a stream");
                            #base
                        }});
                        return;
                    }
                };

                let unit = Unit(yield_span);
                let base: &dyn ToTokens = match &expr_yield.expr {
                    Some(expr) => &**expr,
                    None => &unit,
                };

                *expr = Expr::Verbatim(quote_spanned! {yield_span=>
                    #yielder.send(#base).await
                });
            }
            expr => {
                visit_mut::visit_expr_mut(self, expr);
            }
        }
    }
    fn visit_item_mut(&mut self, _: &mut Item) {
        // Don't do anything, we don't want to touch inner items.
    }
}

#[test]
fn test_transform_stmts() {
    let opts = Opts {
        crate_path: quote!(some::path),
    };

    let stdlib = if cfg!(feature = "std") { "std" } else { "core" };
    let stdlib = Ident::new(stdlib, Span::call_site());

    assert_eq!(
        transform_stmts(
            &opts,
            None,
            parse_quote! {
                fut1.await;

                |x| fut2.await;

                async { fut3.await };

                async fn x() {
                    fut4.await;
                }

                let something = {{
                    call_a_function(fut5.await)?
                }};

                (yield something);

                assert_eq!(fut6.await, fut7.await);
                not_assert_eq!(fut6.await, fut7.await);
                matches!(fut6.await, Any tokens "can" go 'here and _ should be passed verbatim!);

                ((((((((((fut8.await).await.await)))))))))
            },
        )
        .to_string(),
        quote! {
            #[allow(unused_imports)]
            use some::path::__CompletionFutureIntoFutureUnsafe;

            some::path::__FutureOrCompletionFuture(fut1).__into_future_unsafe().await;

            |x| fut2.await;

            async { fut3.await };

            async fn x() {
                fut4.await;
            }

            let something = {{
                call_a_function(
                    some::path::__FutureOrCompletionFuture(fut5).__into_future_unsafe().await
                )?
            }};

            ({
                ::core::compile_error!("`yield` can only be used inside a stream");
                something
            });

            #stdlib::assert_eq!(
                some::path::__FutureOrCompletionFuture(fut6).__into_future_unsafe().await,
                some::path::__FutureOrCompletionFuture(fut7).__into_future_unsafe().await
            );
            not_assert_eq!(fut6.await, fut7.await);
            #stdlib::matches!(
                some::path::__FutureOrCompletionFuture(fut6).__into_future_unsafe().await,
                Any tokens "can" go 'here and _ should be passed verbatim!
            );

            (((((((((
                some::path::__FutureOrCompletionFuture(
                    some::path::__FutureOrCompletionFuture((
                        some::path::__FutureOrCompletionFuture(fut8)
                            .__into_future_unsafe()
                            .await
                    ))
                    .__into_future_unsafe()
                    .await
                )
                .__into_future_unsafe()
                .await
            )))))))))
        }
        .to_string(),
    );

    assert_eq!(
        transform_stmts(
            &opts,
            Some(Ident::new("some_yielder", Span::call_site())),
            parse_quote! {
                fn inner() {
                    yield x;
                }
                |_| yield x;

                yield fut.await;

                yield (yield (yield));

                let x = something?;
            },
        )
        .to_string(),
        quote! {
            #[allow(unused_imports)]
            use some::path::__CompletionFutureIntoFutureUnsafe;

            fn inner() {
                yield x;
            }
            |_| yield x;

            some_yielder.send(
                some::path::__FutureOrCompletionFuture(fut).__into_future_unsafe().await
            ).await;

            some_yielder.send((some_yielder.send((some_yielder.send(()).await)).await)).await;

            let x = match some::path::__Try::into_result(something) {
                ::core::result::Result::Ok(val) => val,
                ::core::result::Result::Err(e) => {
                    #[allow(clippy::useless_conversion)]
                    some_yielder.send(some::path::__Try::from_error(::core::convert::From::from(e))).await;
                    return;
                }
            };
        }
        .to_string(),
    )
}

/// Visitor that unelides the lifetimes of types and adds them to a set of generics.
struct Unelider<'a> {
    generics: &'a mut Generics,
    lifetimes: usize,
}

impl<'a> Unelider<'a> {
    fn new(generics: &'a mut Generics) -> Self {
        Self {
            generics,
            lifetimes: 0,
        }
    }

    fn make_lifetime(&mut self, span: Span) -> Lifetime {
        let lifetime = Lifetime::new(&format!("'__life{}", self.lifetimes), span);
        self.lifetimes += 1;

        self.generics
            .params
            .insert(0, lifetime_generic(lifetime.clone()));

        lifetime
    }
}

impl<'a> VisitMut for Unelider<'a> {
    fn visit_receiver_mut(&mut self, r: &mut Receiver) {
        if let Some((and, lifetime)) = &mut r.reference {
            if lifetime.is_none() {
                *lifetime = Some(self.make_lifetime(and.spans[0]));
            }
        }
    }
    fn visit_type_reference_mut(&mut self, r: &mut TypeReference) {
        if r.lifetime.is_none() {
            r.lifetime = Some(self.make_lifetime(r.and_token.span));
        }
        self.visit_type_mut(&mut r.elem);
    }
}

/// Visitor that finds `Self`.
struct HasSelf(bool);
impl VisitMut for HasSelf {
    fn visit_receiver_mut(&mut self, _: &mut Receiver) {
        self.0 = true;
    }
    fn visit_ident_mut(&mut self, i: &mut Ident) {
        if i == "Self" {
            self.0 = true;
        }
    }
}

/// The unit type and value, `()`.
struct Unit(Span);
impl ToTokens for Unit {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let mut group = Group::new(Delimiter::Parenthesis, TokenStream::new());
        group.set_span(self.0);
        tokens.append(group);
    }
}

fn lifetime_generic(lifetime: Lifetime) -> GenericParam {
    GenericParam::Lifetime(LifetimeDef {
        attrs: Vec::new(),
        lifetime,
        colon_token: None,
        bounds: Punctuated::new(),
    })
}

/// A type that sets the span location of a token stream.
struct SetSpanLocation<'a>(&'a TokenStream, Span);
impl ToTokens for SetSpanLocation<'_> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.extend(self.0.clone().into_iter().map(|mut token| {
            token.set_span(token.span().located_at(self.1));
            token
        }));
    }
}
