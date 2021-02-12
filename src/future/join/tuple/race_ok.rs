use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::CompletionFuture;
use pin_project_lite::pin_project;

use super::super::{ControlFlow, TryFuture};
use super::base::{Join, JoinTuple};

/// Wait for the first future in a tuple to successfully complete.
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
///     future::race_ok((
///         completion_async!(Err(())),
///         completion_async!(Ok::<_, ()>(3)),
///         std::future::pending::<Result<_, ()>>(),
///     ))
///     .await,
///     Ok(3),
/// );
/// # });
/// ```
///
/// If all the futures fail, this will return a tuple of the errors.
///
/// ```
/// use completion::{future, completion_async};
/// use futures_lite::future::yield_now;
///
/// # future::block_on(completion_async! {
/// assert_eq!(
///     future::race_ok((
///         completion_async!(Err::<(), _>(0)),
///         completion_async!(Err(1)),
///         completion_async!(Err(2)),
///     ))
///     .await,
///     Err((0, 1, 2)),
/// );
/// # });
/// ```
///
///
/// If multiple futures are immediately successfully ready, the earlier one will be chosen. However
/// after this polling will be fair.
///
/// ```
/// use completion::{future, completion_async};
/// use futures_lite::future::yield_now;
///
/// # future::block_on(completion_async! {
/// assert_eq!(
///     future::race_ok((
///         completion_async! {
///             yield_now().await;
///             Ok::<_, ()>(0)
///         },
///         completion_async!(Ok::<_, ()>(1)),
///         completion_async!(Err(2)),
///         completion_async!(Ok::<_, ()>(3)),
///     ))
///     .await,
///     Ok(1),
/// );
/// # });
/// ```
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub fn race_ok<T: RaceOkTuple>(futures: T) -> RaceOk<T> {
    RaceOk {
        inner: Join::new(futures.into_tuple()),
        _correct_debug_bounds: PhantomData,
    }
}

pin_project! {
    /// Future for [`race_ok`].
    #[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
    #[derive(Debug)]
    pub struct RaceOk<T: RaceOkTuple> {
        #[pin]
        inner: Join<T::JoinTuple>,
        _correct_debug_bounds: PhantomData<(T::Futures, T::Ok)>,
    }
}

impl<T: RaceOkTuple> CompletionFuture for RaceOk<T> {
    type Output = Result<T::Ok, <T::JoinTuple as JoinTuple>::Output>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().inner.poll(cx).map(|flow| match flow {
            ControlFlow::Continue(errors) => Err(errors),
            ControlFlow::Break(val) => Ok(val),
        })
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.project().inner.poll_cancel(cx)
    }
}

pin_project! {
    /// A wrapper for a future inside a [`RaceOk`].
    #[derive(Debug)]
    pub struct Racing<F> {
        #[pin]
        inner: F,
    }
}
impl<F> Racing<F> {
    fn new(inner: F) -> Self {
        Self { inner }
    }
}
impl<F, T, E> CompletionFuture for Racing<F>
where
    F: CompletionFuture<Output = Result<T, E>>,
{
    type Output = ControlFlow<T, E>;
    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().inner.poll(cx).map(|res| match res {
            Ok(val) => ControlFlow::Break(val),
            Err(e) => ControlFlow::Continue(e),
        })
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.project().inner.poll_cancel(cx)
    }
}

/// A tuple of futures that can be used in `RaceOk`.
pub trait RaceOkTuple {
    /// The tuple that can be used with `Join`.
    type JoinTuple: JoinTuple<Futures = Self::Futures, Break = Self::Ok>;
    fn into_tuple(self) -> Self::JoinTuple;

    type Futures;
    type Ok;
}

macro_rules! impl_race_tuple {
    ($($param:ident),*) => {
        impl<Ok, $($param,)*> RaceOkTuple for ($($param,)*)
        where
            $($param: TryFuture<Ok = Ok>,)*
        {
            type JoinTuple = ($(Racing<$param>,)*);
            fn into_tuple(self) -> Self::JoinTuple {
                let ($($param,)*) = self;
                ($(Racing::new($param),)*)
            }

            type Futures = <Self::JoinTuple as JoinTuple>::Futures;
            type Ok = Ok;
        }
    }
}
apply_on_tuples!(impl_race_tuple!);
