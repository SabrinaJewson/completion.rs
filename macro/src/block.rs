use proc_macro2::{Ident, Span, TokenStream};
use quote::quote_spanned;
use syn::{Expr, ExprAsync, Stmt, Token};

use super::{CratePath, OuterAttrs};

/// Transform an async block so it can await completion futures.
pub(crate) fn transform(mut expr: ExprAsync, crate_path: &CratePath) -> TokenStream {
    let in_scope = in_scope();

    crate::transform_top_level(&mut expr.block.stmts, crate_path, |expr| {
        if let Expr::Await(expr_await) = expr {
            let base = &expr_await.base;
            let await_span = expr_await.await_token.span;
            let attrs = OuterAttrs(&expr_await.attrs);
            let crate_path = crate_path.with_span(await_span);

            *expr = Expr::Verbatim(quote_spanned! {await_span=>
                #attrs
                (#in_scope, #crate_path::__FutureOrCompletionFuture(#base).__into_awaitable().await).1
            });
        }
    });

    let crate_path = crate_path.with_span(expr.async_token.span);

    // Insert our content after all items to mitigate clippy's items_after_statements lint.
    let first_non_item = expr
        .block
        .stmts
        .iter()
        .position(|stmt| !matches!(stmt, Stmt::Item(_)))
        .unwrap_or_else(|| expr.block.stmts.len());

    expr.block.stmts.insert(
        first_non_item,
        Stmt::Semi(
            Expr::Verbatim(quote_spanned! {expr.async_token.span=>
                #[allow(unused_imports)]
                use #crate_path::__CompletionFutureIntoAwaitable;

                #[allow(unused_variables)]
                let #in_scope = ()
            }),
            Token![;](expr.async_token.span),
        ),
    );

    quote_spanned!(expr.async_token.span=> #crate_path::__completion_async(#expr))
}

/// This local variable is required to be in scope when awaiting completion futures, to prevent
/// attribute macros that put statements in items from compiling.
///
/// For example, in this source code:
///
/// ```ignore
/// completion_async! {
///     #[some_attribute_macro]
///     x().await;
/// }
/// ```
///
/// First of all, `completion_async!` will be expanded:
///
/// ```ignore
/// ::completion::__completion_async(async {
///     #[allow(unused_imports)]
///     use ::completion::__CompletionFutureIntoAwaitable;
///     #[allow(unused_variables)]
///     let awaitable_completion_futures = ();
///
///     #[some_attribute_macro]
///     (
///         awaitable_completion_futures,
///         ::completion::__FutureOrCompletionFuture(x()).__into_awaitable().await
///     )
///     .1
/// })
/// ```
///
/// Then `#[some_attribute_macro]` will be expanded:
///
/// ```ignore
/// ::completion::__completion_async(async {
///     #[allow(unused_imports)]
///     use ::completion::__CompletionFutureIntoAwaitable;
///     #[allow(unused_variables)]
///     let awaitable_completion_futures = ();
///
///     async fn some_function() {
///         (
///             awaitable_completion_futures,
///             ::completion::__FutureOrCompletionFuture(x()).__into_awaitable().await
///         )
///         .1
///     }
/// })
/// ```
///
/// If this code would compile, it would be unsound - however, due, to our
/// `awaitable_completion_futures` local variable, it is prevented from even compiling.
/// Additionally, because this span's hygiene is `Span::mixed_site()`, user-defined
/// `awaitable_completion_futures` variables will not be able to interfere.
///
/// Note that this solution isn't perfect - attribute macros can still expand to regular async
/// blocks that await completion futures - however that will be caught at runtime with a panic.
#[cfg(test)]
pub(super) fn in_scope() -> Ident {
    Ident::new("awaitable_completion_futures", Span::mixed_site())
}
#[cfg(not(test))]
fn in_scope() -> Ident {
    Ident::new("awaitable_completion_futures", Span::mixed_site())
}

#[cfg(test)]
mod tests {
    use super::*;

    use quote::{quote, ToTokens};
    use syn::parse_quote;

    #[track_caller]
    fn test(input: TokenStream, output: TokenStream) {
        let input: ExprAsync = parse_quote!(async move { #input });
        let in_scope = in_scope();
        let output: Expr = parse_quote! {
            crate::__completion_async(async move {
                #[allow(unused_imports)]
                use crate::__CompletionFutureIntoAwaitable;
                #[allow(unused_variables)]
                let #in_scope = ();
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
        let in_scope = in_scope();
        let output = quote! {
            #[attr]
            (#in_scope, crate::__FutureOrCompletionFuture(fut).__into_awaitable().await).1
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
        let in_scope = in_scope();
        let output = quote! {
            crate::__special_macros::assert_eq!(
                (#in_scope, crate::__FutureOrCompletionFuture(a).__into_awaitable().await).1,
                (#in_scope, crate::__FutureOrCompletionFuture(b).__into_awaitable().await).1
            );
            crate::__special_macros::assert_eq!(
                (#in_scope, crate::__FutureOrCompletionFuture(a).__into_awaitable().await).1,
                (#in_scope, crate::__FutureOrCompletionFuture(b).__into_awaitable().await).1,
            );

            crate::__special_macros::dbg!(not_assert_eq!(a.await, b.await));

            crate::__special_macros::matches!(
                (#in_scope, crate::__FutureOrCompletionFuture(fut).__into_awaitable().await).1,
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
        let in_scope = in_scope();
        let output = quote! {
            {{((f((
                #in_scope,
                crate::__FutureOrCompletionFuture((
                    #in_scope,
                    crate::__FutureOrCompletionFuture((#[attr] (
                        #in_scope,
                        crate::__FutureOrCompletionFuture(x).__into_awaitable().await
                    ).1)).__into_awaitable().await
                ).1).__into_awaitable().await
            ).1)))}}
        };
        test(input, output);
    }

    #[test]
    fn items_after_statements() {
        let input = parse_quote! {
            async {
                fn x() {}
                struct SomeItems;
                let non_items;
            }
        };
        let in_scope = in_scope();
        let output = quote! {
            crate::__completion_async(async {
                fn x() {}
                struct SomeItems;

                #[allow(unused_imports)]
                use crate::__CompletionFutureIntoAwaitable;
                #[allow(unused_variables)]
                let #in_scope = ();

                let non_items;
            })
        };
        assert_eq!(
            transform(input, &CratePath::new(quote!(crate))).to_string(),
            output.to_string(),
        );
    }
}
