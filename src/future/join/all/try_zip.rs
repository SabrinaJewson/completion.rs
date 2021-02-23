use core::fmt::{self, Debug, Formatter};
use core::iter::{FromIterator, FusedIterator};
use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::CompletionFuture;

use crate::future::CompletionFutureExt;
use crate::stream::{FromCompletionStream, FromCompletionStreamInner};

use super::super::{ControlFlow, FutureState, TryFuture, TryZipFuture};
use super::base::{JoinAll, JoinAllOutput};

/// Wait for all the futures in an iterator to successfully complete or one to return an error.
///
/// On success, this outputs a [`TryZipAllOutput`].
///
/// Requires the `std` feature, as [`std::panic::catch_unwind`] is needed when polling the futures
/// to ensure soundness.
///
/// # Examples
///
/// ```
/// use completion::{future, completion_async_move};
///
/// # future::block_on(completion::completion_async! {
/// assert_eq!(
///     future::try_zip_all(
///         [1, 2, 3, 4]
///             .iter()
///             .map(|&i| completion_async_move!(Ok::<_, ()>(i * 2)))
///     )
///     .await
///     .unwrap()
///     .collect::<Vec<_>>(),
///     vec![2, 4, 6, 8],
/// );
/// assert_eq!(
///     future::try_zip_all(
///         [1, 2, 3, 4]
///             .iter()
///             .map(|&i| completion_async_move! {
///                 if i == 3 {
///                     Err("oh no")
///                 } else {
///                     Ok(i * 2)
///                 }
///             })
///     )
///     .await
///     .unwrap_err(),
///     "oh no",
/// );
/// # });
/// ```
///
/// As [`TryZipAll`] implements [`FromIterator`], it can also be used with [`Iterator::collect`]:
///
/// ```
/// use completion::{future, completion_async_move};
///
/// # future::block_on(completion::completion_async! {
/// assert_eq!(
///     [1, 2, 3, 4]
///         .iter()
///         .map(|&i| completion_async_move!(Ok::<_, ()>(i * 2)))
///         .collect::<future::TryZipAll<_>>()
///         .await
///         .unwrap()
///         .collect::<Vec<_>>(),
///     vec![2, 4, 6, 8],
/// );
/// # });
/// ```
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub fn try_zip_all<I>(iter: I) -> TryZipAll<I::Item>
where
    I: IntoIterator,
    I::Item: TryFuture,
{
    iter.into_iter().collect()
}

/// Future for [`try_zip_all`]. On success, this outputs a [`TryZipAllOutput`].
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
#[derive(Debug)]
pub struct TryZipAll<F: TryFuture> {
    inner: JoinAll<TryZipFuture<F>>,
    _correct_debug_bounds: PhantomData<(F::Ok, F::Error)>,
}

impl<F: TryFuture> Unpin for TryZipAll<F> {}

impl<F: TryFuture> TryZipAll<F> {
    fn new(inner: JoinAll<TryZipFuture<F>>) -> Self {
        Self {
            inner,
            _correct_debug_bounds: PhantomData,
        }
    }
}

impl<F: TryFuture> FromIterator<F> for TryZipAll<F> {
    fn from_iter<T: IntoIterator<Item = F>>(iter: T) -> Self {
        Self::new(JoinAll::new(iter.into_iter().map(TryZipFuture::new)))
    }
}

impl<F: TryFuture> FromCompletionStream<F> for TryZipAll<F> {}
impl<F: TryFuture> FromCompletionStreamInner<F> for TryZipAll<F> {
    type Intermediate = Vec<FutureState<TryZipFuture<F>>>;
    fn start(lower: usize, upper: Option<usize>) -> Self::Intermediate {
        <JoinAll<TryZipFuture<F>>>::start(lower, upper)
    }
    fn push(intermediate: Self::Intermediate, item: F) -> Result<Self::Intermediate, Self> {
        <JoinAll<TryZipFuture<F>>>::push(intermediate, TryZipFuture::new(item)).map_err(Self::new)
    }
    fn finalize(intermediate: Self::Intermediate) -> Self {
        Self::new(<JoinAll<TryZipFuture<F>>>::finalize(intermediate))
    }
}

impl<F: TryFuture> CompletionFuture for TryZipAll<F> {
    type Output = Result<TryZipAllOutput<F>, F::Error>;

    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.inner.poll(cx).map(|flow| match flow {
            ControlFlow::Continue(val) => Ok(TryZipAllOutput(val)),
            ControlFlow::Break(e) => Err(e),
        })
    }
    unsafe fn poll_cancel(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.inner.poll_cancel(cx)
    }
}

/// An iterator over the successful outputs of futures in a [`TryZipAll`].
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub struct TryZipAllOutput<F: TryFuture>(JoinAllOutput<TryZipFuture<F>>);

impl<F: TryFuture> Iterator for TryZipAllOutput<F> {
    type Item = F::Ok;

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
impl<F: TryFuture> ExactSizeIterator for TryZipAllOutput<F> {
    fn len(&self) -> usize {
        self.0.len()
    }
}
impl<F: TryFuture> DoubleEndedIterator for TryZipAllOutput<F> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back()
    }
}
impl<F: TryFuture> FusedIterator for TryZipAllOutput<F> {}

impl<F: TryFuture> Debug for TryZipAllOutput<F>
where
    F::Ok: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_tuple("TryZipAllOutput").field(&self.0).finish()
    }
}
