//! `Find`, `FindMap` and `Position`.

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::{CompletionFuture, CompletionStream};
use futures_core::{ready, Stream};

use crate::CompletionStreamExt;

/// Future for [`CompletionStreamExt::find`].
#[derive(Debug)]
pub struct Find<'a, S: ?Sized, P> {
    stream: &'a mut S,
    predicate: P,
}

impl<S: Unpin + ?Sized, P> Unpin for Find<'_, S, P> {}

impl<'a, S: ?Sized, P> Find<'a, S, P> {
    pub(crate) fn new(stream: &'a mut S, predicate: P) -> Self {
        Self { stream, predicate }
    }
}

impl<S: Unpin + ?Sized, P> CompletionFuture for Find<'_, S, P>
where
    S: CompletionStream,
    P: FnMut(&S::Item) -> bool,
{
    type Output = Option<S::Item>;

    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(loop {
            match ready!(self.stream.poll_next(cx)) {
                Some(item) if (self.predicate)(&item) => break Some(item),
                Some(_) => {}
                None => break None,
            }
        })
    }
    unsafe fn poll_cancel(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.stream.poll_cancel(cx)
    }
}

impl<S: Unpin + ?Sized, P> Future for Find<'_, S, P>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    P: FnMut(&<S as CompletionStream>::Item) -> bool,
{
    type Output = <Self as CompletionFuture>::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

/// Future for [`CompletionStreamExt::find_map`].
#[derive(Debug)]
pub struct FindMap<'a, S: ?Sized, F> {
    stream: &'a mut S,
    f: F,
}

impl<S: Unpin + ?Sized, F> Unpin for FindMap<'_, S, F> {}

impl<'a, S: ?Sized, F> FindMap<'a, S, F> {
    pub(crate) fn new(stream: &'a mut S, f: F) -> Self {
        Self { stream, f }
    }
}

impl<S: Unpin + ?Sized, F, B> CompletionFuture for FindMap<'_, S, F>
where
    S: CompletionStream,
    F: FnMut(S::Item) -> Option<B>,
{
    type Output = Option<B>;
    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(loop {
            match ready!(self.stream.poll_next(cx)) {
                Some(item) => {
                    if let Some(mapped) = (self.f)(item) {
                        break Some(mapped);
                    }
                }
                None => break None,
            }
        })
    }
    unsafe fn poll_cancel(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.stream.poll_cancel(cx)
    }
}

impl<S: Unpin + ?Sized, F, B> Future for FindMap<'_, S, F>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    F: FnMut(<S as CompletionStream>::Item) -> Option<B>,
{
    type Output = <Self as CompletionFuture>::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

/// Future for [`CompletionStreamExt::position`].
#[derive(Debug)]
pub struct Position<'a, S: ?Sized, P> {
    stream: &'a mut S,
    predicate: P,
    position: usize,
}

impl<S: Unpin + ?Sized, P> Unpin for Position<'_, S, P> {}

impl<'a, S: ?Sized, P> Position<'a, S, P> {
    pub(crate) fn new(stream: &'a mut S, predicate: P) -> Self {
        Self {
            stream,
            predicate,
            position: 0,
        }
    }
}

impl<S: Unpin + ?Sized, P> CompletionFuture for Position<'_, S, P>
where
    S: CompletionStream,
    P: FnMut(S::Item) -> bool,
{
    type Output = Option<usize>;
    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(loop {
            match ready!(self.stream.poll_next(cx)) {
                Some(item) => {
                    if (self.predicate)(item) {
                        break Some(self.position);
                    }
                    self.position += 1;
                }
                None => break None,
            }
        })
    }
    unsafe fn poll_cancel(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.stream.poll_cancel(cx)
    }
}

impl<S: Unpin + ?Sized, P> Future for Position<'_, S, P>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    P: FnMut(<S as CompletionStream>::Item) -> bool,
{
    type Output = <Self as CompletionFuture>::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}
