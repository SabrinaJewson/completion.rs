use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::{CompletionFuture, CompletionStream};
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

use super::{CompletionStreamExt, FromCompletionStream};

/// Future for [`CompletionStreamExt::next`].
#[derive(Debug)]
pub struct Next<'a, S: ?Sized> {
    pub(super) stream: &'a mut S,
}

impl<S: Unpin + ?Sized> Unpin for Next<'_, S> {}

impl<S: Unpin + ?Sized> CompletionFuture for Next<'_, S>
where
    S: CompletionStream,
{
    type Output = Option<S::Item>;

    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.stream.poll_next(cx)
    }
}

impl<S: Unpin + ?Sized> Future for Next<'_, S>
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
pub struct Nth<'a, S: ?Sized> {
    pub(super) stream: &'a mut S,
    pub(super) n: usize,
}

impl<S: Unpin + ?Sized> Unpin for Nth<'_, S> {}

impl<S: Unpin + ?Sized> CompletionFuture for Nth<'_, S>
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
impl<S: Unpin + ?Sized> Future for Nth<'_, S>
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

pin_project! {
    /// Future for [`CompletionStreamExt::collect`].
    #[derive(Debug)]
    pub struct Collect<S: CompletionStream, C: FromCompletionStream<S::Item>> {
        #[pin]
        pub(super) stream: S,
        pub(super) collection: Option<C::Intermediate>,
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
    S: CompletionStream,
    C: FromCompletionStream<S::Item>,
{
    type Output = C;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

pin_project! {
    /// Future for [`CompletionStreamExt::fold`].
    #[derive(Debug)]
    pub struct Fold<S, F, T> {
        #[pin]
        pub(super) stream: S,
        pub(super) f: F,
        pub(super) accumulator: Option<T>,
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

/// Future for [`CompletionStreamExt::all`].
#[derive(Debug)]
pub struct All<'a, S: ?Sized, F> {
    pub(super) stream: &'a mut S,
    pub(super) f: F,
}

impl<S: Unpin + ?Sized, F> Unpin for All<'_, S, F> {}

impl<S: Unpin + ?Sized, F> CompletionFuture for All<'_, S, F>
where
    S: CompletionStream,
    F: FnMut(S::Item) -> bool,
{
    type Output = bool;

    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        while let Some(item) = ready!(self.stream.poll_next(cx)) {
            if !(self.f)(item) {
                return Poll::Ready(false);
            }
        }
        Poll::Ready(true)
    }
}
impl<S: Unpin + ?Sized, F> Future for All<'_, S, F>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    F: FnMut(<S as CompletionStream>::Item) -> bool,
{
    type Output = bool;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

/// Future for [`CompletionStreamExt::any`].
#[derive(Debug)]
pub struct Any<'a, S: ?Sized, F> {
    pub(super) stream: &'a mut S,
    pub(super) f: F,
}

impl<S: Unpin + ?Sized, F> Unpin for Any<'_, S, F> {}

impl<S: Unpin + ?Sized, F> CompletionFuture for Any<'_, S, F>
where
    S: CompletionStream,
    F: FnMut(S::Item) -> bool,
{
    type Output = bool;

    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        while let Some(item) = ready!(self.stream.poll_next(cx)) {
            if (self.f)(item) {
                return Poll::Ready(true);
            }
        }
        Poll::Ready(false)
    }
}
impl<S: Unpin + ?Sized, F> Future for Any<'_, S, F>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    F: FnMut(<S as CompletionStream>::Item) -> bool,
{
    type Output = bool;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}
