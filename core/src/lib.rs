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
///
/// A completion future has three states: running, cancelling and complete. Futures initially start
/// out in the running state. To progress the running state, users will call [`poll`], which either
/// returns [`Poll::Pending`] to continue the running state or [`Poll::Ready`] to return a value and
/// reach the complete state.
///
/// At any time during the running state, users may call [`poll_cancel`] to initiate the cancelling
/// state. During this state, only [`poll_cancel`] should be called, and it can return
/// [`Poll::Pending`] to continue the cancelling state or [`Poll::Ready`]`(())` to reach the
/// complete state.
///
/// Once the complete state has been reached, either by regular completion or after cancellation,
/// neither [`poll`] nor [`poll_cancel`] should be called again.
///
/// A violation of these rules can cause unexpected behaviour: the future may panic, block forever
/// or return unexpected results. However, it must never cause undefined behaviour.
///
/// [`poll`]: Self::poll
/// [`poll_cancel`]: Self::poll_cancel
#[must_use = "futures do nothing unless you use them"]
pub trait CompletionFuture {
    /// The type of value produced on completion.
    type Output;

    /// Attempt to resolve the future to a final value, registering the current task for wakeup if
    /// the value is not yet available.
    ///
    /// This function should only be called when the future is in the running state.
    ///
    /// # Safety
    ///
    /// Once this function has been called and the type does not also implement [`Future`], the user
    /// **must not** drop or forget the future until it it has returned [`Poll::Ready`] or panicked.
    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output>;

    /// Attempt to cancel the future, registering the current task for wakeup if has not finished
    /// cancelling yet.
    ///
    /// This function should only be called from the running state or in the cancelling state. Once
    /// this function returns [`Poll::Ready`], the future should be considered complete and should
    /// not be polled again.
    ///
    /// Note that this may be called before [`poll`](Self::poll) has been called for the first
    /// time.
    ///
    /// # Safety
    ///
    /// Once this function has been called and the type does not also implement [`Future`], the user
    /// **must not** drop or forget the future until it has returned [`Poll::Ready`] or panicked.
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()>;
}

impl<F: CompletionFuture + Unpin + ?Sized> CompletionFuture for &'_ mut F {
    type Output = F::Output;

    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut **self).poll(cx)
    }
    unsafe fn poll_cancel(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        Pin::new(&mut **self).poll_cancel(cx)
    }
}

#[cfg(feature = "alloc")]
impl<F: CompletionFuture + Unpin + ?Sized> CompletionFuture for alloc::boxed::Box<F> {
    type Output = F::Output;

    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut **self).poll(cx)
    }
    unsafe fn poll_cancel(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        Pin::new(&mut **self).poll_cancel(cx)
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
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.get_mut().as_mut().poll_cancel(cx)
    }
}

#[cfg(feature = "std")]
impl<F: CompletionFuture> CompletionFuture for std::panic::AssertUnwindSafe<F> {
    type Output = F::Output;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::map_unchecked_mut(self, |this| &mut this.0).poll(cx)
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        Pin::map_unchecked_mut(self, |this| &mut this.0).poll_cancel(cx)
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
                unsafe fn poll_cancel(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
                    Poll::Ready(())
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
///
/// A completion stream has three states: running, cancelling and exhausted. Streams initially
/// start out in the running state. To progress the running state, users will call [`poll_next`],
/// which either returns [`Poll::Pending`] to continue the running state,
/// [`Poll::Ready`]`(`[`Some`]`)` to yield a value and continue the running state, or
/// [`Poll::Ready`]`(`[`None`]`)` to reach the exhausted state.
///
/// At any time during the running state, users may call [`poll_cancel`] to initiate the cancelling
/// state. During this state, only [`poll_cancel`] should be called, and it can return
/// [`Poll::Pending`] to continue the cancelling state or [`Poll::Ready`]`(())` to reach the
/// exhausted state.
///
/// Once the exhausted state has been reached, either by [`poll_next`] returning
/// [`Poll::Ready`]`(`[`None`]`)` or by [`poll_cancel`] returning [`Poll::Ready`], neither
/// [`poll_next`] nor [`poll_cancel`] should be called again.
///
/// [`poll_next`]: Self::poll_next
/// [`poll_cancel`]: Self::poll_cancel
#[must_use = "streams do nothing unless you use them"]
pub trait CompletionStream {
    /// Values yielded by the stream.
    type Item;

    /// Attempt to pull out the next value of this stream, registering the current task for wakeup
    /// if the value is not yet available, and returning [`None`] if the stream is exhausted.
    ///
    /// This function should only be called when the stream is in the running state.
    ///
    /// # Safety
    ///
    /// Once this function has been called and the type does not also implement [`Stream`], the user
    /// **must not** drop or forget the stream until it has returned [`Poll::Ready`] or panicked.
    /// Note that users may drop the stream in between finishing polling one item and starting to
    /// poll the next.
    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>>;

    /// Attempt to cancel the stream, registering the current task for wakeup if it has not finished
    /// cancelling yet.
    ///
    /// This function should only be called when the stream is in the running state or in the
    /// cancelling state. Once this function returns [`Poll::Ready`], the stream should be
    /// considered exhausted and should not be polled again.
    ///
    /// # Safety
    ///
    /// Once this function has been called and the type does not also implement [`Stream`], the user
    /// **must not** drop or forget the stream until it has returned [`Poll::Ready`] or panicked.
    /// Note that users may drop the stream in between cancelling one item and starting to poll the
    /// next.
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()>;

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
    unsafe fn poll_cancel(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        Pin::new(&mut **self).poll_cancel(cx)
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
    unsafe fn poll_cancel(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        Pin::new(&mut **self).poll_cancel(cx)
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
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.get_mut().as_mut().poll_cancel(cx)
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
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        Pin::map_unchecked_mut(self, |this| &mut this.0).poll_cancel(cx)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}
