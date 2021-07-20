use proc_macro2::{Ident, Span, TokenStream};
use quote::{quote_spanned, ToTokens};
use syn::{Expr, ExprAsync, Stmt, Token};

use super::{CratePath, OuterAttrs};

/// Transform an async block to a completion stream.
pub(crate) fn transform(mut expr: ExprAsync, crate_path: &CratePath) -> TokenStream {
    let item = Ident::new("item", Span::mixed_site().located_at(expr.async_token.span));

    let mut yielded = false;

    crate::transform_top_level(&mut expr.block.stmts, crate_path, |expr| match expr {
        Expr::Yield(yield_expr) => {
            yielded = true;

            let crate_path = crate_path.with_span(yield_expr.yield_token.span);
            let attrs = OuterAttrs(&yield_expr.attrs);
            let yielded = yield_expr.expr.take().map_or_else(
                || quote_spanned!(yield_expr.yield_token.span=> ()),
                |e| e.into_token_stream(),
            );

            *expr = syn::parse2(quote_spanned! {yield_expr.yield_token.span=>
                #attrs
                #crate_path::__yield_value(#item, #yielded).await
            })
            .unwrap();
        }
        Expr::Try(expr_try) => {
            let question_span = expr_try.question_token.spans[0];
            let crate_path = crate_path.with_span(question_span);
            let attrs = OuterAttrs(&expr_try.attrs);
            let base = &expr_try.expr;

            *expr = syn::parse2(quote_spanned! {question_span=>
                #attrs
                match #crate_path::__Try::into_result(#base) {
                    ::core::result::Result::Ok(val) => val,
                    ::core::result::Result::Err(e) => {
                        #[allow(clippy::useless_conversion)]
                        #crate_path::__yield_value(
                            #item,
                            #crate_path::__Try::from_error(::core::convert::From::from(e))
                        ).await;
                        return
                    }
                }
            })
            .unwrap();
        }
        _ => {}
    });

    let span = expr.async_token.span;

    if !yielded {
        expr.block.stmts.insert(
            0,
            Stmt::Semi(
                Expr::Verbatim(quote_spanned!(span=>
                    let _: ::core::marker::PhantomData<()> = #item
                )),
                Token![;](span),
            ),
        );
    }

    let transformed = crate::block::transform(expr, crate_path);

    let crate_path = crate_path.with_span(span);

    quote_spanned! {span=> {
        let #item = ::core::marker::PhantomData;
        #crate_path::__completion_stream(#transformed, #item)
    }}
}

#[cfg(test)]
mod tests {
    use super::*;

    use quote::quote;

    use crate::block::in_scope;

    #[track_caller]
    fn test(input: TokenStream, output: TokenStream) {
        let input: ExprAsync =
            syn::parse2(quote!(async move { #input })).expect("Failed to parse input");
        let in_scope = in_scope();
        let output: Expr = syn::parse2(quote! {{
            let item = ::core::marker::PhantomData;
            crate::__completion_stream(
                crate::__completion_async(async move {
                    #[allow(unused_imports)]
                    use crate::{__CompletionFutureIntoAwaitable, __IntoFutureOrCompletionFuture};
                    #[allow(unused_variables)]
                    let #in_scope = ();
                    #output
                }),
                item
            )
        }})
        .expect("Failed to parse output");

        assert_eq!(
            transform(input, &CratePath::new(quote!(crate))).to_string(),
            output.into_token_stream().to_string(),
        );
    }

    #[test]
    fn empty() {
        test(
            TokenStream::new(),
            quote! {
                let _: ::core::marker::PhantomData<()> = item;
            },
        );
    }

    #[test]
    fn ignore_scopes() {
        let input = quote! {
            fn inner() {
                yield x;
            }
            |_| yield x;
        };
        let output = quote! {
            let _: ::core::marker::PhantomData<()> = item;
            fn inner() {
                yield x;
            }
            |_| yield x;
        };
        test(input, output);
    }

    #[test]
    fn yielding() {
        let input = quote! {
            #[attr1] yield #[attr2] fut.await;
            yield (yield (yield))
        };
        let in_scope = in_scope();
        let output = quote! {
            #[attr1]
            (
                #in_scope,
                crate::__yield_value(
                    item,
                    #[attr2]
                    (
                        #in_scope,
                        fut.__into_future_or_completion_future().__into_awaitable().await
                    )
                    .1
                )
                .__into_future_or_completion_future()
                .__into_awaitable()
                .await
            ).1;

            (
                #in_scope,
                crate::__yield_value(item, ((
                    #in_scope,
                    crate::__yield_value(item, ((
                        #in_scope,
                        crate::__yield_value(item, ())
                        .__into_future_or_completion_future()
                        .__into_awaitable()
                        .await
                    ).1))
                    .__into_future_or_completion_future()
                    .__into_awaitable()
                    .await
                ).1))
                .__into_future_or_completion_future()
                .__into_awaitable()
                .await
            )
            .1
        };
        test(input, output);
    }

    #[test]
    fn r#try() {
        let input = quote! {
            let x = #[attr] something?;
        };
        let in_scope = in_scope();
        let output = quote! {
            let _: ::core::marker::PhantomData<()> = item;

            let x = #[attr] match crate::__Try::into_result(something) {
                ::core::result::Result::Ok(val) => val,
                ::core::result::Result::Err(e) => {
                    #[allow(clippy::useless_conversion)]
                    (
                        #in_scope,
                        crate::__yield_value(
                            item,
                            crate::__Try::from_error(::core::convert::From::from(e))
                        )
                        .__into_future_or_completion_future()
                        .__into_awaitable()
                        .await
                    )
                    .1;
                    return
                }
            };
        };
        test(input, output);
    }
}
