//! Core traits and types for completion-based asynchronous programming.
//!
//! See [completion](https://crates.io/crates/completion) for utilities based on this.
#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

use core::future::Future as RegularFuture;
#[cfg(doc)]
use core::future::Future;
use core::ops::DerefMut;
use core::pin::Pin;
use core::task::{Context, Poll};

#[cfg(doc)]
use futures_core::Stream;

/// A [`Future`] that must be polled to completion.
///
/// All types that implement [`Future`] should also implement this.
#[must_use = "futures do nothing unless you use them"]
pub trait CompletionFuture {
    /// The type of value produced on completion.
    type Output;

    /// Attempt to resolve the future to a final value, registering the current task for wakeup if
    /// the value is not yet available.
    ///
    /// # Safety
    ///
    /// Once this function is called and the type does not also implement [`Future`], the user
    /// **must not** drop or forget the future until it it has returned [`Poll::Ready`] or panicked.
    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output>;
}

impl<F: CompletionFuture + Unpin + ?Sized> CompletionFuture for &'_ mut F {
    type Output = F::Output;

    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut **self).poll(cx)
    }
}

#[cfg(feature = "alloc")]
impl<F: CompletionFuture + Unpin + ?Sized> CompletionFuture for alloc::boxed::Box<F> {
    type Output = F::Output;

    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut **self).poll(cx)
    }
}

impl<P> CompletionFuture for Pin<P>
where
    P: Unpin + DerefMut,
    P::Target: CompletionFuture,
{
    type Output = <P::Target as CompletionFuture>::Output;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().as_mut().poll(cx)
    }
}

#[cfg(feature = "std")]
impl<F: CompletionFuture> CompletionFuture for std::panic::AssertUnwindSafe<F> {
    type Output = F::Output;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::map_unchecked_mut(self, |this| &mut this.0).poll(cx)
    }
}

macro_rules! derive_completion_future {
    ($([$($generics:tt)*] $t:ty,)*) => {
        $(
            impl<$($generics)*> CompletionFuture for $t
            where
                Self: RegularFuture,
            {
                type Output = <$t as RegularFuture>::Output;

                unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                    RegularFuture::poll(self, cx)
                }
            }
        )*
    };
}

derive_completion_future! {
    [T] core::future::Pending<T>,
    [T] core::future::Ready<T>,
}

/// A [`Stream`] where each value must be polled to completion.
///
/// All types that implement [`Stream`] should also implement this.
#[must_use = "streams do nothing unless you use them"]
pub trait CompletionStream {
    /// Values yielded by the stream.
    type Item;

    /// Attempt to pull out the next value of this stream, registering the current task for wakeup
    /// if the value is not yet available, and returning [`None`] if the stream is exhausted.
    ///
    /// # Safety
    ///
    /// Once this function is called and the type does not also implement [`Stream`], the user
    /// **must not** drop or forget the stream until it has returned [`Poll::Ready`] or panicked.
    /// Note that users may drop the stream in between finishing polling one item and starting to
    /// poll the next.
    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>>;

    /// Returns the bounds on the remaining length of the stream.
    ///
    /// See [`Stream::size_hint`] for more details.
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, None)
    }
}

impl<S: CompletionStream + Unpin + ?Sized> CompletionStream for &'_ mut S {
    type Item = S::Item;

    unsafe fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        Pin::new(&mut **self).poll_next(cx)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (**self).size_hint()
    }
}

#[cfg(feature = "alloc")]
impl<S: CompletionStream + Unpin + ?Sized> CompletionStream for alloc::boxed::Box<S> {
    type Item = S::Item;

    unsafe fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        Pin::new(&mut **self).poll_next(cx)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (**self).size_hint()
    }
}

impl<P> CompletionStream for Pin<P>
where
    P: Unpin + DerefMut,
    P::Target: CompletionStream,
{
    type Item = <P::Target as CompletionStream>::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().as_mut().poll_next(cx)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (**self).size_hint()
    }
}

#[cfg(feature = "std")]
impl<S: CompletionStream> CompletionStream for std::panic::AssertUnwindSafe<S> {
    type Item = S::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::map_unchecked_mut(self, |this| &mut this.0).poll_next(cx)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}
