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

use super::MustComplete;

#[cfg(feature = "std")]
mod block_on;
#[cfg(feature = "std")]
pub use block_on::block_on;

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

    /// Catch panics in the future.
    ///
    /// Requires the `std` feature.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion_util::{CompletionFutureExt, completion_async};
    ///
    /// # completion_util::future::block_on(completion_async! {
    /// let future = completion_async!(panic!());
    /// assert!(future.catch_unwind().await.is_err());
    /// # });
    /// ```
    #[cfg(feature = "std")]
    fn catch_unwind(self) -> CatchUnwind<Self>
    where
        Self: Sized + UnwindSafe,
    {
        CatchUnwind { inner: self }
    }

    /// Box the future, erasing its type.
    ///
    /// Requires the `alloc` feature.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion_util::{CompletionFutureExt, completion_async};
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
    fn boxed<'a>(self) -> BoxCompletionFuture<'a, Self::Output>
    where
        Self: Sized + Send + 'a,
    {
        Box::pin(self)
    }

    /// Box the future locally, erasing its type.
    ///
    /// Requires the `alloc` feature.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion_util::{CompletionFutureExt, completion_async};
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
    ///
    /// Requires the `std` feature.
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
}

#[cfg(feature = "std")]
impl<F: Future + UnwindSafe + ?Sized> Future for CatchUnwind<F> {
    type Output = Result<F::Output, Box<dyn Any + Send>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        catch_unwind(AssertUnwindSafe(|| self.project().inner.poll(cx)))?.map(Ok)
    }
}

/// A type-erased completion future.
///
/// Requires the `alloc` feature.
#[cfg(feature = "alloc")]
pub type BoxCompletionFuture<'a, T> = Pin<Box<dyn CompletionFuture<Output = T> + Send + 'a>>;

/// A type-erased completion future that cannot be send across threads.
///
/// Requires the `alloc` feature.
#[cfg(feature = "alloc")]
pub type LocalBoxCompletionFuture<'a, T> = Pin<Box<dyn CompletionFuture<Output = T> + 'a>>;

/// Joins two futures, waiting for them both to complete.
///
/// Requires the `std` feature, as [`catch_unwind`] is needed when polling the futures to ensure
/// soundness.
///
/// # Examples
///
/// ```
/// use completion_util::{future, completion_async};
///
/// # future::block_on(completion_async! {
/// assert_eq!(
///     future::zip(
///         completion_async!(5),
///         completion_async!(6),
///     )
///     .await,
///     (5, 6),
/// );
/// # });
/// ```
#[cfg(feature = "std")]
pub fn zip<F1: CompletionFuture, F2: CompletionFuture>(future1: F1, future2: F2) -> Zip<F1, F2> {
    Zip {
        future1,
        output1: None,
        future2,
        output2: None,
    }
}

#[cfg(feature = "std")]
pin_project! {
    /// Future for [`zip`].
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you use them"]
    pub struct Zip<F1: CompletionFuture, F2: CompletionFuture> {
        #[pin]
        future1: F1,
        output1: Option<Result<F1::Output, Box<dyn Any + Send>>>,
        #[pin]
        future2: F2,
        output2: Option<Result<F2::Output, Box<dyn Any + Send>>>,
    }
}

#[cfg(feature = "std")]
impl<F1: CompletionFuture, F2: CompletionFuture> CompletionFuture for Zip<F1, F2> {
    type Output = (F1::Output, F2::Output);

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        if this.output1.is_none() {
            match catch_unwind(AssertUnwindSafe(|| this.future1.as_mut().poll(cx))) {
                Ok(Poll::Ready(output)) => {
                    *this.output1 = Some(Ok(output));
                }
                Ok(Poll::Pending) => {}
                Err(payload) => {
                    *this.output1 = Some(Err(payload));
                }
            }
        }
        if this.output2.is_none() {
            match catch_unwind(AssertUnwindSafe(|| this.future2.as_mut().poll(cx))) {
                Ok(Poll::Ready(output)) => {
                    *this.output2 = Some(Ok(output));
                }
                Ok(Poll::Pending) => {}
                Err(payload) => {
                    *this.output2 = Some(Err(payload));
                }
            }
        }

        if this.output1.is_some() && this.output2.is_some() {
            let output1 = this.output1.take().unwrap();
            let output2 = this.output2.take().unwrap();
            let (output1, output2) = match (output1, output2) {
                (Ok(output1), Ok(output2)) => (output1, output2),
                (Err(output1), _) => panic!(output1),
                (_, Err(output2)) => panic!(output2),
            };
            Poll::Ready((output1, output2))
        } else {
            Poll::Pending
        }
    }
}

/// Extension trait for converting [`Future`]s to [`CompletionFuture`]s.
pub trait FutureExt: Future + Sized {
    /// Make sure that the future will complete. Equivalent to [`MustComplete::new`].
    ///
    /// # Examples
    ///
    /// ```
    /// use completion_util::FutureExt;
    ///
    /// let completion_future = async { 19 }.must_complete();
    /// ```
    fn must_complete(self) -> MustComplete<Self> {
        MustComplete::new(self)
    }
}
impl<T: Future> FutureExt for T {}
