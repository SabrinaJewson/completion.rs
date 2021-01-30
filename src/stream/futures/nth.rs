use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::{CompletionFuture, CompletionStream};
use futures_core::{ready, Stream};

use super::super::CompletionStreamExt;

/// Future for [`CompletionStreamExt::nth`].
#[derive(Debug)]
pub struct Nth<'a, S: ?Sized> {
    stream: &'a mut S,
    n: usize,
}

impl<'a, S: ?Sized> Nth<'a, S> {
    pub(crate) fn new(stream: &'a mut S, n: usize) -> Self {
        Self { stream, n }
    }
}

impl<S: Unpin + ?Sized> Unpin for Nth<'_, S> {}

impl<S: Unpin + ?Sized> CompletionFuture for Nth<'_, S>
where
    S: CompletionStream,
{
    type Output = Option<S::Item>;

    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match ready!(self.stream.poll_next(cx)) {
                Some(v) => {
                    self.n = match self.n.checked_sub(1) {
                        Some(n) => n,
                        None => return Poll::Ready(Some(v)),
                    }
                }
                None => return Poll::Ready(None),
            }
        }
    }
    unsafe fn poll_cancel(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.stream.poll_cancel(cx)
    }
}

impl<S: Unpin + ?Sized> Future for Nth<'_, S>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
{
    type Output = Option<<S as CompletionStream>::Item>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}
