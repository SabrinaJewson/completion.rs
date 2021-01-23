use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::{CompletionFuture, CompletionStream};
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

pin_project! {
    /// Future for [`CompletionStreamExt::fold`](crate::CompletionStreamExt::fold).
    #[derive(Debug)]
    pub struct Fold<S, F, T> {
        #[pin]
        stream: S,
        f: F,
        accumulator: Option<T>,
    }
}

impl<S, F, T> Fold<S, F, T> {
    pub(crate) fn new(stream: S, init: T, f: F) -> Self {
        Self {
            stream,
            f,
            accumulator: Some(init),
        }
    }
}

impl<S, F, T> CompletionFuture for Fold<S, F, T>
where
    S: CompletionStream,
    F: FnMut(T, S::Item) -> T,
{
    type Output = T;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        loop {
            match ready!(this.stream.as_mut().poll_next(cx)) {
                Some(item) => {
                    let accumulator = this.accumulator.take().expect("polled after completion");
                    *this.accumulator = Some((this.f)(accumulator, item));
                }
                None => {
                    break Poll::Ready(this.accumulator.take().expect("polled after completion"))
                }
            }
        }
    }
}

impl<S, F, T> Future for Fold<S, F, T>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    F: FnMut(T, <S as CompletionStream>::Item) -> T,
{
    type Output = <Self as CompletionFuture>::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}
