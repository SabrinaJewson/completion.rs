use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::{CompletionFuture, CompletionStream};
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

use super::CompletionStreamExt;

/// Future for [`CompletionStreamExt::next`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you use them"]
pub struct Next<'a, S: ?Sized> {
    pub(super) stream: &'a mut S,
}

impl<'a, S: Unpin + ?Sized> Unpin for Next<'a, S> {}

impl<'a, S: Unpin + ?Sized> CompletionFuture for Next<'a, S>
where
    S: CompletionStream,
{
    type Output = Option<S::Item>;

    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.stream.poll_next(cx)
    }
}

impl<'a, S: Unpin + ?Sized> Future for Next<'a, S>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
{
    type Output = Option<<S as CompletionStream>::Item>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

pin_project! {
    /// Future for [`CompletionStreamExt::count`].
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you use them"]
    pub struct Count<S: ?Sized> {
        pub(super) count: usize,
        #[pin]
        pub(super) stream: S,
    }
}

impl<S: CompletionStream + ?Sized> CompletionFuture for Count<S> {
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
impl<S: Stream + CompletionStream + ?Sized> Future for Count<S> {
    type Output = usize;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

pin_project! {
    /// Future for [`CompletionStreamExt::last`].
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you use them"]
    pub struct Last<S: CompletionStream> {
        #[pin]
        pub(super) stream: S,
        pub(super) last: Option<S::Item>,
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
}
impl<S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>> Future for Last<S> {
    type Output = Option<<S as CompletionStream>::Item>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

/// Future for [`CompletionStreamExt::nth`].
#[derive(Debug)]
#[must_use = "futures do nothing unless you use them"]
pub struct Nth<'a, S: ?Sized> {
    pub(super) stream: &'a mut S,
    pub(super) n: usize,
}

impl<'a, S: Unpin + ?Sized> Unpin for Nth<'a, S> {}

impl<'a, S: Unpin + ?Sized> CompletionFuture for Nth<'a, S>
where
    S: CompletionStream,
{
    type Output = Option<S::Item>;

    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match ready!(self.stream.poll_next(cx)) {
                Some(v) => {
                    self.n = match self.n.checked_sub(1) {
                        Some(n) => n,
                        None => return Poll::Ready(Some(v)),
                    }
                }
                None => return Poll::Ready(None),
            }
        }
    }
}
impl<'a, S: Unpin + ?Sized> Future for Nth<'a, S>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
{
    type Output = Option<<S as CompletionStream>::Item>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

pin_project! {
    /// Future for [`CompletionStreamExt::for_each`].
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you use them"]
    pub struct ForEach<S, F> {
        #[pin]
        pub(super) stream: S,
        pub(super) f: F,
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
