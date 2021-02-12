use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::CompletionFuture;
use pin_project_lite::pin_project;

use super::super::{ControlFlow, TryFuture};
use super::base::{Join, JoinTuple};

/// Wait for all the futures in a tuple to successfully complete or one to return an error.
///
/// This takes any tuple of two or more futures. On success it outputs a tuple of the results, and
/// on failure it returns the error.
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
///     future::try_zip((
///         completion_async!(Ok::<_, ()>(5)),
///         completion_async!(Ok(vec![5, 7])),
///     ))
///     .await,
///     Ok((5, vec![5, 7])),
/// );
/// assert_eq!(
///     future::try_zip((
///         completion_async!(Ok(5)),
///         completion_async!(Ok(6)),
///         completion_async!(Err::<(), _>(7)),
///         completion_async!(Ok(8)),
///     ))
///     .await,
///     Err(7),
/// );
/// # });
/// ```
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub fn try_zip<T: TryZipTuple>(futures: T) -> TryZip<T> {
    TryZip {
        inner: Join::new(futures.into_tuple()),
        _correct_debug_bounds: PhantomData,
    }
}

pin_project! {
    /// Future for [`try_zip`].
    #[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
    #[derive(Debug)]
    pub struct TryZip<T: TryZipTuple> {
        #[pin]
        inner: Join<T::JoinTuple>,
        _correct_debug_bounds: PhantomData<(T::Futures, T::Error)>,
    }
}

impl<T: TryZipTuple> CompletionFuture for TryZip<T> {
    type Output = Result<<T::JoinTuple as JoinTuple>::Output, <T::JoinTuple as JoinTuple>::Break>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().inner.poll(cx).map(|flow| match flow {
            ControlFlow::Continue(val) => Ok(val),
            ControlFlow::Break(e) => Err(e),
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
impl<F, T, E> CompletionFuture for Zipped<F>
where
    F: CompletionFuture<Output = Result<T, E>>,
{
    type Output = ControlFlow<E, T>;
    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().inner.poll(cx).map(|res| match res {
            Ok(val) => ControlFlow::Continue(val),
            Err(e) => ControlFlow::Break(e),
        })
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.project().inner.poll_cancel(cx)
    }
}

/// A tuple of futures that can be used in `Zip`.
pub trait TryZipTuple {
    /// The tuple that can be used with `Join`.
    type JoinTuple: JoinTuple<Futures = Self::Futures, Break = Self::Error>;
    fn into_tuple(self) -> Self::JoinTuple;

    type Futures;
    type Error;
}

macro_rules! impl_try_zip_tuple {
    ($($param:ident),*) => {
        impl<Error, $($param,)*> TryZipTuple for ($($param,)*)
        where
            $($param: TryFuture<Error = Error>,)*
        {
            type JoinTuple = ($(Zipped<$param>,)*);
            fn into_tuple(self) -> Self::JoinTuple {
                let ($($param,)*) = self;
                ($(Zipped::new($param),)*)
            }

            type Futures = <Self::JoinTuple as JoinTuple>::Futures;
            type Error = <Self::JoinTuple as JoinTuple>::Break;
        }
    }
}
apply_on_tuples!(impl_try_zip_tuple!);
