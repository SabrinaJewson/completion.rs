use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::CompletionStream;
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

pin_project! {
    /// Stream for [`CompletionStreamExt::step_by`](crate::CompletionStreamExt::step_by).
    #[derive(Debug, Clone)]
    pub struct StepBy<S> {
        #[pin]
        stream: S,
        step: usize,
        i: usize,
    }
}

impl<S> StepBy<S> {
    pub(crate) fn new(stream: S, step: usize) -> Self {
        assert!(step != 0, "cannot step by zero");
        Self { stream, step, i: 0 }
    }
}

impl<S: CompletionStream> CompletionStream for StepBy<S> {
    type Item = S::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        loop {
            match ready!(this.stream.as_mut().poll_next(cx)) {
                Some(v) => {
                    if *this.i == 0 {
                        *this.i = *this.step - 1;
                        break Poll::Ready(Some(v));
                    }
                    *this.i -= 1;
                }
                None => break Poll::Ready(None),
            }
        }
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.project().stream.poll_cancel(cx)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let (low, high) = self.stream.size_hint();
        let f = |n: usize| {
            n.saturating_sub(self.i)
                .checked_sub(1)
                .map_or(0, |n| n / self.step + 1)
        };
        (f(low), high.map(f))
    }
}

impl<S> Stream for StepBy<S>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
{
    type Item = <S as CompletionStream>::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}
