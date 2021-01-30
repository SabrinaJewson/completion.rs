use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::CompletionStream;
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

pin_project! {
    /// Stream for [`CompletionStreamExt::fuse`](crate::CompletionStreamExt::fuse).
    #[derive(Debug, Clone)]
    pub struct Fuse<S> {
        #[pin]
        stream: Option<S>,
    }
}

impl<S> Fuse<S> {
    pub(crate) fn new(stream: S) -> Self {
        Self {
            stream: Some(stream),
        }
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
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut this = self.project();
        if let Some(stream) = this.stream.as_mut().as_pin_mut() {
            ready!(stream.poll_cancel(cx));
            this.stream.set(None);
        }
        Poll::Ready(())
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
