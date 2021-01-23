//! `Skip` and `Take`.

use core::cmp;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::CompletionStream;
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

#[cfg(doc)]
use super::super::CompletionStreamExt;

pin_project! {
    /// Stream for [`CompletionStreamExt::skip`].
    #[derive(Debug, Clone)]
    pub struct Skip<S> {
        #[pin]
        stream: S,
        to_skip: usize,
    }
}

impl<S> Skip<S> {
    pub(crate) fn new(stream: S, to_skip: usize) -> Self {
        Self { stream, to_skip }
    }
}

impl<S: CompletionStream> CompletionStream for Skip<S> {
    type Item = S::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        while *this.to_skip > 0 {
            if ready!(this.stream.as_mut().poll_next(cx)).is_none() {
                return Poll::Ready(None);
            }
            *this.to_skip -= 1;
        }
        this.stream.poll_next(cx)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let (lower, upper) = self.stream.size_hint();
        (
            lower.saturating_sub(self.to_skip),
            upper.map(|upper| upper.saturating_sub(self.to_skip)),
        )
    }
}

impl<S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>> Stream for Skip<S> {
    type Item = <Self as CompletionStream>::Item;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}

pin_project! {
    /// Stream for [`CompletionStreamExt::take`].
    #[derive(Debug, Clone)]
    pub struct Take<S> {
        #[pin]
        stream: S,
        to_take: usize,
    }
}

impl<S> Take<S> {
    pub(crate) fn new(stream: S, to_take: usize) -> Self {
        Self { stream, to_take }
    }
}

impl<S: CompletionStream> CompletionStream for Take<S> {
    type Item = S::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();

        Poll::Ready(if *this.to_take == 0 {
            None
        } else {
            let item = ready!(this.stream.poll_next(cx));
            *this.to_take -= 1;
            item
        })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.to_take == 0 {
            (0, Some(0))
        } else {
            let (lower, upper) = self.stream.size_hint();
            (
                cmp::min(lower, self.to_take),
                Some(upper.map_or(self.to_take, |upper| cmp::min(upper, self.to_take))),
            )
        }
    }
}

impl<S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>> Stream for Take<S> {
    type Item = <Self as CompletionStream>::Item;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}
