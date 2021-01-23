use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::CompletionStream;
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

#[cfg(doc)]
use super::super::CompletionStreamExt;

pin_project! {
    /// Stream for [`CompletionStreamExt::inspect`].
    #[derive(Debug, Clone)]
    pub struct Inspect<S, F> {
        #[pin]
        stream: S,
        f: F,
    }
}

impl<S, F> Inspect<S, F> {
    pub(crate) fn new(stream: S, f: F) -> Self {
        Self { stream, f }
    }
}

impl<S, F> CompletionStream for Inspect<S, F>
where
    S: CompletionStream,
    F: FnMut(&S::Item),
{
    type Item = S::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();

        let val = ready!(this.stream.poll_next(cx));
        if let Some(val) = &val {
            (this.f)(val);
        }
        Poll::Ready(val)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.stream.size_hint()
    }
}

impl<S, F> Stream for Inspect<S, F>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    F: FnMut(&<S as CompletionStream>::Item),
{
    type Item = <S as CompletionStream>::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}
