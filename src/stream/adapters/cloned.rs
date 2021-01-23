//! `Cloned` and `Copied`.

use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::CompletionStream;
use futures_core::Stream;
use pin_project_lite::pin_project;

pin_project! {
    /// Stream for [`CompletionStreamExt::cloned`](crate::CompletionStreamExt::cloned).
    #[derive(Debug, Clone)]
    pub struct Cloned<S> {
        #[pin]
        stream: S,
    }
}

impl<S> Cloned<S> {
    pub(crate) fn new(stream: S) -> Self {
        Self { stream }
    }
}

impl<'a, S, T: Clone + 'a> CompletionStream for Cloned<S>
where
    S: CompletionStream<Item = &'a T>,
{
    type Item = T;
    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project()
            .stream
            .poll_next(cx)
            .map(<Option<S::Item>>::cloned)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.stream.size_hint()
    }
}

impl<'a, S, T: Clone + 'a> Stream for Cloned<S>
where
    S: CompletionStream<Item = &'a T> + Stream<Item = &'a T>,
{
    type Item = T;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}

pin_project! {
    /// Stream for [`CompletionStreamExt::copied`](crate::CompletionStreamExt::copied).
    #[derive(Debug, Clone)]
    pub struct Copied<S> {
        #[pin]
        stream: S,
    }
}

impl<S> Copied<S> {
    pub(crate) fn new(stream: S) -> Self {
        Self { stream }
    }
}

impl<'a, S, T: Copy + 'a> CompletionStream for Copied<S>
where
    S: CompletionStream<Item = &'a T>,
{
    type Item = T;
    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project()
            .stream
            .poll_next(cx)
            .map(<Option<S::Item>>::copied)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.stream.size_hint()
    }
}

impl<'a, S, T: Copy + 'a> Stream for Copied<S>
where
    S: CompletionStream<Item = &'a T> + Stream<Item = &'a T>,
{
    type Item = T;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}
