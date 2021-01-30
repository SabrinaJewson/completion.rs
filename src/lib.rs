//! Utilities for writing completion-based asynchronous code.
//!
//! A completion future is a future that must be run to completion, unlike regular futures which
//! can be dropped and stopped at any time without the future's knowledge. This allows for more
//! flexibility for the implementer of the future and allows APIs like `io_uring` and IOCP to be
//! wrapped in a zero-cost way.
//!
//! This is based off [this RFC by Matthias247](https://github.com/Matthias247/rfcs/pull/1).
//!
//! # Features
//!
//! - `std`: Enables features that require the standard library, on by default.
//! - `alloc`: Enables features that require allocation, on by default.
//! - `macro`: Enables the [`completion`], [`completion_async`], [`completion_async_move`] and
//! [`completion_stream`] macros, on by default.
#![cfg_attr(not(feature = "std"), no_std)]
#![warn(
    clippy::pedantic,
    clippy::wrong_pub_self_convention,
    rust_2018_idioms,
    missing_docs,
    unused_qualifications,
    missing_debug_implementations,
    explicit_outlives_requirements,
    unused_lifetimes
)]
#![allow(
    clippy::module_name_repetitions,
    clippy::shadow_unrelated,
    clippy::clippy::option_if_let_else
)]

#[cfg(feature = "alloc")]
extern crate alloc;

use core::future::Future;
use core::ops::{Deref, DerefMut};
use core::pin::Pin;
use core::task::{Context, Poll};

#[doc(no_inline)]
pub use completion_core::{CompletionFuture, CompletionStream};
use futures_core::Stream;
use pin_project_lite::pin_project;

pub mod future;
#[cfg(feature = "alloc")]
pub use self::future::{BoxCompletionFuture, LocalBoxCompletionFuture};
#[doc(no_inline)]
pub use self::future::{CompletionFutureExt, FutureExt};

pub mod stream;
#[cfg(feature = "alloc")]
pub use self::stream::{BoxCompletionStream, LocalBoxCompletionStream};
#[doc(no_inline)]
pub use self::stream::{CompletionStreamExt, StreamExt};

#[cfg(feature = "macro")]
mod macros;
#[cfg(feature = "macro")]
pub use macros::*;

#[cfg(feature = "std")]
pub mod io;

pin_project! {
    /// Unsafely assert that the inner future or stream will complete.
    ///
    /// This will wrap a [`CompletionFuture`] or [`CompletionStream`] and implement [`Future`] or
    /// [`Stream`] for it respectively.
    ///
    /// It can be used in conjunction with [`MustComplete`] to apply [`Future`]-only combinators to
    /// [`CompletionFuture`]s.
    #[derive(Debug, Clone)]
    #[must_use = "futures and streams do nothing unless you use them"]
    pub struct AssertCompletes<T> {
        #[pin]
        inner: T,
    }
}

impl<T> AssertCompletes<T> {
    /// Create a new `AssertCompletes` around a future or stream that must complete.
    ///
    /// # Safety
    ///
    /// This future or stream, once polled, must be polled to completion.
    pub unsafe fn new(inner: T) -> Self {
        Self { inner }
    }

    /// Take the inner item.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T> Deref for AssertCompletes<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> DerefMut for AssertCompletes<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<T: CompletionFuture> Future for AssertCompletes<T> {
    type Output = T::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { self.project().inner.poll(cx) }
    }
}
impl<T: CompletionFuture> CompletionFuture for AssertCompletes<T> {
    type Output = T::Output;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().inner.poll(cx)
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.project().inner.poll_cancel(cx)
    }
}
impl<T: CompletionStream> Stream for AssertCompletes<T> {
    type Item = T::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { self.project().inner.poll_next(cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}
impl<T: CompletionStream> CompletionStream for AssertCompletes<T> {
    type Item = T::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().inner.poll_next(cx)
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.project().inner.poll_cancel(cx)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

/// Helper type to implement [`CompletionFuture`] or [`CompletionStream`] for a type that only
/// implements [`Future`] or [`Stream`].
///
/// This is typically created through the [`FutureExt::into_completion`] and
/// [`StreamExt::into_completion`] methods.
///
/// # Examples
///
/// ```
/// // This future doesn't implement `CompletionFuture`, so to use it in `block_on` we need to wrap
/// // it in Adapter.
/// let future = async {};
/// completion::future::block_on(completion::Adapter(future));
/// ```
#[derive(Debug, Clone)]
pub struct Adapter<T>(pub T);

impl<T> Adapter<T> {
    /// Get a pinned shared reference to the inner future or stream.
    #[must_use]
    pub fn get_pin_ref(self: Pin<&Self>) -> Pin<&T> {
        unsafe { self.map_unchecked(|this| &this.0) }
    }

    /// Get a pinned mutable reference to the inner future or stream.
    #[must_use]
    pub fn get_pin_mut(self: Pin<&mut Self>) -> Pin<&mut T> {
        unsafe { self.map_unchecked_mut(|this| &mut this.0) }
    }
}

impl<T: Future> Future for Adapter<T> {
    type Output = T::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_pin_mut().poll(cx)
    }
}

impl<T: Future> CompletionFuture for Adapter<T> {
    type Output = T::Output;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_pin_mut().poll(cx)
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        Poll::Ready(())
    }
}

impl<T: Stream> Stream for Adapter<T> {
    type Item = T::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_pin_mut().poll_next(cx)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<T: Stream> CompletionStream for Adapter<T> {
    type Item = T::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_pin_mut().poll_next(cx)
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        Poll::Ready(())
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

pin_project! {
    /// Make sure that a future or stream will complete, created by
    /// [`CompletionFutureExt::must_complete`] and [`CompletionStreamExt::must_complete`].
    ///
    /// This wraps a [`CompletionFuture`] or [`CompletionStream`] and will ignore all requests to
    /// cancel it, enforcing that it will complete.
    #[derive(Debug, Clone)]
    pub struct MustComplete<T> {
        #[pin]
        inner: T,
    }
}

impl<T> MustComplete<T> {
    /// Take the inner item.
    ///
    /// # Safety
    ///
    /// This value must not be cancelled with [`CompletionFuture::poll_cancel`] or
    /// [`CompletionStream::poll_cancel`].
    pub unsafe fn into_inner(self) -> T {
        self.inner
    }
}

impl<T: CompletionFuture> CompletionFuture for MustComplete<T> {
    type Output = T::Output;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().inner.poll(cx)
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.project().inner.poll(cx).map(drop)
    }
}

impl<T: CompletionStream> CompletionStream for MustComplete<T> {
    type Item = T::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().inner.poll_next(cx)
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.project().inner.poll_next(cx).map(drop)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

#[cfg(test)]
mod test_utils {
    use core::future::Future;
    use core::pin::Pin;
    use core::task::{Context, Poll};

    use completion_core::CompletionFuture;

    pin_project_lite::pin_project! {
        /// Future that yields once, before polling the inner future.
        pub(super) struct Yield<F> {
            times: usize,
            #[pin]
            fut: F,
        }
    }
    impl<F> Yield<F> {
        #[cfg_attr(not(feature = "std"), allow(dead_code))]
        pub(super) fn new(times: usize, fut: F) -> Self {
            Self { times, fut }
        }
        #[cfg_attr(not(feature = "std"), allow(dead_code))]
        pub(super) fn once(fut: F) -> Self {
            Self::new(1, fut)
        }
    }
    impl<F: CompletionFuture> CompletionFuture for Yield<F> {
        type Output = F::Output;

        unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let this = self.project();
            if *this.times > 0 {
                *this.times -= 1;
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                this.fut.poll(cx)
            }
        }
        unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            let this = self.project();
            if *this.times > 0 {
                *this.times -= 1;
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                this.fut.poll_cancel(cx)
            }
        }
    }
    impl<F: CompletionFuture + Future> Future for Yield<F> {
        type Output = <F as CompletionFuture>::Output;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            unsafe { CompletionFuture::poll(self, cx) }
        }
    }

    #[cfg_attr(not(feature = "std"), allow(dead_code))]
    pub(super) fn now_or_never<F: Future>(fut: F) -> Option<F::Output> {
        use core::ptr;
        use core::task::{RawWaker, RawWakerVTable, Waker};

        const WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(|_| RAW_WAKER, drop, drop, drop);
        const RAW_WAKER: RawWaker = RawWaker::new(ptr::null(), &WAKER_VTABLE);

        let waker = unsafe { Waker::from_raw(RAW_WAKER) };
        let mut cx = Context::from_waker(&waker);

        futures_lite::pin!(fut);
        match fut.poll(&mut cx) {
            Poll::Ready(val) => Some(val),
            Poll::Pending => None,
        }
    }
}
