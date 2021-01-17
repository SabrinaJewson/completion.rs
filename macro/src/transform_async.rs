use proc_macro2::{Delimiter, Group, Ident, Span, TokenStream};
use quote::{quote, quote_spanned, ToTokens, TokenStreamExt};
use syn::parse::ParseStream;
use syn::punctuated::Punctuated;
use syn::visit_mut::{self, VisitMut};
use syn::{Expr, ExprAsync, Item, Stmt, Token};

use super::CratePath;

/// Transforms an async block so it can await completion futures and yield if it's a stream.
pub(crate) fn transform(mut expr: ExprAsync, stream: bool, crate_path: &CratePath) -> TokenStream {
    let yielder = if stream {
        Some(Ident::new("__yielder", Span::mixed_site()))
    } else {
        None
    };

    let mut transformer = Transformer {
        crate_path,
        yielder: &yielder,
        yielded: false,
    };
    for stmt in &mut expr.block.stmts {
        transformer.visit_stmt_mut(stmt);
    }

    let crate_path = crate_path.with_span(expr.async_token.span);
    let mut block_start_tokens = quote_spanned! {expr.async_token.span=>
        #[allow(unused_imports)]
        use #crate_path::__CompletionFutureIntoFutureUnsafe;
    };

    if let (Some(yielder), false) = (&yielder, transformer.yielded) {
        block_start_tokens.extend(quote! {
            if false {
                #yielder.send(()).await;
            }
        });
    }

    expr.block
        .stmts
        .insert(0, Stmt::Expr(Expr::Verbatim(block_start_tokens)));

    if let Some(yielder) = yielder {
        quote_spanned!(expr.async_token.span=>
            #crate_path::__AsyncStream::new(move |mut #yielder| #expr)
        )
    } else {
        quote_spanned!(expr.async_token.span=>
            #crate_path::__make_completion_future(#expr)
        )
    }
}

/// Allows completion futures to be awaited and yields to be used inside streams.
struct Transformer<'a> {
    crate_path: &'a CratePath,
    yielder: &'a Option<Ident>,
    yielded: bool,
}
impl VisitMut for Transformer<'_> {
    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        match expr {
            Expr::Async(_) | Expr::Closure(_) => {
                // Don't do anything, we don't want to touch inner async blocks or closures.
            }
            Expr::Await(expr_await) => {
                self.visit_expr_mut(&mut expr_await.base);

                let base = &expr_await.base;
                let await_span = expr_await.await_token.span;
                let crate_path = self.crate_path.with_span(await_span);

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

                        let crate_path = self.crate_path.with_span(span);
                        let macro_ident = Ident::new(macro_name, span);
                        let path =
                            quote_spanned!(span=> #crate_path::__special_macros::#macro_ident);
                        expr_macro.mac.path = syn::parse2(path).unwrap();
                    }
                }
            }
            Expr::Try(expr_try) => {
                if let Some(yielder) = &self.yielder {
                    let question_span = expr_try.question_token.spans[0];
                    let crate_path = self.crate_path.with_span(question_span);
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

/// The unit type and value, `()`.
struct Unit(Span);
impl ToTokens for Unit {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let mut group = Group::new(Delimiter::Parenthesis, TokenStream::new());
        group.set_span(self.0);
        tokens.append(group);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use syn::parse_quote;

    #[test]
    fn async_block() {
        let input = parse_quote! {
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

                (yield something);

                assert_eq!(fut6.await, fut7.await);
                not_assert_eq!(fut6.await, fut7.await);
                matches!(fut6.await, Any tokens "can" go 'here and _ should be passed verbatim!);

                ((((((((((fut8.await).await.await)))))))))
            }
        };
        let output = quote! {
            some::path::__make_completion_future(async move {
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

                some::path::__special_macros::assert_eq!(
                    some::path::__FutureOrCompletionFuture(fut6).__into_future_unsafe().await,
                    some::path::__FutureOrCompletionFuture(fut7).__into_future_unsafe().await
                );
                not_assert_eq!(fut6.await, fut7.await);
                some::path::__special_macros::matches!(
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
            })
        };

        let crate_path = CratePath::new(quote!(some::path));
        assert_eq!(
            transform(input, false, &crate_path).to_string(),
            output.to_string()
        );
    }

    #[test]
    fn stream() {
        let input = parse_quote! {
            async {
                fn inner() {
                    yield x;
                }
                |_| yield x;

                yield fut.await;

                yield (yield (yield));

                let x = something?;
            }
        };
        let output = quote! {
            some::path::__AsyncStream::new(move |mut __yielder| async {
                #[allow(unused_imports)]
                use some::path::__CompletionFutureIntoFutureUnsafe;

                fn inner() {
                    yield x;
                }
                |_| yield x;

                __yielder.send(
                    some::path::__FutureOrCompletionFuture(fut).__into_future_unsafe().await
                ).await;

                __yielder.send((__yielder.send((__yielder.send(()).await)).await)).await;

                let x = match some::path::__Try::into_result(something) {
                    ::core::result::Result::Ok(val) => val,
                    ::core::result::Result::Err(e) => {
                        #[allow(clippy::useless_conversion)]
                        __yielder.send(some::path::__Try::from_error(::core::convert::From::from(e))).await;
                        return;
                    }
                };
            })
        };

        let crate_path = CratePath::new(quote!(some::path));
        assert_eq!(
            transform(input, true, &crate_path).to_string(),
            output.to_string()
        );
    }
}
