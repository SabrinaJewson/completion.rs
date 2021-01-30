use proc_macro2::TokenStream;
use quote::quote_spanned;
use syn::{Expr, ExprAsync, Stmt, Token};

use super::{CratePath, OuterAttrs};

/// Transform an async block so it can await completion futures.
pub(crate) fn transform(mut expr: ExprAsync, crate_path: &CratePath) -> TokenStream {
    crate::transform_top_level(&mut expr.block.stmts, crate_path, |expr| {
        if let Expr::Await(expr_await) = expr {
            let base = &expr_await.base;
            let await_span = expr_await.await_token.span;
            let attrs = OuterAttrs(&expr_await.attrs);
            let crate_path = crate_path.with_span(await_span);

            *expr = Expr::Verbatim(quote_spanned! {await_span=>
                #attrs
                #crate_path::__FutureOrCompletionFuture(#base).__into_awaitable().await
            });
        }
    });

    let crate_path = crate_path.with_span(expr.async_token.span);

    expr.block.stmts.insert(
        0,
        Stmt::Semi(
            Expr::Verbatim(quote_spanned! {expr.async_token.span=>
                #[allow(unused_imports)]
                use #crate_path::__CompletionFutureIntoAwaitable
            }),
            Token![;](expr.async_token.span),
        ),
    );

    quote_spanned!(expr.async_token.span=> #crate_path::__completion_async(#expr))
}

#[cfg(test)]
mod tests {
    use super::*;

    use quote::{quote, ToTokens};
    use syn::parse_quote;

    #[track_caller]
    fn test(input: TokenStream, output: TokenStream) {
        let input: ExprAsync = parse_quote!(async move { #input });
        let output: Expr = parse_quote! {
            crate::__completion_async(async move {
                #[allow(unused_imports)]
                use crate::__CompletionFutureIntoAwaitable;
                #output
            })
        };

        assert_eq!(
            transform(input, &CratePath::new(quote!(crate))).to_string(),
            output.into_token_stream().to_string(),
        );
    }

    #[test]
    fn empty() {
        test(TokenStream::new(), TokenStream::new());
    }

    #[test]
    fn awaiting() {
        let input = quote! {
            #[attr]
            fut.await
        };
        let output = quote! {
            #[attr]
            crate::__FutureOrCompletionFuture(fut).__into_awaitable().await
        };

        test(input, output);
    }

    #[test]
    fn ignore_scopes() {
        let input = quote! {
            |_| ().await;
            async { ().await; }
            async fn x() {
                ().await;
            }
        };
        let output = quote! {
            |_| ().await;
            async { ().await; }
            async fn x() {
                ().await;
            }
        };

        test(input, output);
    }

    #[test]
    fn macros() {
        let input = quote! {
            assert_eq!(a.await, b.await);
            crate::__special_macros::assert_eq!(a.await, b.await,);
            dbg!(not_assert_eq!(a.await, b.await));

            matches!(fut.await, Any tokens "can" go 'here and _ should be passed verbatim!.await);
        };
        let output = quote! {
            crate::__special_macros::assert_eq!(
                crate::__FutureOrCompletionFuture(a).__into_awaitable().await,
                crate::__FutureOrCompletionFuture(b).__into_awaitable().await
            );
            crate::__special_macros::assert_eq!(
                crate::__FutureOrCompletionFuture(a).__into_awaitable().await,
                crate::__FutureOrCompletionFuture(b).__into_awaitable().await,
            );

            crate::__special_macros::dbg!(not_assert_eq!(a.await, b.await));

            crate::__special_macros::matches!(
                crate::__FutureOrCompletionFuture(fut).__into_awaitable().await,
                Any tokens "can" go 'here and _ should be passed verbatim!.await
            );
        };
        test(input, output);
    }

    #[test]
    fn nesting() {
        let input = quote! {
            {{((f(
                (#[attr] x.await).await.await
            )))}}
        };
        let output = quote! {
            {{((f(
                crate::__FutureOrCompletionFuture(
                    crate::__FutureOrCompletionFuture(
                        (#[attr] crate::__FutureOrCompletionFuture(
                            x
                        ).__into_awaitable().await)
                    ).__into_awaitable().await
                ).__into_awaitable().await
            )))}}
        };
        test(input, output);
    }
}
