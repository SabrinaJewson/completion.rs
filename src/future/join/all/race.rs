use core::iter::FromIterator;
use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::CompletionFuture;

use crate::future::CompletionFutureExt;
use crate::stream::{FromCompletionStream, FromCompletionStreamInner};

use super::super::{ControlFlow, FutureState, RaceFuture};
use super::base::JoinAll;

/// Wait for the first future in an iterator to complete.
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
///     future::race_all(
///         [7, 3, 2, 6]
///             .iter()
///             .map(|&i| completion_async_move! {
///                 for _ in 0..i {
///                     yield_now().await;
///                 }
///                 i
///             })
///     )
///     .await,
///     2,
/// );
/// # });
/// ```
///
/// As [`RaceAll`] implements [`FromIterator`], it can also be used with [`Iterator::collect`]:
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
///             for _ in 0..i {
///                 yield_now().await;
///             }
///             i
///         })
///         .collect::<future::RaceAll<_>>()
///         .await,
///     2,
/// );
/// # });
/// ```
///
/// If multiple futures are immediately ready, the earlier one will be chosen. However after this
/// polling will be fair.
///
/// ```
/// use completion::{future, completion_async_move};
///
/// # future::block_on(completion::completion_async! {
/// assert_eq!(
///     future::race_all([7, 3, 2, 6].iter().copied().map(std::future::ready)).await,
///     7,
/// );
/// # });
/// ```
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub fn race_all<I>(iter: I) -> RaceAll<I::Item>
where
    I: IntoIterator,
    I::Item: CompletionFuture,
{
    iter.into_iter().collect()
}

/// Future for [`race_all`].
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
#[derive(Debug)]
pub struct RaceAll<F: CompletionFuture> {
    inner: JoinAll<RaceFuture<F>>,
    _correct_debug_bounds: PhantomData<F::Output>,
}

impl<F: CompletionFuture> Unpin for RaceAll<F> {}

impl<F: CompletionFuture> RaceAll<F> {
    fn new(inner: JoinAll<RaceFuture<F>>) -> Self {
        Self {
            inner,
            _correct_debug_bounds: PhantomData,
        }
    }
}

impl<F: CompletionFuture> FromIterator<F> for RaceAll<F> {
    fn from_iter<T: IntoIterator<Item = F>>(iter: T) -> Self {
        Self::new(JoinAll::new(iter.into_iter().map(RaceFuture::new)))
    }
}

impl<F: CompletionFuture> FromCompletionStream<F> for RaceAll<F> {}
impl<F: CompletionFuture> FromCompletionStreamInner<F> for RaceAll<F> {
    type Intermediate = Vec<FutureState<RaceFuture<F>>>;
    fn start(lower: usize, upper: Option<usize>) -> Self::Intermediate {
        <JoinAll<RaceFuture<F>>>::start(lower, upper)
    }
    fn push(intermediate: Self::Intermediate, item: F) -> Result<Self::Intermediate, Self> {
        <JoinAll<RaceFuture<F>>>::push(intermediate, RaceFuture::new(item)).map_err(Self::new)
    }
    fn finalize(intermediate: Self::Intermediate) -> Self {
        Self::new(<JoinAll<RaceFuture<F>>>::finalize(intermediate))
    }
}

impl<F: CompletionFuture> CompletionFuture for RaceAll<F> {
    type Output = F::Output;

    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.inner.poll(cx).map(|flow| match flow {
            // This type contains an `Infallible`.
            ControlFlow::Continue(_) => unreachable!(),
            ControlFlow::Break(val) => val,
        })
    }
    unsafe fn poll_cancel(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.inner.poll_cancel(cx)
    }
}
