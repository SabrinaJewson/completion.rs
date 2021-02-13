use core::fmt::{self, Debug, Formatter};
use core::iter::{FromIterator, FusedIterator};
use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::CompletionFuture;

use crate::future::CompletionFutureExt;
use crate::stream::{FromCompletionStream, FromCompletionStreamInner};

use super::super::{ControlFlow, FutureState, ZipFuture};
use super::base::{JoinAll, JoinAllOutput};

/// Wait for all the futures in an iterator to complete.
///
/// This outputs a [`ZipAllOutput`].
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
///     future::zip_all(
///         [1, 2, 3, 4]
///             .iter()
///             .map(|&i| completion_async_move!(i * 2))
///     )
///     .await
///     .collect::<Vec<_>>(),
///     vec![2, 4, 6, 8],
/// );
/// # });
/// ```
///
/// As [`ZipAll`] implements [`FromIterator`], it can also be used with [`Iterator::collect`]:
///
/// ```
/// use completion::{future, completion_async_move};
///
/// # future::block_on(completion::completion_async! {
/// assert_eq!(
///     [1, 2, 3, 4]
///         .iter()
///         .map(|&i| completion_async_move!(i * 2))
///         .collect::<future::ZipAll<_>>()
///         .await
///         .collect::<Vec<_>>(),
///     vec![2, 4, 6, 8],
/// );
/// # });
/// ```
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub fn zip_all<I>(iter: I) -> ZipAll<I::Item>
where
    I: IntoIterator,
    I::Item: CompletionFuture,
{
    iter.into_iter().collect()
}

/// Future for [`zip_all`]. This outputs a [`ZipAllOutput`].
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
#[derive(Debug)]
pub struct ZipAll<F: CompletionFuture> {
    inner: JoinAll<ZipFuture<F>>,
    _correct_debug_bounds: PhantomData<F::Output>,
}

impl<F: CompletionFuture> Unpin for ZipAll<F> {}

impl<F: CompletionFuture> ZipAll<F> {
    fn new(inner: JoinAll<ZipFuture<F>>) -> Self {
        Self {
            inner,
            _correct_debug_bounds: PhantomData,
        }
    }
}

impl<F: CompletionFuture> FromIterator<F> for ZipAll<F> {
    fn from_iter<T: IntoIterator<Item = F>>(iter: T) -> Self {
        Self::new(JoinAll::new(iter.into_iter().map(ZipFuture::new)))
    }
}

impl<F: CompletionFuture> FromCompletionStream<F> for ZipAll<F> {}
impl<F: CompletionFuture> FromCompletionStreamInner<F> for ZipAll<F> {
    type Intermediate = Vec<FutureState<ZipFuture<F>>>;
    fn start(lower: usize, upper: Option<usize>) -> Self::Intermediate {
        <JoinAll<ZipFuture<F>>>::start(lower, upper)
    }
    fn push(intermediate: Self::Intermediate, item: F) -> Result<Self::Intermediate, Self> {
        <JoinAll<ZipFuture<F>>>::push(intermediate, ZipFuture::new(item)).map_err(Self::new)
    }
    fn finalize(intermediate: Self::Intermediate) -> Self {
        Self::new(<JoinAll<ZipFuture<F>>>::finalize(intermediate))
    }
}

impl<F: CompletionFuture> CompletionFuture for ZipAll<F> {
    type Output = ZipAllOutput<F>;

    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.inner.poll(cx).map(|flow| match flow {
            ControlFlow::Continue(val) => ZipAllOutput(val),
            ControlFlow::Break(no) => match no {},
        })
    }
    unsafe fn poll_cancel(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.inner.poll_cancel(cx)
    }
}

/// An iterator over the outputs of futures in a [`ZipAll`].
pub struct ZipAllOutput<F: CompletionFuture>(JoinAllOutput<ZipFuture<F>>);

impl<F: CompletionFuture> Iterator for ZipAllOutput<F> {
    type Item = F::Output;

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
impl<F: CompletionFuture> ExactSizeIterator for ZipAllOutput<F> {
    fn len(&self) -> usize {
        self.0.len()
    }
}
impl<F: CompletionFuture> DoubleEndedIterator for ZipAllOutput<F> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back()
    }
}
impl<F: CompletionFuture> FusedIterator for ZipAllOutput<F> {}

impl<F: CompletionFuture> Debug for ZipAllOutput<F>
where
    F::Output: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ZipAllOutput").field(&self.0).finish()
    }
}
