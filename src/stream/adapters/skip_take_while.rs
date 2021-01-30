//! `SkipWhile` and `TakeWhile`.

use core::{
    pin::Pin,
    task::{Context, Poll},
};

use completion_core::CompletionStream;
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

pin_project! {
    /// Stream for [`CompletionStreamExt::skip_while`](crate::CompletionStreamExt::skip_while).
    #[derive(Debug, Clone)]
    pub struct SkipWhile<S, P> {
        #[pin]
        stream: S,
        skipping: bool,
        predicate: P,
    }
}

impl<S, P> SkipWhile<S, P> {
    pub(crate) fn new(stream: S, predicate: P) -> Self {
        Self {
            stream,
            skipping: true,
            predicate,
        }
    }
}

impl<S: CompletionStream, P> CompletionStream for SkipWhile<S, P>
where
    P: FnMut(&S::Item) -> bool,
{
    type Item = S::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        loop {
            match ready!(this.stream.as_mut().poll_next(cx)) {
                Some(item) => {
                    if *this.skipping {
                        *this.skipping = (this.predicate)(&item);
                    }
                    if !*this.skipping {
                        break Poll::Ready(Some(item));
                    }
                }
                None => break Poll::Ready(None),
            }
        }
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.project().stream.poll_cancel(cx)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.skipping {
            (0, self.stream.size_hint().1)
        } else {
            self.stream.size_hint()
        }
    }
}

impl<S, P> Stream for SkipWhile<S, P>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    P: FnMut(&<S as CompletionStream>::Item) -> bool,
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
    /// Stream for [`CompletionStreamExt::take_while`](crate::CompletionStreamExt::take_while).
    #[derive(Debug, Clone)]
    pub struct TakeWhile<S, P> {
        #[pin]
        stream: S,
        taking: bool,
        predicate: P,
    }
}

impl<S, P> TakeWhile<S, P> {
    pub(crate) fn new(stream: S, predicate: P) -> Self {
        Self {
            stream,
            taking: true,
            predicate,
        }
    }
}

impl<S: CompletionStream, P> CompletionStream for TakeWhile<S, P>
where
    P: FnMut(&S::Item) -> bool,
{
    type Item = S::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();

        if *this.taking {
            match ready!(this.stream.poll_next(cx)) {
                Some(item) => {
                    if (this.predicate)(&item) {
                        return Poll::Ready(Some(item));
                    }
                    *this.taking = false;
                }
                None => return Poll::Ready(None),
            }
        }
        Poll::Ready(None)
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.project();
        if *this.taking {
            this.stream.poll_cancel(cx)
        } else {
            Poll::Ready(())
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.taking {
            (0, self.stream.size_hint().1)
        } else {
            (0, Some(0))
        }
    }
}

impl<S, P> Stream for TakeWhile<S, P>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    P: FnMut(&<S as CompletionStream>::Item) -> bool,
{
    type Item = <Self as CompletionStream>::Item;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}
