use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::CompletionStream;
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

pin_project! {
    /// Stream for [`CompletionStreamExt::enumerate`](crate::CompletionStreamExt::enumerate).
    #[derive(Debug, Clone)]
    pub struct Enumerate<S> {
        #[pin]
        stream: S,
        count: usize,
    }
}

impl<S> Enumerate<S> {
    pub(crate) fn new(stream: S) -> Self {
        Self { stream, count: 0 }
    }
}

impl<S> CompletionStream for Enumerate<S>
where
    S: CompletionStream,
{
    type Item = (usize, S::Item);

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();

        let item = match ready!(this.stream.poll_next(cx)) {
            Some(item) => item,
            None => return Poll::Ready(None),
        };
        let i = *this.count;
        *this.count += 1;
        Poll::Ready(Some((i, item)))
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.project().stream.poll_cancel(cx)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.stream.size_hint()
    }
}

impl<S> Stream for Enumerate<S>
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
