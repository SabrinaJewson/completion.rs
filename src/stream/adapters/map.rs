//! `Map` and `Then`.

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::{CompletionFuture, CompletionStream};
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

#[cfg(doc)]
use super::super::CompletionStreamExt;

pin_project! {
    /// Stream for [`CompletionStreamExt::map`].
    #[derive(Debug, Clone)]
    pub struct Map<S, F> {
        #[pin]
        stream: S,
        f: F,
    }
}

impl<S, F> Map<S, F> {
    pub(crate) fn new(stream: S, f: F) -> Self {
        Self { stream, f }
    }
}

impl<S, F, T> CompletionStream for Map<S, F>
where
    S: CompletionStream,
    F: FnMut(S::Item) -> T,
{
    type Item = T;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        this.stream
            .as_mut()
            .poll_next(cx)
            .map(|value| value.map(this.f))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.stream.size_hint()
    }
}

impl<S, F, T> Stream for Map<S, F>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    F: FnMut(<S as CompletionStream>::Item) -> T,
{
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(&self.stream)
    }
}

pin_project! {
    /// Stream for [`CompletionStreamExt::then`].
    #[derive(Debug, Clone)]
    pub struct Then<S, F, Fut> {
        #[pin]
        stream: S,
        #[pin]
        fut: Option<Fut>,
        f: F,
    }
}

impl<S, F, Fut> Then<S, F, Fut> {
    pub(crate) fn new(stream: S, f: F) -> Self {
        Self {
            stream,
            fut: None,
            f,
        }
    }
}

impl<S, F, Fut> CompletionStream for Then<S, F, Fut>
where
    S: CompletionStream,
    F: FnMut(S::Item) -> Fut,
    Fut: CompletionFuture,
{
    type Item = Fut::Output;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        let fut = match this.fut.as_mut().as_pin_mut() {
            Some(fut) => fut,
            None => match ready!(this.stream.poll_next(cx)) {
                Some(item) => {
                    this.fut.set(Some((this.f)(item)));
                    this.fut.as_mut().as_pin_mut().unwrap()
                }
                None => {
                    return Poll::Ready(None);
                }
            },
        };

        let val = ready!(fut.poll(cx));
        this.fut.set(None);
        Poll::Ready(Some(val))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let future_len = if self.fut.is_some() { 1 } else { 0 };
        let (stream_min, stream_max) = self.stream.size_hint();
        (
            stream_min.saturating_add(future_len),
            stream_max.and_then(|l| l.checked_add(future_len)),
        )
    }
}

impl<S, F, Fut> Stream for Then<S, F, Fut>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    F: FnMut(<S as CompletionStream>::Item) -> Fut,
    Fut: CompletionFuture + Future<Output = <Fut as CompletionFuture>::Output>,
{
    type Item = <Fut as CompletionFuture>::Output;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}
