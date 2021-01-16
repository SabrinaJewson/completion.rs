use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::{CompletionFuture, CompletionStream};
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

#[cfg(doc)]
use super::CompletionStreamExt;

pin_project! {
    /// Stream for [`CompletionStreamExt::chain`].
    #[derive(Debug, Clone)]
    pub struct Chain<A, B> {
        #[pin]
        pub(super) a: Option<A>,
        #[pin]
        pub(super) b: B,
    }
}

impl<A, B, I> CompletionStream for Chain<A, B>
where
    A: CompletionStream<Item = I>,
    B: CompletionStream<Item = I>,
{
    type Item = I;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        if let Some(a) = this.a.as_mut().as_pin_mut() {
            match ready!(a.poll_next(cx)) {
                Some(item) => return Poll::Ready(Some(item)),
                None => this.a.set(None),
            }
        }
        this.b.poll_next(cx)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.a.as_ref().map_or_else(
            || self.b.size_hint(),
            |a| {
                let (a_min, a_max) = a.size_hint();
                let (b_min, b_max) = self.b.size_hint();

                (
                    usize::saturating_add(a_min, b_min),
                    Option::zip(a_max, b_max).and_then(|(a, b)| usize::checked_add(a, b)),
                )
            },
        )
    }
}
impl<A, B, I> Stream for Chain<A, B>
where
    A: CompletionStream<Item = I> + Stream<Item = I>,
    B: CompletionStream<Item = I> + Stream<Item = I>,
{
    type Item = I;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}

pin_project! {
    /// Stream for [`CompletionStreamExt::map`].
    #[derive(Debug, Clone)]
    pub struct Map<S, F> {
        #[pin]
        pub(super) stream: S,
        pub(super) f: F,
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
        pub(super) stream: S,
        #[pin]
        pub(super) fut: Option<Fut>,
        pub(super) f: F,
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

pin_project! {
    /// Stream for [`CompletionStreamExt::fuse`].
    #[derive(Debug, Clone)]
    pub struct Fuse<S> {
        #[pin]
        pub(super) stream: Option<S>,
    }
}

impl<S> Fuse<S> {
    /// Get whether the underlying stream has finished and this adapter will only return [`None`].
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.stream.is_none()
    }
}

impl<S: CompletionStream> CompletionStream for Fuse<S> {
    type Item = S::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        Poll::Ready(if let Some(stream) = this.stream.as_mut().as_pin_mut() {
            let value = ready!(stream.poll_next(cx));
            if value.is_none() {
                this.stream.set(None);
            }
            value
        } else {
            None
        })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.stream
            .as_ref()
            .map_or((0, Some(0)), CompletionStream::size_hint)
    }
}
impl<S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>> Stream for Fuse<S> {
    type Item = <S as CompletionStream>::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}
