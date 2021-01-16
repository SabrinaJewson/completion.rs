//! Utilities for writing completion-based asynchronous code.
//!
//! # Features
//!
//! - `std`: Enables features that require the standard library, on by default.
//! - `alloc`: Enables features that require allocation, on by default.
//! - `macro`: Enables the [`completion`], [`completion_async`], [`completion_async_move`] and
//! [`completion_stream`] macros.
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
#![allow(clippy::module_name_repetitions, clippy::shadow_unrelated)]

#[cfg(feature = "alloc")]
extern crate alloc;

use core::future::Future;
use core::ops::{Deref, DerefMut};
use core::pin::Pin;
use core::task::{Context, Poll};

use futures_core::Stream;
use pin_project_lite::pin_project;

pub mod future;
#[cfg(feature = "alloc")]
pub use self::future::{BoxCompletionFuture, LocalBoxCompletionFuture};
#[doc(no_inline)]
pub use self::future::{CompletionFuture, CompletionFutureExt, FutureExt};

pub mod stream;
#[cfg(feature = "alloc")]
pub use self::stream::{BoxCompletionStream, LocalBoxCompletionStream};
#[doc(no_inline)]
pub use self::stream::{CompletionStream, CompletionStreamExt, StreamExt};

#[cfg(feature = "macro")]
mod macros;
#[cfg(feature = "macro")]
pub use macros::*;

#[cfg(feature = "std")]
pub mod io;

pin_project! {
    /// Unsafely assert that the inner future or stream will complete.
    ///
    /// This is typically used in conjunction with [`MustComplete`] to apply [`Future`]-only
    /// combinators to [`CompletionFuture`]s.
    ///
    /// # Examples
    ///
    /// Use a [`CompletionFuture`] in an async block:
    ///
    /// ```
    /// use completion::{AssertCompletes, MustComplete};
    ///
    /// let future = MustComplete::new(async {
    /// # let completion_future = MustComplete::new(async {});
    ///     unsafe { AssertCompletes::new(completion_future) }.await;
    /// });
    /// ```
    ///
    /// Note that the [`completion_async!`] macro is a better way to achieve this.
    #[derive(Debug)]
    #[must_use = "futures and streams do nothing unless you use them"]
    pub struct AssertCompletes<T: ?Sized> {
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

impl<T: ?Sized> Deref for AssertCompletes<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T: ?Sized> DerefMut for AssertCompletes<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<T: CompletionFuture + ?Sized> Future for AssertCompletes<T> {
    type Output = T::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { self.project().inner.poll(cx) }
    }
}
impl<T: CompletionFuture + ?Sized> CompletionFuture for AssertCompletes<T> {
    type Output = T::Output;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().inner.poll(cx)
    }
}
impl<T: CompletionStream + ?Sized> Stream for AssertCompletes<T> {
    type Item = T::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { self.project().inner.poll_next(cx) }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}
impl<T: CompletionStream + ?Sized> CompletionStream for AssertCompletes<T> {
    type Item = T::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().inner.poll_next(cx)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

pin_project! {
    /// Make sure that a future or stream will complete.
    ///
    /// This is typically used in conjunction with [`AssertCompletes`] to apply [`Future`]-only
    /// combinators to [`CompletionFuture`]s. See [`AssertCompletes`] for details and examples.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::MustComplete;
    ///
    /// async fn send_request() {
    ///     /* Send a request to a server */
    /// }
    ///
    /// let request_future = MustComplete::new(send_request());
    /// // Now you can be sure that the request will finish sending.
    /// ```
    #[derive(Debug)]
    #[must_use = "futures and streams do nothing unless you use them"]
    pub struct MustComplete<T: ?Sized> {
        #[pin]
        inner: T,
    }
}

impl<T> MustComplete<T> {
    /// Make sure that the given future or stream will complete.
    ///
    /// [`FutureExt::must_complete`] is generally preferred over calling this.
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    /// Take the inner item.
    ///
    /// # Safety
    ///
    /// This value must be polled to completion.
    pub unsafe fn into_inner(self) -> T {
        self.inner
    }
}

impl<T: Future + ?Sized> CompletionFuture for MustComplete<T> {
    type Output = T::Output;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().inner.poll(cx)
    }
}

impl<T: Stream + ?Sized> CompletionStream for MustComplete<T> {
    type Item = T::Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.project().inner.poll_next(cx)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}
