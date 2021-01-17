use core::cmp;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::{CompletionFuture, CompletionStream};
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

#[cfg(doc)]
use super::CompletionStreamExt;

pin_project! {
    /// Stream for [`CompletionStreamExt::step_by`].
    #[derive(Debug, Clone)]
    pub struct StepBy<S> {
        #[pin]
        pub(super) stream: S,
        pub(super) step: usize,
        pub(super) i: usize,
    }
}

impl<S: CompletionStream> CompletionStream for StepBy<S> {
    type Item = S::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        loop {
            match ready!(this.stream.as_mut().poll_next(cx)) {
                Some(v) => {
                    if *this.i == 0 {
                        *this.i = *this.step - 1;
                        break Poll::Ready(Some(v));
                    } else {
                        *this.i -= 1;
                    }
                }
                None => break Poll::Ready(None),
            }
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let (low, high) = self.stream.size_hint();
        let f = |n: usize| {
            n.saturating_sub(self.i)
                .checked_sub(1)
                .map_or(0, |n| n / self.step + 1)
        };
        (f(low), high.map(f))
    }
}
impl<S> Stream for StepBy<S>
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

pin_project! {
    /// Stream for [`CompletionStreamExt::chain`].
    #[derive(Debug, Clone)]
    pub struct Chain<A, B> {
        #[pin]
        pub(super) a: Option<A>,
        #[pin]
        pub(super) b: B,
    }
}

impl<A, B, I> CompletionStream for Chain<A, B>
where
    A: CompletionStream<Item = I>,
    B: CompletionStream<Item = I>,
{
    type Item = I;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        if let Some(a) = this.a.as_mut().as_pin_mut() {
            match ready!(a.poll_next(cx)) {
                Some(item) => return Poll::Ready(Some(item)),
                None => this.a.set(None),
            }
        }
        this.b.poll_next(cx)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.a.as_ref().map_or_else(
            || self.b.size_hint(),
            |a| {
                let (a_min, a_max) = a.size_hint();
                let (b_min, b_max) = self.b.size_hint();

                (
                    usize::saturating_add(a_min, b_min),
                    Option::zip(a_max, b_max).and_then(|(a, b)| usize::checked_add(a, b)),
                )
            },
        )
    }
}
impl<A, B, I> Stream for Chain<A, B>
where
    A: CompletionStream<Item = I> + Stream<Item = I>,
    B: CompletionStream<Item = I> + Stream<Item = I>,
{
    type Item = I;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}

pin_project! {
    /// Stream for [`CompletionStreamExt::map`].
    #[derive(Debug, Clone)]
    pub struct Map<S, F> {
        #[pin]
        pub(super) stream: S,
        pub(super) f: F,
    }
}

impl<S, F, T> CompletionStream for Map<S, F>
where
    S: CompletionStream,
    F: FnMut(S::Item) -> T,
{
    type Item = T;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        this.stream
            .as_mut()
            .poll_next(cx)
            .map(|value| value.map(this.f))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.stream.size_hint()
    }
}
impl<S, F, T> Stream for Map<S, F>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    F: FnMut(<S as CompletionStream>::Item) -> T,
{
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(&self.stream)
    }
}

pin_project! {
    /// Stream for [`CompletionStreamExt::then`].
    #[derive(Debug, Clone)]
    pub struct Then<S, F, Fut> {
        #[pin]
        pub(super) stream: S,
        #[pin]
        pub(super) fut: Option<Fut>,
        pub(super) f: F,
    }
}

impl<S, F, Fut> CompletionStream for Then<S, F, Fut>
where
    S: CompletionStream,
    F: FnMut(S::Item) -> Fut,
    Fut: CompletionFuture,
{
    type Item = Fut::Output;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        let fut = match this.fut.as_mut().as_pin_mut() {
            Some(fut) => fut,
            None => match ready!(this.stream.poll_next(cx)) {
                Some(item) => {
                    this.fut.set(Some((this.f)(item)));
                    this.fut.as_mut().as_pin_mut().unwrap()
                }
                None => {
                    return Poll::Ready(None);
                }
            },
        };

        let val = ready!(fut.poll(cx));
        this.fut.set(None);
        Poll::Ready(Some(val))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let future_len = if self.fut.is_some() { 1 } else { 0 };
        let (stream_min, stream_max) = self.stream.size_hint();
        (
            stream_min.saturating_add(future_len),
            stream_max.and_then(|l| l.checked_add(future_len)),
        )
    }
}

impl<S, F, Fut> Stream for Then<S, F, Fut>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    F: FnMut(<S as CompletionStream>::Item) -> Fut,
    Fut: CompletionFuture + Future<Output = <Fut as CompletionFuture>::Output>,
{
    type Item = <Fut as CompletionFuture>::Output;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}

pin_project! {
    /// Stream for [`CompletionStreamExt::filter`].
    #[derive(Debug, Clone)]
    pub struct Filter<S, F> {
        #[pin]
        pub(super) stream: S,
        pub(super) f: F,
    }
}

impl<S: CompletionStream, F: FnMut(&S::Item) -> bool> CompletionStream for Filter<S, F> {
    type Item = S::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        loop {
            match ready!(this.stream.as_mut().poll_next(cx)) {
                Some(item) if (this.f)(&item) => break Poll::Ready(Some(item)),
                Some(_) => {}
                None => break Poll::Ready(None),
            }
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let (_, high) = self.stream.size_hint();
        (0, high)
    }
}
impl<S, F: FnMut(&<S as CompletionStream>::Item) -> bool> Stream for Filter<S, F>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
{
    type Item = <Self as CompletionStream>::Item;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}

pin_project! {
    /// Stream for [`CompletionStreamExt::filter_map`].
    #[derive(Debug, Clone)]
    pub struct FilterMap<S, F> {
        #[pin]
        pub(super) stream: S,
        pub(super) f: F,
    }
}

impl<S, F, T> CompletionStream for FilterMap<S, F>
where
    S: CompletionStream,
    F: FnMut(S::Item) -> Option<T>,
{
    type Item = T;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        loop {
            match ready!(this.stream.as_mut().poll_next(cx)) {
                Some(item) => {
                    if let Some(mapped) = (this.f)(item) {
                        break Poll::Ready(Some(mapped));
                    }
                }
                None => break Poll::Ready(None),
            }
        }
    }
}

impl<S, F, T> Stream for FilterMap<S, F>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    F: FnMut(<S as CompletionStream>::Item) -> Option<T>,
{
    type Item = <Self as CompletionStream>::Item;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}

pin_project! {
    /// Stream for [`CompletionStreamExt::skip_while`].
    #[derive(Debug, Clone)]
    pub struct SkipWhile<S, P> {
        #[pin]
        pub(super) stream: S,
        pub(super) skipping: bool,
        pub(super) predicate: P,
    }
}
impl<S: CompletionStream, P> CompletionStream for SkipWhile<S, P>
where
    P: FnMut(&S::Item) -> bool,
{
    type Item = S::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        loop {
            match ready!(this.stream.as_mut().poll_next(cx)) {
                Some(item) => {
                    if *this.skipping {
                        *this.skipping = (this.predicate)(&item);
                    }
                    if !*this.skipping {
                        break Poll::Ready(Some(item));
                    }
                }
                None => break Poll::Ready(None),
            }
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.skipping {
            (0, self.stream.size_hint().1)
        } else {
            self.stream.size_hint()
        }
    }
}
impl<S, P> Stream for SkipWhile<S, P>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    P: FnMut(&<S as CompletionStream>::Item) -> bool,
{
    type Item = <Self as CompletionStream>::Item;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}

pin_project! {
    /// Stream for [`CompletionStreamExt::take_while`].
    #[derive(Debug, Clone)]
    pub struct TakeWhile<S, P> {
        #[pin]
        pub(super) stream: S,
        pub(super) taking: bool,
        pub(super) predicate: P,
    }
}
impl<S: CompletionStream, P> CompletionStream for TakeWhile<S, P>
where
    P: FnMut(&S::Item) -> bool,
{
    type Item = S::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();

        if *this.taking {
            match ready!(this.stream.poll_next(cx)) {
                Some(item) => {
                    if (this.predicate)(&item) {
                        return Poll::Ready(Some(item));
                    }
                    *this.taking = false;
                }
                None => return Poll::Ready(None),
            }
        }
        Poll::Ready(None)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.taking {
            (0, self.stream.size_hint().1)
        } else {
            (0, Some(0))
        }
    }
}
impl<S, P> Stream for TakeWhile<S, P>
where
    S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>,
    P: FnMut(&<S as CompletionStream>::Item) -> bool,
{
    type Item = <Self as CompletionStream>::Item;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}

pin_project! {
    /// Stream for [`CompletionStreamExt::skip`].
    #[derive(Debug, Clone)]
    pub struct Skip<S> {
        #[pin]
        pub(super) stream: S,
        pub(super) to_skip: usize,
    }
}
impl<S: CompletionStream> CompletionStream for Skip<S> {
    type Item = S::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        while *this.to_skip > 0 {
            if ready!(this.stream.as_mut().poll_next(cx)).is_none() {
                return Poll::Ready(None);
            }
            *this.to_skip -= 1;
        }
        this.stream.poll_next(cx)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let (lower, upper) = self.stream.size_hint();
        (
            lower.saturating_sub(self.to_skip),
            upper.map(|upper| upper.saturating_sub(self.to_skip)),
        )
    }
}
impl<S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>> Stream for Skip<S> {
    type Item = <Self as CompletionStream>::Item;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}

pin_project! {
    /// Stream for [`CompletionStreamExt::take`].
    #[derive(Debug, Clone)]
    pub struct Take<S> {
        #[pin]
        pub(super) stream: S,
        pub(super) to_take: usize,
    }
}
impl<S: CompletionStream> CompletionStream for Take<S> {
    type Item = S::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();

        Poll::Ready(if *this.to_take == 0 {
            None
        } else {
            let item = ready!(this.stream.poll_next(cx));
            *this.to_take -= 1;
            item
        })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.to_take == 0 {
            (0, Some(0))
        } else {
            let (lower, upper) = self.stream.size_hint();
            (
                cmp::min(lower, self.to_take),
                Some(upper.map_or(self.to_take, |upper| cmp::min(upper, self.to_take))),
            )
        }
    }
}
impl<S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>> Stream for Take<S> {
    type Item = <Self as CompletionStream>::Item;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}

pin_project! {
    /// Stream for [`CompletionStreamExt::fuse`].
    #[derive(Debug, Clone)]
    pub struct Fuse<S> {
        #[pin]
        pub(super) stream: Option<S>,
    }
}

impl<S> Fuse<S> {
    /// Get whether the underlying stream has finished and this adapter will only return [`None`].
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.stream.is_none()
    }
}

impl<S: CompletionStream> CompletionStream for Fuse<S> {
    type Item = S::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();
        Poll::Ready(if let Some(stream) = this.stream.as_mut().as_pin_mut() {
            let value = ready!(stream.poll_next(cx));
            if value.is_none() {
                this.stream.set(None);
            }
            value
        } else {
            None
        })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.stream
            .as_ref()
            .map_or((0, Some(0)), CompletionStream::size_hint)
    }
}
impl<S: CompletionStream + Stream<Item = <S as CompletionStream>::Item>> Stream for Fuse<S> {
    type Item = <S as CompletionStream>::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        CompletionStream::size_hint(self)
    }
}

pin_project! {
    /// Stream for [`CompletionStreamExt::copied`].
    #[derive(Debug, Clone)]
    pub struct Copied<S> {
        #[pin]
        pub(super) stream: S,
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

pin_project! {
    /// Stream for [`CompletionStreamExt::cloned`].
    #[derive(Debug, Clone)]
    pub struct Cloned<S> {
        #[pin]
        pub(super) stream: S,
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
