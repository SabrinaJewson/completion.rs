//! Utilities for the [`CompletionFuture`] trait.

#[cfg(feature = "alloc")]
use alloc::boxed::Box;
#[cfg(feature = "std")]
use core::any::Any;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
#[cfg(feature = "std")]
use std::panic::{catch_unwind, AssertUnwindSafe, UnwindSafe};

#[cfg(feature = "std")]
use pin_project_lite::pin_project;

#[doc(no_inline)]
pub use completion_core::CompletionFuture;

use super::{Adapter, MustComplete};

#[cfg(feature = "std")]
mod block_on;
#[cfg(feature = "std")]
pub use block_on::block_on;

#[cfg(feature = "std")]
mod zip;
#[cfg(feature = "std")]
pub use zip::{zip, Zip};

/// Extension trait for [`CompletionFuture`].
pub trait CompletionFutureExt: CompletionFuture {
    /// A convenience for calling [`CompletionFuture::poll`] on [`Unpin`] futures.
    ///
    /// # Safety
    ///
    /// Identical to [`CompletionFuture::poll`].
    unsafe fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Self::Output>
    where
        Self: Unpin,
    {
        Pin::new(self).poll(cx)
    }

    /// A convenience for calling [`CompletionFuture::poll_cancel`] on [`Unpin`] futures.
    ///
    /// # Safety
    ///
    /// Identical to [`CompletionFuture::poll_cancel`].
    unsafe fn poll_cancel(&mut self, cx: &mut Context<'_>) -> Poll<()>
    where
        Self: Unpin,
    {
        Pin::new(self).poll_cancel(cx)
    }

    /// Make sure that the future will complete. Any requests to cancel the future through
    /// [`poll_cancel`](CompletionFuture::poll_cancel) will be ignored.
    fn must_complete(self) -> MustComplete<Self>
    where
        Self: Sized,
    {
        MustComplete { inner: self }
    }

    /// Catch panics in the future.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionFutureExt, completion_async};
    ///
    /// # completion::future::block_on(completion_async! {
    /// let future = completion_async!(panic!());
    /// assert!(future.catch_unwind().await.is_err());
    /// # });
    /// ```
    #[cfg(feature = "std")]
    #[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
    fn catch_unwind(self) -> CatchUnwind<Self>
    where
        Self: Sized + UnwindSafe,
    {
        CatchUnwind { inner: self }
    }

    /// Box the future, erasing its type.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionFutureExt, completion_async};
    ///
    /// # let some_condition = true;
    /// // These futures are different types, but boxing them makes them the same type.
    /// let fut = if some_condition {
    ///     completion_async!(5).boxed()
    /// } else {
    ///     completion_async!(6).boxed()
    /// };
    /// ```
    #[cfg(feature = "alloc")]
    #[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
    fn boxed<'a>(self) -> BoxCompletionFuture<'a, Self::Output>
    where
        Self: Sized + Send + 'a,
    {
        Box::pin(self)
    }

    /// Box the future locally, erasing its type.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionFutureExt, completion_async};
    ///
    /// # let some_condition = true;
    /// // These futures are different types, but boxing them makes them the same type.
    /// let fut = if some_condition {
    ///     completion_async!(5).boxed_local()
    /// } else {
    ///     completion_async!(6).boxed_local()
    /// };
    /// ```
    #[cfg(feature = "alloc")]
    #[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
    fn boxed_local<'a>(self) -> LocalBoxCompletionFuture<'a, Self::Output>
    where
        Self: Sized + 'a,
    {
        Box::pin(self)
    }
}

impl<T: CompletionFuture + ?Sized> CompletionFutureExt for T {}

#[cfg(feature = "std")]
pin_project! {
    /// Future for [`CompletionFutureExt::catch_unwind`].
    #[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you use them"]
    pub struct CatchUnwind<F: ?Sized> {
        #[pin]
        inner: F,
    }
}

#[cfg(feature = "std")]
impl<F: CompletionFuture + UnwindSafe + ?Sized> CompletionFuture for CatchUnwind<F> {
    type Output = Result<F::Output, Box<dyn Any + Send>>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        catch_unwind(AssertUnwindSafe(|| self.project().inner.poll(cx)))?.map(Ok)
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        catch_unwind(AssertUnwindSafe(|| self.project().inner.poll_cancel(cx)))
            .unwrap_or(Poll::Ready(()))
    }
}

#[cfg(feature = "std")]
impl<F: Future + UnwindSafe + ?Sized> Future for CatchUnwind<F> {
    type Output = Result<F::Output, Box<dyn Any + Send>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        catch_unwind(AssertUnwindSafe(|| self.project().inner.poll(cx)))?.map(Ok)
    }
}

/// A type-erased completion future.
#[cfg(feature = "alloc")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
pub type BoxCompletionFuture<'a, T> = Pin<Box<dyn CompletionFuture<Output = T> + Send + 'a>>;

/// A type-erased completion future that cannot be send across threads.
#[cfg(feature = "alloc")]
#[cfg_attr(doc_cfg, doc(cfg(feature = "alloc")))]
pub type LocalBoxCompletionFuture<'a, T> = Pin<Box<dyn CompletionFuture<Output = T> + 'a>>;

/// Extension trait for converting [`Future`]s to [`CompletionFuture`]s.
pub trait FutureExt: Future + Sized {
    /// Convert this future into a [`CompletionFuture`].
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::FutureExt;
    ///
    /// let completion_future = async { 19 }.into_completion();
    /// ```
    fn into_completion(self) -> Adapter<Self> {
        Adapter(self)
    }
}
impl<T: Future> FutureExt for T {}
