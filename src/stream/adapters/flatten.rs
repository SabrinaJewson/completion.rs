//! `FlatMap` and `Flatten`.

use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::CompletionStream;
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

use crate::CompletionStreamExt;

use super::Map;

pin_project! {
    /// Stream for [`CompletionStreamExt::flat_map`].
    #[derive(Debug, Clone)]
    pub struct FlatMap<S: CompletionStream, U, F>
    where
        F: FnMut(S::Item) -> U,
    {
        #[pin]
        inner: Flatten<Map<S, F>>,
    }
}

impl<S: CompletionStream, U: CompletionStream, F: FnMut(S::Item) -> U> FlatMap<S, U, F> {
    pub(crate) fn new(stream: S, f: F) -> Self {
        Self {
            inner: stream.map(f).flatten(),
        }
    }
}

impl<S, U, F> CompletionStream for FlatMap<S, U, F>
where
    S: CompletionStream,
    U: CompletionStream,
    F: FnMut(S::Item) -> U,
{
    type Item = U::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().inner.poll_next(cx)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<S, U, F> Stream for FlatMap<S, U, F>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    U: CompletionStream + Stream<Item = <U as CompletionStream>::Item>,
    F: FnMut(<S as CompletionStream>::Item) -> U,
{
    type Item = <Self as CompletionStream>::Item;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        #[inline]
        fn assert_is_stream<T: Stream>() {}
        assert_is_stream::<Flatten<Map<S, F>>>();

        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}

pin_project! {
    /// Stream for [`CompletionStreamExt::flatten`].
    #[derive(Debug, Clone)]
    pub struct Flatten<S: CompletionStream> {
        #[pin]
        stream: S,
        #[pin]
        current: Option<S::Item>,
    }
}

impl<S: CompletionStream> Flatten<S> {
    pub(crate) fn new(stream: S) -> Self {
        Self {
            stream,
            current: None,
        }
    }
}

impl<S, U> CompletionStream for Flatten<S>
where
    S: CompletionStream<Item = U>,
    U: CompletionStream,
{
    type Item = U::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        Poll::Ready(loop {
            if let Some(current) = this.current.as_mut().as_pin_mut() {
                if let Some(item) = ready!(current.poll_next(cx)) {
                    break Some(item);
                }
                this.current.set(None);
            }

            this.current
                .set(Some(match ready!(this.stream.as_mut().poll_next(cx)) {
                    Some(stream) => stream,
                    None => break None,
                }));
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let (_, stream_upper) = self.stream.size_hint();
        let (current_lower, current_upper) =
            self.current.as_ref().map_or((0, Some(0)), U::size_hint);
        (
            current_lower,
            if stream_upper == Some(0) {
                current_upper
            } else {
                None
            },
        )
    }
}

impl<S, U> Stream for Flatten<S>
where
    S: CompletionStream<Item = U> + Stream<Item = U>,
    U: CompletionStream + Stream<Item = <U as CompletionStream>::Item>,
{
    type Item = <Self as CompletionStream>::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}
