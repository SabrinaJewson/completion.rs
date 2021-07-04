macro_rules! define_macro {
    ($vis:vis $name:ident { $($rules:tt)* }) => {
        mod $name {
            macro_rules! $name { $($rules)* }
            pub(crate) use $name;
        }
        $vis use $name::$name;
    };
}

define_macro! { pub(crate) derive_completion_future {
    ([$($generics:tt)*] $ty:ty) => {
        const _: () = {
            use ::std::future::Future;
            use ::std::pin::Pin;
            use ::std::task::{Context, Poll};

            impl<$($generics)*> ::completion_core::CompletionFuture for $ty {
                type Output = <Self as Future>::Output;

                unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                    Future::poll(self, cx)
                }
                unsafe fn poll_cancel(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                    Poll::Ready(())
                }
            }
        };
    };
    ($ty:ty) => { $crate::util::derive_completion_future!([] $ty); };
}}
