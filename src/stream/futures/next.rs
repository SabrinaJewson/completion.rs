use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::{CompletionFuture, CompletionStream};
use futures_core::Stream;

use super::super::CompletionStreamExt;

/// Future for [`CompletionStreamExt::next`].
#[derive(Debug)]
pub struct Next<'a, S: ?Sized> {
    stream: &'a mut S,
}

impl<'a, S: ?Sized> Next<'a, S> {
    pub(crate) fn new(stream: &'a mut S) -> Self {
        Self { stream }
    }
}

impl<S: Unpin + ?Sized> Unpin for Next<'_, S> {}

impl<S: Unpin + ?Sized> CompletionFuture for Next<'_, S>
where
    S: CompletionStream,
{
    type Output = Option<S::Item>;

    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.stream.poll_next(cx)
    }
    unsafe fn poll_cancel(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.stream.poll_cancel(cx)
    }
}

impl<S: Unpin + ?Sized> Future for Next<'_, S>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
{
    type Output = Option<<S as CompletionStream>::Item>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}
