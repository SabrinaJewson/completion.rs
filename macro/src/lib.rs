//! Macro to generate completion-based async functions and blocks. This crate shouldn't be used
//! directly, instead use `completion-util`.

use proc_macro::TokenStream as TokenStream1;

use proc_macro2::{Ident, Span, TokenStream, TokenTree};
use quote::{quote, quote_spanned, ToTokens};
use syn::parse::{self, Parse, ParseStream};
use syn::parse_quote;
use syn::punctuated::Punctuated;
use syn::visit_mut::VisitMut;
use syn::MetaNameValue;
use syn::{token, Token};
use syn::{Block, Lifetime, Lit, Receiver, ReturnType, Stmt};
use syn::{Expr, ExprAsync, ExprAwait, ExprClosure, ExprMacro};
use syn::{GenericParam, Generics, LifetimeDef};
use syn::{Item, ItemFn};
use syn::{Type, TypeReference};

#[proc_macro_attribute]
pub fn completion(attr: TokenStream1, input: TokenStream1) -> TokenStream1 {
    match completion2(attr.into(), input.into()) {
        Ok(tokens) => tokens,
        Err(e) => e.to_compile_error(),
    }
    .into()
}

fn completion2(attr: TokenStream, input: TokenStream) -> parse::Result<TokenStream> {
    Ok(transform_completion_input(
        &syn::parse2(attr)?,
        syn::parse2(input)?,
    ))
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
            ::completion_util::MustComplete::new(async {
                #[allow(unused_imports)]
                use ::completion_util::__CompletionFutureIntoFutureUnsafe;

                ::completion_util::__FutureOrCompletionFuture(fut).__into_future_unsafe().await
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
            #[must_use]
            fn foo<'__completion_future>() -> impl ::completion_util::CompletionFuture<Output = ()> + '__completion_future {
                ::completion_util::MustComplete::new(async move {
                    #[allow(unused_imports)]
                    use ::completion_util::__CompletionFutureIntoFutureUnsafe;

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
            #[must_use]
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
                ::crate::path::MustComplete::new(async move {
                    #[allow(unused_imports)]
                    use ::crate::path::__CompletionFutureIntoFutureUnsafe;

                    ::crate::path::__FutureOrCompletionFuture(fut).__into_future_unsafe().await;
                })
            }
        }
        .to_string(),
    );
}

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

/// Transform the input of the macro.
fn transform_completion_input(opts: &Opts, input: CompletionInput) -> TokenStream {
    match input {
        CompletionInput::AsyncFn(f) => transform_async_fn(opts, f),
        CompletionInput::AsyncBlock(block, semi) => {
            let block = transform_async_block(opts, block);
            quote!(#block #semi)
        }
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

    let crate_path = &opts.crate_path;
    let ret_ty = quote!(impl #crate_path::CompletionFuture<Output = #ret_ty> + #ret_lifetime);
    f.sig.output = ReturnType::Type(rarrow, Box::new(Type::Verbatim(ret_ty)));

    // Change the function body to be an async block and then transform it.
    let brace_token = token::Brace {
        span: f.block.brace_token.span,
    };
    f.block = Box::new(Block {
        brace_token,
        stmts: vec![Stmt::Expr(Expr::Verbatim(transform_async_block(
            opts,
            ExprAsync {
                attrs: Vec::new(),
                async_token: Token![async](async_span),
                capture: Some(Token![move](async_span)),
                block: *f.block,
            },
        )))],
    });

    // Add a #[must_use] attribute if there isn't one.
    if f.attrs.iter().all(|attr| !attr.path.is_ident("must_use")) {
        f.attrs.push(parse_quote!(#[must_use]));
    }

    f.into_token_stream()
}

#[proc_macro]
#[doc(hidden)]
pub fn completion_async_inner(input: TokenStream1) -> TokenStream1 {
    match completion_async_inner2(input.into(), false) {
        Ok(tokens) => tokens,
        Err(e) => e.to_compile_error(),
    }
    .into()
}

#[proc_macro]
#[doc(hidden)]
pub fn completion_async_move_inner(input: TokenStream1) -> TokenStream1 {
    match completion_async_inner2(input.into(), true) {
        Ok(tokens) => tokens,
        Err(e) => e.to_compile_error(),
    }
    .into()
}

fn completion_async_inner2(input: TokenStream, capture_move: bool) -> parse::Result<TokenStream> {
    let mut tokens = input.into_iter();
    let crate_path = match tokens.next().unwrap() {
        TokenTree::Group(group) => group.stream(),
        _ => panic!(),
    };
    let opts = Opts { crate_path };

    let input: TokenStream = tokens.collect();
    let expr_async = if capture_move {
        syn::parse2(quote!(async move { #input }))?
    } else {
        syn::parse2(quote!(async { #input }))?
    };
    Ok(transform_async_block(&opts, expr_async))
}

struct Opts {
    crate_path: TokenStream,
}
impl Parse for Opts {
    fn parse(input: ParseStream<'_>) -> parse::Result<Self> {
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
            crate_path: crate_path.unwrap_or_else(|| quote!(::completion_util)),
        })
    }
}

/// Transform async blocks like:
///
/// ```ignore
/// async { fut.await }
/// ```
///
/// into:
///
/// ```ignore
/// MustComplete::new(async {
///     use completion_util::__CompletionFutureIntoFutureUnsafe;
///
///     completion_util::__FutureOrCompletionFuture(fut).__into_future_unsafe()
/// })
/// ```
fn transform_async_block(opts: &Opts, mut async_block: ExprAsync) -> TokenStream {
    Transformer { opts }.visit_block_mut(&mut async_block.block);

    let crate_path = &opts.crate_path;

    async_block.block.stmts.insert(
        0,
        parse_quote! {
            #[allow(unused_imports)]
            use #crate_path::__CompletionFutureIntoFutureUnsafe;
        },
    );

    quote! {
        #crate_path::MustComplete::new(#async_block)
    }
}

/// A transformer that transforms `fut.await` into `unsafe { AssertCompletes::new(fut) }.await`.
struct Transformer<'a> {
    opts: &'a Opts,
}
impl<'a> VisitMut for Transformer<'a> {
    fn visit_expr_async_mut(&mut self, _: &mut ExprAsync) {
        // Don't do anything, we don't want to touch inner async blocks.
    }
    fn visit_expr_await_mut(&mut self, expr: &mut ExprAwait) {
        self.visit_expr_mut(&mut expr.base);

        let base = &expr.base;
        let crate_path = &self.opts.crate_path;

        expr.base = Box::new(parse_quote! {
            #crate_path::__FutureOrCompletionFuture(#base).__into_future_unsafe()
        });
    }
    fn visit_expr_closure_mut(&mut self, _: &mut ExprClosure) {
        // Don't do anything, we don't want to touch inner closures.
    }
    fn visit_expr_macro_mut(&mut self, expr: &mut ExprMacro) {
        // Normally we don't transform the inner expressions in macros as they could for example
        // put the inner code in a function, which would make this macro unsound. However, we
        // transform the bodies of some macros by special-casing them.
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

        if let Some(&macro_name) = SPECIAL_MACROS
            .iter()
            .find(|name| expr.mac.path.is_ident(name))
        {
            let succeeded = if macro_name == "matches" {
                let res = expr.mac.parse_body_with(|tokens: ParseStream<'_>| {
                    let expr = tokens.parse::<Expr>()?;
                    let rest = tokens.parse::<TokenStream>()?;
                    Ok((expr, rest))
                });
                if let Ok((mut scrutinee, rest)) = res {
                    self.visit_expr_mut(&mut scrutinee);
                    expr.mac.tokens = scrutinee.into_token_stream();
                    expr.mac.tokens.extend(rest.into_token_stream());
                    true
                } else {
                    false
                }
            } else {
                let res = expr
                    .mac
                    .parse_body_with(<Punctuated<_, Token![,]>>::parse_terminated);
                if let Ok(mut exprs) = res {
                    for expr in &mut exprs {
                        self.visit_expr_mut(expr);
                    }
                    expr.mac.tokens = exprs.into_token_stream();
                    true
                } else {
                    false
                }
            };

            if succeeded {
                let crate_path = &self.opts.crate_path;
                let macro_ident = Ident::new(macro_name, Span::call_site());
                expr.mac.path = parse_quote!(#crate_path::__special_macros::#macro_ident);
            }
        }
    }
    fn visit_item_mut(&mut self, _: &mut Item) {
        // Don't do anything, we don't want to touch inner items.
    }
}

#[test]
fn test_transform_async_block() {
    let opts = Opts {
        crate_path: quote!(some::path),
    };

    assert_eq!(
        transform_async_block(
            &opts,
            parse_quote! {
                async move {
                    fut1.await;

                    |x| fut2.await;

                    async { fut3.await };

                    async fn x() {
                        fut4.await;
                    }

                    let something = {{
                        call_a_function(fut5.await)?
                    }};

                    ((((((((((fut6.await).await.await)))))))))
                }
            },
        )
        .to_string(),
        quote! {
            some::path::MustComplete::new(async move {
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

                (((((((((
                    some::path::__FutureOrCompletionFuture(
                        some::path::__FutureOrCompletionFuture((
                            some::path::__FutureOrCompletionFuture(fut6)
                                .__into_future_unsafe()
                                .await
                        ))
                        .__into_future_unsafe()
                        .await
                    )
                    .__into_future_unsafe()
                    .await
                )))))))))
            })
        }
        .to_string(),
    );
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

fn lifetime_generic(lifetime: Lifetime) -> GenericParam {
    GenericParam::Lifetime(LifetimeDef {
        attrs: Vec::new(),
        lifetime,
        colon_token: None,
        bounds: Punctuated::new(),
    })
}
