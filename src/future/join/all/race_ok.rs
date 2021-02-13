use core::fmt::{self, Debug, Formatter};
use core::iter::{FromIterator, FusedIterator};
use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::CompletionFuture;

use crate::future::CompletionFutureExt;
use crate::stream::{FromCompletionStream, FromCompletionStreamInner};

use super::super::{ControlFlow, FutureState, RaceOkFuture, TryFuture};
use super::base::{JoinAll, JoinAllOutput};

/// Wait for the first future in an iterator to successfully complete.
///
/// If all the futures fail, this will return all the errors in a [`RaceOkAllErrors`].
///
/// Requires the `std` feature, as [`std::panic::catch_unwind`] is needed when polling the futures
/// to ensure soundness.
///
/// # Examples
///
/// ```
/// use completion::{future, completion_async_move};
/// use futures_lite::future::yield_now;
///
/// # future::block_on(completion::completion_async! {
/// assert_eq!(
///     future::race_ok_all(
///         [7, 3, 2, 6]
///             .iter()
///             .map(|&i| completion_async_move! {
///                 if i < 3 {
///                     return Err("oh no");
///                 }
///                 for _ in 0..i {
///                     yield_now().await;
///                 }
///                 Ok(i)
///             })
///     )
///     .await
///     .unwrap(),
///     3,
/// );
/// # });
/// ```
///
/// If all the futures fail, this will return an iterator over the errors.
///
/// ```
/// use completion::{future, completion_async_move};
///
/// # future::block_on(completion::completion_async! {
/// assert_eq!(
///     future::race_ok_all(
///         ["lots", "of", "errors"]
///             .iter()
///             .map(|&s| completion_async_move! {
///                 Err::<(), _>(s)
///             })
///     )
///     .await
///     .unwrap_err()
///     .collect::<Vec<_>>(),
///     vec!["lots", "of", "errors"],
/// );
/// # });
/// ```
///
/// As [`RaceOkAll`] implements [`FromIterator`], it can also be used with [`Iterator::collect`]:
///
/// ```
/// use completion::{future, completion_async_move};
/// use futures_lite::future::yield_now;
///
/// # future::block_on(completion::completion_async! {
/// assert_eq!(
///     [7, 3, 2, 6]
///         .iter()
///         .map(|&i| completion_async_move! {
///             if i < 3 {
///                 return Err("oh no");
///             }
///             for _ in 0..i {
///                 yield_now().await;
///             }
///             Ok(i)
///         })
///         .collect::<future::RaceOkAll<_>>()
///         .await
///         .unwrap(),
///     3,
/// );
/// # });
/// ```
///
/// If multiple futures are immediately successfully ready, the earlier one will be chosen. However
/// after this polling will be fair.
///
/// ```
/// use completion::{future, completion_async_move};
///
/// # future::block_on(completion::completion_async! {
/// assert_eq!(
///     future::race_ok_all([Ok::<_, ()>(1), Ok(2), Ok(3)].iter().copied().map(std::future::ready))
///     .await
///     .unwrap(),
///     1,
/// );
/// # });
/// ```
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub fn race_ok_all<I>(iter: I) -> RaceOkAll<I::Item>
where
    I: IntoIterator,
    I::Item: TryFuture,
{
    iter.into_iter().collect()
}

/// Future for [`race_ok_all`]. If all the futures fail, this returns a [`RaceOkAllErrors`].
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
#[derive(Debug)]
pub struct RaceOkAll<F: TryFuture> {
    inner: JoinAll<RaceOkFuture<F>>,
    _correct_debug_bounds: PhantomData<(F::Ok, F::Error)>,
}

impl<F: TryFuture> Unpin for RaceOkAll<F> {}

impl<F: TryFuture> RaceOkAll<F> {
    fn new(inner: JoinAll<RaceOkFuture<F>>) -> Self {
        Self {
            inner,
            _correct_debug_bounds: PhantomData,
        }
    }
}

impl<F: TryFuture> FromIterator<F> for RaceOkAll<F> {
    fn from_iter<T: IntoIterator<Item = F>>(iter: T) -> Self {
        Self::new(JoinAll::new(iter.into_iter().map(RaceOkFuture::new)))
    }
}

impl<F: TryFuture> FromCompletionStream<F> for RaceOkAll<F> {}
impl<F: TryFuture> FromCompletionStreamInner<F> for RaceOkAll<F> {
    type Intermediate = Vec<FutureState<RaceOkFuture<F>>>;
    fn start(lower: usize, upper: Option<usize>) -> Self::Intermediate {
        <JoinAll<RaceOkFuture<F>>>::start(lower, upper)
    }
    fn push(intermediate: Self::Intermediate, item: F) -> Result<Self::Intermediate, Self> {
        <JoinAll<RaceOkFuture<F>>>::push(intermediate, RaceOkFuture::new(item)).map_err(Self::new)
    }
    fn finalize(intermediate: Self::Intermediate) -> Self {
        Self::new(<JoinAll<RaceOkFuture<F>>>::finalize(intermediate))
    }
}

impl<F: TryFuture> CompletionFuture for RaceOkAll<F> {
    type Output = Result<F::Ok, RaceOkAllErrors<F>>;

    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.inner.poll(cx).map(|flow| match flow {
            ControlFlow::Continue(errors) => Err(RaceOkAllErrors(errors)),
            ControlFlow::Break(val) => Ok(val),
        })
    }
    unsafe fn poll_cancel(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.inner.poll_cancel(cx)
    }
}

/// An iterator over the errors of the futures in a [`RaceOkAll`].
pub struct RaceOkAllErrors<F: TryFuture>(JoinAllOutput<RaceOkFuture<F>>);

impl<F: TryFuture> Iterator for RaceOkAllErrors<F> {
    type Item = F::Error;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
    fn count(self) -> usize {
        self.0.count()
    }
}
impl<F: TryFuture> ExactSizeIterator for RaceOkAllErrors<F> {
    fn len(&self) -> usize {
        self.0.len()
    }
}
impl<F: TryFuture> DoubleEndedIterator for RaceOkAllErrors<F> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back()
    }
}
impl<F: TryFuture> FusedIterator for RaceOkAllErrors<F> {}

impl<F: TryFuture> Debug for RaceOkAllErrors<F>
where
    F::Error: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_tuple("RaceOkAllErrors").field(&self.0).finish()
    }
}
