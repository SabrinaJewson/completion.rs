use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::{CompletionFuture, CompletionStream};
use futures_core::Stream;
use pin_project_lite::pin_project;

use crate::stream::FromCompletionStream;

#[cfg(doc)]
use super::super::CompletionStreamExt;

pin_project! {
    /// Future for [`CompletionStreamExt::collect`].
    #[derive(Debug)]
    pub struct Collect<S: CompletionStream, C: FromCompletionStream<S::Item>> {
        #[pin]
        stream: S,
        collection: Option<C::Intermediate>,
    }
}

impl<S: CompletionStream, C: FromCompletionStream<S::Item>> Collect<S, C> {
    pub(crate) fn new(stream: S) -> Self {
        let (lower, upper) = stream.size_hint();
        Self {
            stream,
            collection: Some(C::start(lower, upper)),
        }
    }
}

impl<S, C> CompletionFuture for Collect<S, C>
where
    S: CompletionStream,
    C: FromCompletionStream<S::Item>,
{
    type Output = C;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        let mut collection = this
            .collection
            .take()
            .expect("`Collect` polled after completion");

        loop {
            match this.stream.as_mut().poll_next(cx) {
                Poll::Ready(Some(item)) => match C::push(collection, item) {
                    Ok(new_collection) => collection = new_collection,
                    Err(finished) => return Poll::Ready(finished),
                },
                Poll::Ready(None) => return Poll::Ready(C::finalize(collection)),
                Poll::Pending => break,
            }
        }
        *this.collection = Some(collection);

        Poll::Pending
    }
}

impl<S, C> Future for Collect<S, C>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    C: FromCompletionStream<<S as CompletionStream>::Item>,
{
    type Output = C;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}
