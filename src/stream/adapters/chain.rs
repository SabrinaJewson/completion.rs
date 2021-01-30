use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::CompletionStream;
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

pin_project! {
    /// Stream for [`CompletionStreamExt::chain`](crate::CompletionStreamExt::chain).
    #[derive(Debug, Clone)]
    pub struct Chain<A, B> {
        #[pin]
        a: Option<A>,
        #[pin]
        b: B,
    }
}

impl<A, B> Chain<A, B> {
    pub(crate) fn new(a: A, b: B) -> Self {
        Self { a: Some(a), b }
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
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut this = self.project();

        if let Some(a) = this.a.as_mut().as_pin_mut() {
            a.poll_cancel(cx)
        } else {
            this.b.poll_cancel(cx)
        }
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
