use core::convert::Infallible;
use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::CompletionFuture;
use pin_project_lite::pin_project;

use super::super::ControlFlow;
use super::base::{Join, JoinTuple};

/// Zip a tuple of futures, waiting for them all to complete.
///
/// This takes any tuple of two or more futures, and outputs a tuple of the results.
///
/// Requires the `std` feature, as [`std::panic::catch_unwind`] is needed when polling the futures
/// to ensure soundness.
///
/// # Examples
///
/// ```
/// use completion::{future, completion_async};
///
/// # future::block_on(completion_async! {
/// assert_eq!(
///     future::zip((
///         completion_async!(5),
///         completion_async!(6),
///     ))
///     .await,
///     (5, 6),
/// );
/// # });
/// ```
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub fn zip<T: ZipTuple>(futures: T) -> Zip<T> {
    Zip {
        inner: Join::new(futures.into_tuple()),
        _correct_debug_bounds: PhantomData,
    }
}

pin_project! {
    /// Future for [`zip`].
    #[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
    #[derive(Debug)]
    pub struct Zip<T: ZipTuple> {
        #[pin]
        inner: Join<T::JoinTuple>,
        _correct_debug_bounds: PhantomData<T::Futures>,
    }
}

impl<T: ZipTuple> CompletionFuture for Zip<T> {
    type Output = <T::JoinTuple as JoinTuple>::Output;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().inner.poll(cx).map(|flow| match flow {
            ControlFlow::Continue(val) => val,
            ControlFlow::Break(no) => match no {},
        })
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.project().inner.poll_cancel(cx)
    }
}

pin_project! {
    /// A wrapper for a future inside a [`Zip`].
    #[derive(Debug)]
    pub struct Zipped<F> {
        #[pin]
        inner: F,
    }
}
impl<F> Zipped<F> {
    fn new(inner: F) -> Self {
        Self { inner }
    }
}
impl<F: CompletionFuture> CompletionFuture for Zipped<F> {
    type Output = ControlFlow<Infallible, F::Output>;
    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().inner.poll(cx).map(ControlFlow::Continue)
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.project().inner.poll_cancel(cx)
    }
}

/// A tuple of futures that can be used in `Zip`.
pub trait ZipTuple {
    /// The tuple that can be used with `Join`.
    type JoinTuple: JoinTuple<Futures = Self::Futures, Break = Infallible>;
    fn into_tuple(self) -> Self::JoinTuple;

    type Futures;
}

macro_rules! impl_zip_tuple {
    ($($param:ident),*) => {
        impl<$($param,)*> ZipTuple for ($($param,)*)
        where
            $($param: CompletionFuture,)*
        {
            type JoinTuple = ($(Zipped<$param>,)*);
            fn into_tuple(self) -> Self::JoinTuple {
                let ($($param,)*) = self;
                ($(Zipped::new($param),)*)
            }

            type Futures = <Self::JoinTuple as JoinTuple>::Futures;
        }
    }
}
apply_on_tuples!(impl_zip_tuple!);

#[cfg(test)]
mod tests {
    use super::*;

    use std::future::ready;

    use crate::future::block_on;

    #[test]
    fn success() {
        assert_eq!(
            block_on(zip((
                ready(Box::new(0)),
                ready(Box::new(1)),
                ready(Box::new(2)),
            ))),
            (Box::new(0), Box::new(1), Box::new(2)),
        );
    }
}
