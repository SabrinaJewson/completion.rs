use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::CompletionFuture;
use futures_core::ready;
use pin_project_lite::pin_project;

pin_project! {
    /// Future for [`CompletionFutureExt::now_or_never`](super::CompletionFutureExt::now_or_never).
    #[derive(Debug)]
    pub struct NowOrNever<F> {
        #[pin]
        fut: F,
        cancelling: bool,
    }
}

impl<F: CompletionFuture> NowOrNever<F> {
    pub(super) fn new(fut: F) -> Self {
        Self {
            fut,
            cancelling: false,
        }
    }
}

impl<F> CompletionFuture for NowOrNever<F>
where
    F: CompletionFuture,
{
    type Output = Option<F::Output>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        if !*this.cancelling {
            if let Poll::Ready(output) = this.fut.as_mut().poll(&mut crate::noop_cx()) {
                return Poll::Ready(Some(output));
            }
            *this.cancelling = true;
        }
        ready!(this.fut.poll_cancel(cx));
        Poll::Ready(None)
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.project().fut.poll_cancel(cx)
    }
}
impl<F> Future for NowOrNever<F>
where
    F: CompletionFuture + Future<Output = <F as CompletionFuture>::Output>,
{
    type Output = Option<<F as CompletionFuture>::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}
