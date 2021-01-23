use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::{CompletionFuture, CompletionStream};
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

#[cfg(doc)]
use super::super::CompletionStreamExt;

pin_project! {
    /// Future for [`CompletionStreamExt::count`].
    #[derive(Debug)]
    pub struct Count<S> {
        count: usize,
        #[pin]
        stream: S,
    }
}

impl<S> Count<S> {
    pub(crate) fn new(stream: S) -> Self {
        Self { count: 0, stream }
    }
}

impl<S> CompletionFuture for Count<S>
where
    S: CompletionStream,
{
    type Output = usize;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        loop {
            match ready!(this.stream.as_mut().poll_next(cx)) {
                Some(_) => *this.count += 1,
                None => return Poll::Ready(*this.count),
            }
        }
    }
}
impl<S> Future for Count<S>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
{
    type Output = usize;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}
