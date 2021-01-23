//! `Filter` and `FilterMap`.

use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::CompletionStream;
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

#[cfg(doc)]
use super::super::CompletionStreamExt;

pin_project! {
    /// Stream for [`CompletionStreamExt::filter`].
    #[derive(Debug, Clone)]
    pub struct Filter<S, F> {
        #[pin]
        stream: S,
        f: F,
    }
}

impl<S, F> Filter<S, F> {
    pub(crate) fn new(stream: S, f: F) -> Self {
        Self { stream, f }
    }
}

impl<S: CompletionStream, F: FnMut(&S::Item) -> bool> CompletionStream for Filter<S, F> {
    type Item = S::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        loop {
            match ready!(this.stream.as_mut().poll_next(cx)) {
                Some(item) if (this.f)(&item) => break Poll::Ready(Some(item)),
                Some(_) => {}
                None => break Poll::Ready(None),
            }
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let (_, high) = self.stream.size_hint();
        (0, high)
    }
}

impl<S, F: FnMut(&<S as CompletionStream>::Item) -> bool> Stream for Filter<S, F>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
{
    type Item = <Self as CompletionStream>::Item;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}

pin_project! {
    /// Stream for [`CompletionStreamExt::filter_map`].
    #[derive(Debug, Clone)]
    pub struct FilterMap<S, F> {
        #[pin]
        stream: S,
        f: F,
    }
}

impl<S, F> FilterMap<S, F> {
    pub(crate) fn new(stream: S, f: F) -> Self {
        Self { stream, f }
    }
}

impl<S, F, T> CompletionStream for FilterMap<S, F>
where
    S: CompletionStream,
    F: FnMut(S::Item) -> Option<T>,
{
    type Item = T;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        loop {
            match ready!(this.stream.as_mut().poll_next(cx)) {
                Some(item) => {
                    if let Some(mapped) = (this.f)(item) {
                        break Poll::Ready(Some(mapped));
                    }
                }
                None => break Poll::Ready(None),
            }
        }
    }
}

impl<S, F, T> Stream for FilterMap<S, F>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    F: FnMut(<S as CompletionStream>::Item) -> Option<T>,
{
    type Item = <Self as CompletionStream>::Item;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}
