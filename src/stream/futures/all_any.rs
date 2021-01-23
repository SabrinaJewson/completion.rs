//! `All` and `Any`.

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::{CompletionFuture, CompletionStream};
use futures_core::{ready, Stream};

use super::super::CompletionStreamExt;

/// Future for [`CompletionStreamExt::all`].
#[derive(Debug)]
pub struct All<'a, S: ?Sized, F> {
    stream: &'a mut S,
    f: F,
}

impl<'a, S: ?Sized, F> All<'a, S, F> {
    pub(crate) fn new(stream: &'a mut S, f: F) -> Self {
        Self { stream, f }
    }
}

impl<S: Unpin + ?Sized, F> Unpin for All<'_, S, F> {}

impl<S: Unpin + ?Sized, F> CompletionFuture for All<'_, S, F>
where
    S: CompletionStream,
    F: FnMut(S::Item) -> bool,
{
    type Output = bool;

    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        while let Some(item) = ready!(self.stream.poll_next(cx)) {
            if !(self.f)(item) {
                return Poll::Ready(false);
            }
        }
        Poll::Ready(true)
    }
}

impl<S: Unpin + ?Sized, F> Future for All<'_, S, F>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    F: FnMut(<S as CompletionStream>::Item) -> bool,
{
    type Output = bool;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

/// Future for [`CompletionStreamExt::any`].
#[derive(Debug)]
pub struct Any<'a, S: ?Sized, F> {
    stream: &'a mut S,
    f: F,
}

impl<'a, S: ?Sized, F> Any<'a, S, F> {
    pub(crate) fn new(stream: &'a mut S, f: F) -> Self {
        Self { stream, f }
    }
}

impl<S: Unpin + ?Sized, F> Unpin for Any<'_, S, F> {}

impl<S: Unpin + ?Sized, F> CompletionFuture for Any<'_, S, F>
where
    S: CompletionStream,
    F: FnMut(S::Item) -> bool,
{
    type Output = bool;

    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        while let Some(item) = ready!(self.stream.poll_next(cx)) {
            if (self.f)(item) {
                return Poll::Ready(true);
            }
        }
        Poll::Ready(false)
    }
}

impl<S: Unpin + ?Sized, F> Future for Any<'_, S, F>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    F: FnMut(<S as CompletionStream>::Item) -> bool,
{
    type Output = bool;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}
