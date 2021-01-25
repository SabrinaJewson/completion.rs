//! `Max`, `MaxBy`, `MaxByKey`, `Min`, `MinBy`, `MinByKey`.

use core::cmp::Ordering;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::{CompletionFuture, CompletionStream};
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

use crate::CompletionStreamExt;

type OrdFn<S> = fn(&<S as CompletionStream>::Item, &<S as CompletionStream>::Item) -> Ordering;

pin_project! {
    /// Future for [`CompletionStreamExt::max`].
    pub struct Max<S: CompletionStream> {
        #[pin]
        inner: MaxBy<S, OrdFn<S>>,
    }
}

impl<S: CompletionStream> Max<S>
where
    S::Item: Ord,
{
    pub(crate) fn new(stream: S) -> Self {
        Self {
            inner: stream.max_by(<S::Item as Ord>::cmp),
        }
    }
}

impl<S> CompletionFuture for Max<S>
where
    S: CompletionStream,
{
    type Output = Option<S::Item>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().inner.poll(cx)
    }
}

impl<S> Future for Max<S>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
{
    type Output = <Self as CompletionFuture>::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

pin_project! {
    /// Future for [`CompletionStreamExt::max_by`].
    pub struct MaxBy<S: CompletionStream, F> {
        #[pin]
        stream: S,
        max: Option<S::Item>,
        compare: F,
    }
}

impl<S: CompletionStream, F> MaxBy<S, F> {
    pub(crate) fn new(stream: S, compare: F) -> Self {
        Self {
            stream,
            max: None,
            compare,
        }
    }
}

impl<S, F> CompletionFuture for MaxBy<S, F>
where
    S: CompletionStream,
    F: FnMut(&S::Item, &S::Item) -> Ordering,
{
    type Output = Option<S::Item>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        loop {
            match ready!(this.stream.as_mut().poll_next(cx)) {
                Some(item) => {
                    let compare = &mut *this.compare;
                    if this
                        .max
                        .as_ref()
                        .map_or(true, |val| compare(&item, val) != Ordering::Less)
                    {
                        *this.max = Some(item);
                    }
                }
                None => break Poll::Ready(this.max.take()),
            }
        }
    }
}

impl<S, F> Future for MaxBy<S, F>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    F: FnMut(&<S as CompletionStream>::Item, &<S as CompletionStream>::Item) -> Ordering,
{
    type Output = <Self as CompletionFuture>::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

pin_project! {
    /// Future for [`CompletionStreamExt::max_by_key`].
    pub struct MaxByKey<S: CompletionStream, B, F> {
        #[pin]
        stream: S,
        max: Option<(S::Item, B)>,
        f: F,
    }
}

impl<S: CompletionStream, B, F> MaxByKey<S, B, F> {
    pub(crate) fn new(stream: S, f: F) -> Self {
        Self {
            stream,
            max: None,
            f,
        }
    }
}

impl<S, B, F> CompletionFuture for MaxByKey<S, B, F>
where
    S: CompletionStream,
    F: FnMut(&S::Item) -> B,
    B: Ord,
{
    type Output = Option<S::Item>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        loop {
            match ready!(this.stream.as_mut().poll_next(cx)) {
                Some(item) => {
                    let item_key = (this.f)(&item);
                    if this.max.as_ref().map_or(true, |(_, key)| item_key >= *key) {
                        *this.max = Some((item, item_key));
                    }
                }
                None => break Poll::Ready(this.max.take().map(|(item, _)| item)),
            }
        }
    }
}

impl<S, B, F> Future for MaxByKey<S, B, F>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    F: FnMut(&<S as CompletionStream>::Item) -> B,
    B: Ord,
{
    type Output = <Self as CompletionFuture>::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

pin_project! {
    /// Future for [`CompletionStreamExt::min`].
    pub struct Min<S: CompletionStream> {
        #[pin]
        inner: MinBy<S, OrdFn<S>>,
    }
}

impl<S: CompletionStream> Min<S>
where
    S::Item: Ord,
{
    pub(crate) fn new(stream: S) -> Self {
        Self {
            inner: stream.min_by(<S::Item as Ord>::cmp),
        }
    }
}

impl<S> CompletionFuture for Min<S>
where
    S: CompletionStream,
{
    type Output = Option<S::Item>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().inner.poll(cx)
    }
}

impl<S> Future for Min<S>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
{
    type Output = <Self as CompletionFuture>::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

pin_project! {
    /// Future for [`CompletionStreamExt::min_by`].
    pub struct MinBy<S: CompletionStream, F> {
        #[pin]
        stream: S,
        min: Option<S::Item>,
        compare: F,
    }
}

impl<S: CompletionStream, F> MinBy<S, F> {
    pub(crate) fn new(stream: S, compare: F) -> Self {
        Self {
            stream,
            min: None,
            compare,
        }
    }
}

impl<S, F> CompletionFuture for MinBy<S, F>
where
    S: CompletionStream,
    F: FnMut(&S::Item, &S::Item) -> Ordering,
{
    type Output = Option<S::Item>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        loop {
            match ready!(this.stream.as_mut().poll_next(cx)) {
                Some(item) => {
                    let compare = &mut *this.compare;
                    if this
                        .min
                        .as_ref()
                        .map_or(true, |val| compare(&item, val) == Ordering::Less)
                    {
                        *this.min = Some(item);
                    }
                }
                None => break Poll::Ready(this.min.take()),
            }
        }
    }
}

impl<S, F> Future for MinBy<S, F>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    F: FnMut(&<S as CompletionStream>::Item, &<S as CompletionStream>::Item) -> Ordering,
{
    type Output = <Self as CompletionFuture>::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

pin_project! {
    /// Future for [`CompletionStreamExt::min_by_key`].
    pub struct MinByKey<S: CompletionStream, B, F> {
        #[pin]
        stream: S,
        min: Option<(S::Item, B)>,
        f: F,
    }
}

impl<S: CompletionStream, B, F> MinByKey<S, B, F> {
    pub(crate) fn new(stream: S, f: F) -> Self {
        Self {
            stream,
            min: None,
            f,
        }
    }
}

impl<S, B, F> CompletionFuture for MinByKey<S, B, F>
where
    S: CompletionStream,
    F: FnMut(&S::Item) -> B,
    B: Ord,
{
    type Output = Option<S::Item>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        loop {
            match ready!(this.stream.as_mut().poll_next(cx)) {
                Some(item) => {
                    let item_key = (this.f)(&item);
                    if this.min.as_ref().map_or(true, |(_, key)| item_key < *key) {
                        *this.min = Some((item, item_key));
                    }
                }
                None => break Poll::Ready(this.min.take().map(|(item, _)| item)),
            }
        }
    }
}

impl<S, B, F> Future for MinByKey<S, B, F>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    F: FnMut(&<S as CompletionStream>::Item) -> B,
    B: Ord,
{
    type Output = <Self as CompletionFuture>::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}
