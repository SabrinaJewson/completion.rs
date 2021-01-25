use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::CompletionStream;
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

pin_project! {
    /// Stream for [`CompletionStreamExt::cycle`](crate::CompletionStreamExt::cycle).
    #[derive(Debug, Clone)]
    pub struct Cycle<S> {
        template: S,
        #[pin]
        stream: S,
    }
}

impl<S: Clone> Cycle<S> {
    pub(crate) fn new(stream: S) -> Self {
        Self {
            template: stream.clone(),
            stream,
        }
    }
}

impl<S: Clone> CompletionStream for Cycle<S>
where
    S: CompletionStream,
{
    type Item = S::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        if let Some(val) = ready!(this.stream.as_mut().poll_next(cx)) {
            Poll::Ready(Some(val))
        } else {
            this.stream.as_mut().set(this.template.clone());
            this.stream.poll_next(cx)
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self.template.size_hint() {
            (0, Some(0)) => (0, Some(0)),
            (0, _) => (0, None),
            _ => (usize::MAX, None),
        }
    }
}

impl<S: Clone> Stream for Cycle<S>
where
    S: CompletionStream + Stream<Item = <Self as CompletionStream>::Item>,
{
    type Item = <Self as CompletionStream>::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}
