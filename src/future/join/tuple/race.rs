use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::CompletionFuture;
use pin_project_lite::pin_project;

use super::super::{ControlFlow, RaceFuture};
use super::base::{Join, JoinTuple};

/// Wait for the first future in a tuple to complete.
///
/// Requires the `std` feature, as [`std::panic::catch_unwind`] is needed when polling the futures
/// to ensure soundness.
///
/// # Examples
///
/// ```
/// use completion::{future, completion_async};
/// use futures_lite::future::yield_now;
///
/// # future::block_on(completion_async! {
/// assert_eq!(
///     future::race((
///         completion_async! {
///             yield_now().await;
///             0
///         },
///         completion_async!(1),
///         std::future::pending(),
///     ))
///     .await,
///     1,
/// );
/// # });
/// ```
///
/// If multiple futures are immediately ready, the earlier one will be chosen. However after this
/// polling will be fair.
///
/// ```
/// use completion::{future, completion_async};
/// use futures_lite::future::yield_now;
///
/// # future::block_on(completion_async! {
/// assert_eq!(
///     future::race((
///         completion_async! {
///             yield_now().await;
///             0
///         },
///         completion_async!(1),
///         completion_async!(2),
///         completion_async!(3),
///     ))
///     .await,
///     1,
/// );
/// # });
/// ```
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub fn race<T: RaceTuple>(futures: T) -> Race<T> {
    Race {
        inner: Join::new(futures.into_tuple()),
        _correct_debug_bounds: PhantomData,
    }
}

pin_project! {
    /// Future for [`race`].
    #[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
    #[derive(Debug)]
    pub struct Race<T: RaceTuple> {
        #[pin]
        inner: Join<T::JoinTuple>,
        _correct_debug_bounds: PhantomData<(T::Futures, T::Output)>,
    }
}

impl<T: RaceTuple> CompletionFuture for Race<T> {
    type Output = T::Output;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().inner.poll(cx).map(|flow| match flow {
            // This type contains an `Infallible`.
            ControlFlow::Continue(_) => unreachable!(),
            ControlFlow::Break(val) => val,
        })
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.project().inner.poll_cancel(cx)
    }
}

/// A tuple of futures that can be used in `Race`.
pub trait RaceTuple {
    /// The tuple that can be used with `Join`.
    type JoinTuple: JoinTuple<Futures = Self::Futures, Break = Self::Output>;
    fn into_tuple(self) -> Self::JoinTuple;

    type Futures;
    type Output;
}

macro_rules! impl_race_tuple {
    ($($param:ident),*) => {
        impl<Output, $($param,)*> RaceTuple for ($($param,)*)
        where
            $($param: CompletionFuture<Output = Output>,)*
        {
            type JoinTuple = ($(RaceFuture<$param>,)*);
            fn into_tuple(self) -> Self::JoinTuple {
                let ($($param,)*) = self;
                ($(RaceFuture::new($param),)*)
            }

            type Futures = <Self::JoinTuple as JoinTuple>::Futures;
            type Output = Output;
        }
    }
}
apply_on_tuples!(impl_race_tuple!);
