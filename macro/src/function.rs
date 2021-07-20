use std::mem;

use proc_macro2::{Ident, Span, TokenStream};
use quote::{quote_spanned, ToTokens};
use syn::parse_quote;
use syn::punctuated::Punctuated;
use syn::visit_mut::VisitMut;
use syn::{
    token, Block, Expr, ExprAsync, GenericParam, Generics, Lifetime, LifetimeDef, Receiver,
    ReturnType, Stmt, Token, Type, TypeReference,
};

use crate::{AnyFn, Boxed, CratePath};

/// Transform an `async fn` to a completion async fn.
pub(crate) fn transform(mut f: AnyFn, boxed: Option<Boxed>, crate_path: &CratePath) -> TokenStream {
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

    // Change the return type.
    let (rarrow, ret_ty) = match f.sig.output {
        ReturnType::Default => (Token![->](async_span), quote_spanned!(async_span=> ())),
        ReturnType::Type(rarrow, ty) => (rarrow, ty.into_token_stream()),
    };

    let crate_path_async_span = crate_path.with_span(async_span);
    let ret_ty_bounds = quote_spanned! {async_span=>
        #crate_path_async_span::CompletionFuture<Output = #ret_ty> + #ret_lifetime
    };
    let ret_ty = if let Some(boxed) = &boxed {
        let send = if boxed.send {
            Some(quote_spanned!(boxed.span=> + ::core::marker::Send))
        } else {
            None
        };
        quote_spanned!(boxed.span=> ::core::pin::Pin<::std::boxed::Box<dyn #ret_ty_bounds #send>>)
    } else {
        quote_spanned!(async_span=> impl #ret_ty_bounds)
    };
    f.sig.output = ReturnType::Type(rarrow, Box::new(Type::Verbatim(ret_ty)));

    if let Some(block) = &mut f.block {
        // Transform the function body.
        let body = crate::block::transform(
            ExprAsync {
                attrs: Vec::new(),
                async_token: Token![async](async_span),
                capture: Some(Token![move](async_span)),
                block: Block {
                    brace_token: token::Brace {
                        span: block.brace_token.span,
                    },
                    stmts: mem::take(&mut block.stmts),
                },
            },
            crate_path,
        );
        let body = if let Some(boxed) = boxed {
            quote_spanned!(boxed.span=> ::std::boxed::Box::pin(#body))
        } else {
            body
        };
        block.stmts = vec![Stmt::Expr(Expr::Verbatim(body))];
    }

    f.into_token_stream()
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

#[cfg(test)]
mod tests {
    use super::*;

    use quote::quote;

    use crate::block::in_scope;

    #[test]
    fn basic() {
        let input = parse_quote! {
            #[attr]
            async fn foo() {
                |_| fut.await;
            }
        };
        let in_scope = in_scope();
        let output = quote! {
            #[attr]
            fn foo<'__completion_future>() -> impl c::CompletionFuture<Output = ()> + '__completion_future {
                c::__completion_async(async move {
                    #[allow(unused_imports)]
                    use c::{__CompletionFutureIntoAwaitable, __IntoFutureOrCompletionFuture};
                    #[allow(unused_variables)]
                    let #in_scope = ();

                    |_| fut.await;
                })
            }
        };
        let crate_path = CratePath::new(quote!(c));
        assert_eq!(
            transform(input, None, &crate_path).to_string(),
            output.to_string()
        );
    }

    #[test]
    fn lifetimed() {
        let input = parse_quote! {
            pub(super) async fn do_stuff<T: Clone>(&mut self, x: &&T) -> Vec<u8> {
                fut.await;
            }
        };
        let in_scope = in_scope();
        let output = quote! {
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
                ::crate::path::__completion_async(async move {
                    #[allow(unused_imports)]
                    use ::crate::path::{__CompletionFutureIntoAwaitable, __IntoFutureOrCompletionFuture};
                    #[allow(unused_variables)]
                    let #in_scope = ();

                    (#in_scope, fut.__into_future_or_completion_future().__into_awaitable().await).1;
                })
            }
        };
        let crate_path = CratePath::new(quote!(::crate::path));
        assert_eq!(
            transform(input, None, &crate_path).to_string(),
            output.to_string()
        );
    }

    #[test]
    fn boxed() {
        let input = parse_quote! {
            crate async fn do_stuff(x: &i32);
        };
        let output = quote! {
            crate fn do_stuff<'__completion_future, '__life0>(
                x: &'__life0 i32
            ) -> ::core::pin::Pin<::std::boxed::Box<
                dyn crate::CompletionFuture<Output = ()> + '__completion_future + ::core::marker::Send
            >>
            where
                '__life0: '__completion_future;
        };

        let crate_path = CratePath::new(quote!(crate));
        assert_eq!(
            transform(
                input,
                Some(Boxed {
                    span: Span::call_site(),
                    send: true
                }),
                &crate_path
            )
            .to_string(),
            output.to_string()
        );
    }

    #[test]
    fn boxed_no_send() {
        let input = parse_quote! {
            #[outer = "attributes"]
            async fn xyz() -> i32 {
                #![inner(attributes)]
                x.await?
            }
        };
        let in_scope = in_scope();
        let output = quote! {
            #[outer = "attributes"]
            fn xyz<'__completion_future>() -> ::core::pin::Pin<::std::boxed::Box<
                dyn c::CompletionFuture<Output = i32> + '__completion_future
            >> {
                #![inner(attributes)]
                ::std::boxed::Box::pin(c::__completion_async(async move {
                    #[allow(unused_imports)]
                    use c::{__CompletionFutureIntoAwaitable, __IntoFutureOrCompletionFuture};
                    #[allow(unused_variables)]
                    let #in_scope = ();

                    (#in_scope, x.__into_future_or_completion_future().__into_awaitable().await).1?
                }))
            }
        };

        let crate_path = CratePath::new(quote!(c));
        assert_eq!(
            transform(
                input,
                Some(Boxed {
                    span: Span::call_site(),
                    send: false,
                }),
                &crate_path
            )
            .to_string(),
            output.to_string(),
        );
    }
}
