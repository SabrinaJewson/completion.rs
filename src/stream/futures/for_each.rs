use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::{CompletionFuture, CompletionStream};
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

#[cfg(doc)]
use super::super::CompletionStreamExt;

pin_project! {
    /// Future for [`CompletionStreamExt::for_each`].
    #[derive(Debug)]
    pub struct ForEach<S, F> {
        #[pin]
        stream: S,
        f: F,
    }
}

impl<S, F> ForEach<S, F> {
    pub(crate) fn new(stream: S, f: F) -> Self {
        Self { stream, f }
    }
}

impl<S, F> CompletionFuture for ForEach<S, F>
where
    S: CompletionStream,
    F: FnMut(S::Item),
{
    type Output = ();

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        while let Some(v) = ready!(this.stream.as_mut().poll_next(cx)) {
            (this.f)(v);
        }
        Poll::Ready(())
    }
}

impl<S, F> Future for ForEach<S, F>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    F: FnMut(<S as CompletionStream>::Item),
{
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}
