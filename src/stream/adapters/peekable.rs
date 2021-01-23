use core::fmt::{self, Debug, Formatter};
use core::future::Future;
use core::mem;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::{CompletionFuture, CompletionStream};
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

#[cfg(doc)]
use super::super::CompletionStreamExt;

pin_project! {
    /// Stream for [`CompletionStreamExt::peekable`].
    #[derive(Debug, Clone)]
    pub struct Peekable<S: CompletionStream> {
        #[pin]
        stream: S,
        peeked: Poll<Option<S::Item>>,
    }
}

impl<S: CompletionStream> Peekable<S> {
    pub(crate) fn new(stream: S) -> Self {
        Self {
            stream,
            peeked: Poll::Pending,
        }
    }

    /// Peek the next value in the stream.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::{stream, pin};
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let stream = stream::once(5).must_complete().peekable();
    /// pin!(stream);
    ///
    /// assert_eq!(stream.as_mut().peek().await, Some(&5));
    /// assert_eq!(stream.as_mut().peek().await, Some(&5));
    /// assert_eq!(stream.next().await, Some(5));
    /// assert_eq!(stream.as_mut().peek().await, None);
    /// assert_eq!(stream.next().await, None);
    /// # });
    /// ```
    #[must_use]
    pub fn peek(self: Pin<&mut Self>) -> Peek<'_, S> {
        Peek { stream: Some(self) }
    }

    /// Peek the next value in the stream if the underlying type implements [`Unpin`].
    ///
    /// `stream.peek_unpin()` is equivalent to `Pin::new(&mut stream).peek()`.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let mut stream = stream::once(8).must_complete().peekable();
    ///
    /// assert_eq!(stream.peek_unpin().await, Some(&8));
    /// assert_eq!(stream.peek_unpin().await, Some(&8));
    /// assert_eq!(stream.next().await, Some(8));
    /// assert_eq!(stream.peek_unpin().await, None);
    /// assert_eq!(stream.next().await, None);
    /// # });
    /// ```
    #[must_use]
    pub fn peek_unpin(&mut self) -> Peek<'_, S>
    where
        Self: Unpin,
    {
        Pin::new(self).peek()
    }

    /// Attempt to peek the next value in the stream.
    ///
    /// This function is quite low level, use [`peek`](Self::peek) or
    /// [`peek_unpin`](Self::peek_unpin) for a higher-level equivalent.
    ///
    /// # Safety
    ///
    /// See [`CompletionStream::poll_next`].
    pub unsafe fn poll_peek(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<&S::Item>> {
        let mut this = self.project();

        if this.peeked.is_pending() {
            *this.peeked = this.stream.as_mut().poll_next(cx);
        }
        match &*this.peeked {
            Poll::Ready(ready) => Poll::Ready(ready.as_ref()),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S: CompletionStream> CompletionStream for Peekable<S> {
    type Item = S::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        if this.peeked.is_pending() {
            *this.peeked = this.stream.as_mut().poll_next(cx);
        }
        mem::replace(this.peeked, Poll::Pending)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let peek_len = match self.peeked {
            Poll::Ready(Some(_)) => 1,
            Poll::Ready(None) => return (0, Some(0)),
            Poll::Pending => 0,
        };
        let (lower, upper) = self.stream.size_hint();
        (
            lower.saturating_add(peek_len),
            upper.and_then(|n| n.checked_add(peek_len)),
        )
    }
}

impl<S> Stream for Peekable<S>
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

/// Future for [`Peekable::peek`].
pub struct Peek<'a, S: CompletionStream> {
    stream: Option<Pin<&'a mut Peekable<S>>>,
}

impl<S: CompletionStream + Debug> Debug for Peek<'_, S>
where
    S::Item: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Peek")
            .field("stream", &self.stream)
            .finish()
    }
}

impl<'a, S> CompletionFuture for Peek<'a, S>
where
    S: CompletionStream,
{
    type Output = Option<&'a S::Item>;
    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let stream = self.stream.as_mut().expect("polled after completion");
        ready!(stream.as_mut().poll_peek(cx));
        self.stream.take().unwrap().poll_peek(cx)
    }
}

impl<'a, S> Future for Peek<'a, S>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
{
    type Output = <Self as CompletionFuture>::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}
