use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::{CompletionFuture, CompletionStream};
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

pin_project! {
    /// Future for [`CompletionStreamExt::last`](crate::CompletionStreamExt::last).
    #[derive(Debug)]
    pub struct Last<S: CompletionStream> {
        #[pin]
        stream: S,
        last: Option<S::Item>,
    }
}

impl<S: CompletionStream> Last<S> {
    pub(crate) fn new(stream: S) -> Self {
        Self { stream, last: None }
    }
}

impl<S: CompletionStream> CompletionFuture for Last<S> {
    type Output = Option<S::Item>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        loop {
            match ready!(this.stream.as_mut().poll_next(cx)) {
                Some(last) => *this.last = Some(last),
                None => return Poll::Ready(this.last.take()),
            }
        }
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.project().stream.poll_cancel(cx)
    }
}

impl<S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>> Future for Last<S> {
    type Output = Option<<S as CompletionStream>::Item>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}
