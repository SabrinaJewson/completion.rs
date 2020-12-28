use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::CompletionStream;
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

#[cfg(doc)]
use super::CompletionStreamExt;

pin_project! {
    /// Stream for [`CompletionStreamExt::map`].
    #[derive(Debug, Clone)]
    #[must_use = "streams do nothing unless you use them"]
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
    /// Stream for [`CompletionStreamExt::chain`].
    #[derive(Debug, Clone)]
    #[must_use = "streams do nothing unless you use them"]
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
        if let Some(a) = &self.a {
            let (a_min, a_max) = a.size_hint();
            let (b_min, b_max) = self.b.size_hint();

            (
                usize::saturating_add(a_min, b_min),
                Option::zip(a_max, b_max).and_then(|(a, b)| usize::checked_add(a, b)),
            )
        } else {
            self.b.size_hint()
        }
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
    /// Stream for [`CompletionStreamExt::fuse`].
    #[derive(Debug, Clone)]
    #[must_use = "streams do nothing unless you use them"]
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
        if let Some(stream) = &self.stream {
            stream.size_hint()
        } else {
            (0, Some(0))
        }
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
