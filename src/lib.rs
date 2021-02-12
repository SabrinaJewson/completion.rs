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
#![cfg_attr(doc_cfg, feature(doc_cfg))]
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
    clippy::option_if_let_else,
    clippy::items_after_statements
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
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
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
    #[cfg(feature = "std")]
    use core::task::Waker;
    use core::task::{Context, Poll};

    use completion_core::CompletionFuture;
    use pin_project_lite::pin_project;

    pub(crate) trait CompletionFutureExt: CompletionFuture + Sized {
        /// Dynamically check the future is polled correctly.
        fn check(self) -> Check<Self> {
            Check {
                fut: self,
                inner: CheckInner {
                    state: CheckState::Running,
                    max_polls: None,
                    max_cancels: None,
                    polled_once: false,
                },
            }
        }
    }
    impl<T: CompletionFuture> CompletionFutureExt for T {}

    pin_project! {
        pub(crate) struct Check<F> {
            #[pin]
            fut: F,
            inner: CheckInner,
        }
    }
    /// A separate type that holds the non-future state of a Check so that it supports checking on
    /// Drop.
    struct CheckInner {
        state: CheckState,
        max_polls: Option<usize>,
        max_cancels: Option<usize>,
        polled_once: bool,
    }
    impl Drop for CheckInner {
        fn drop(&mut self) {
            if self.polled_once {
                assert_eq!(self.state, CheckState::Finished);
            }
        }
    }

    #[derive(Debug, PartialEq)]
    enum CheckState {
        Running,
        Cancelling,
        Finished,
    }

    impl<F> Check<F> {
        #[cfg(feature = "std")]
        pub(crate) fn max_polls(mut self, max_polls: usize) -> Self {
            self.inner.max_polls = Some(max_polls);
            self
        }
        #[cfg(feature = "std")]
        pub(crate) fn max_cancels(mut self, max_cancels: usize) -> Self {
            self.inner.max_cancels = Some(max_cancels);
            self
        }
    }
    impl<F: CompletionFuture> CompletionFuture for Check<F> {
        type Output = F::Output;
        #[track_caller]
        unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let this = self.project();
            assert_eq!(this.inner.state, CheckState::Running);
            if let Some(max_polls) = &mut this.inner.max_polls {
                assert_ne!(*max_polls, 0);
                *max_polls -= 1;
            }
            this.inner.polled_once = true;
            this.inner.state = CheckState::Finished;
            let poll = this.fut.poll(cx);
            if poll.is_pending() {
                this.inner.state = CheckState::Running;
            }
            poll
        }
        #[track_caller]
        unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            let this = self.project();
            assert_ne!(this.inner.state, CheckState::Finished);
            if let Some(max_cancels) = &mut this.inner.max_cancels {
                assert_ne!(*max_cancels, 0);
                *max_cancels -= 1;
            }
            this.inner.polled_once = true;
            this.inner.state = CheckState::Finished;
            let poll = this.fut.poll_cancel(cx);
            if poll.is_pending() {
                this.inner.state = CheckState::Cancelling;
            }
            poll
        }
    }

    pin_project! {
        /// Future that yields a number of times, before polling the inner future.
        pub(crate) struct Yield<F> {
            times: usize,
            #[pin]
            fut: F,
        }
    }
    impl<F> Yield<F> {
        #[cfg(feature = "std")]
        pub(crate) fn new(times: usize, fut: F) -> Self {
            Self { times, fut }
        }
        #[cfg(feature = "std")]
        pub(crate) fn once(fut: F) -> Self {
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

    #[cfg(feature = "std")]
    pub(crate) async fn sleep(duration: core::time::Duration) {
        use core::sync::atomic::{self, AtomicBool};
        use std::sync::Arc;
        use std::thread;

        use atomic_waker::AtomicWaker;

        struct Sleep {
            inner: Arc<SleepInner>,
        }
        struct SleepInner {
            done: AtomicBool,
            waker: AtomicWaker,
        }
        impl Future for Sleep {
            type Output = ();
            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                self.inner.waker.register(cx.waker());
                if self.inner.done.load(atomic::Ordering::SeqCst) {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }
        }

        let inner = Arc::new(SleepInner {
            done: AtomicBool::new(false),
            waker: AtomicWaker::new(),
        });
        thread::spawn({
            let inner = Arc::clone(&inner);
            move || {
                thread::sleep(duration);
                inner.done.store(true, atomic::Ordering::SeqCst);
                inner.waker.wake();
            }
        });
        Sleep { inner }.await
    }

    #[cfg(feature = "std")]
    pub(crate) fn noop_waker() -> Waker {
        use core::ptr;
        use core::task::{RawWaker, RawWakerVTable};

        const WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(|_| RAW_WAKER, drop, drop, drop);
        const RAW_WAKER: RawWaker = RawWaker::new(ptr::null(), &WAKER_VTABLE);

        unsafe { Waker::from_raw(RAW_WAKER) }
    }

    #[cfg(feature = "std")]
    pub(crate) fn poll_once<F: CompletionFuture>(fut: F) -> Option<F::Output> {
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        futures_lite::pin!(fut);
        match unsafe { fut.poll(&mut cx) } {
            Poll::Ready(val) => Some(val),
            Poll::Pending => None,
        }
    }

    #[cfg(all(feature = "macro", feature = "std"))]
    pub(crate) fn poll_cancel_once<F: CompletionFuture>(fut: F) -> bool {
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        futures_lite::pin!(fut);
        unsafe { fut.poll_cancel(&mut cx) }.is_ready()
    }
}
