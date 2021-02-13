//! Futures that join together several futures.

use core::any::Any;
use core::convert::Infallible;
use core::fmt::{self, Debug, Formatter};
use core::mem;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::panic::{catch_unwind, AssertUnwindSafe};

use completion_core::CompletionFuture;
use pin_project_lite::pin_project;

mod tuple;
pub use tuple::*;

mod all;
pub use all::*;

/// The payload of a panic.
pub struct Panic(Box<dyn Any + Send>);
unsafe impl Sync for Panic {}
impl Debug for Panic {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.pad("Panic")
    }
}
impl Panic {
    fn into_inner(self) -> Box<dyn Any + Send> {
        self.0
    }
}

/// A future that outputs a `Result`.
pub trait TryFuture:
    CompletionFuture<Output = Result<<Self as TryFuture>::Ok, <Self as TryFuture>::Error>>
{
    type Ok;
    type Error;
}
impl<T, E, F> TryFuture for F
where
    F: CompletionFuture<Output = Result<T, E>> + ?Sized,
{
    type Ok = T;
    type Error = E;
}

/// How a future can choose to affect control flow of its containing joined future.
#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub enum ControlFlow<B, C = ()> {
    /// The future wishes to discard the results of all the other futures and return this value to
    /// the caller.
    Break(B),
    /// The future wishes to wait for all the other futures to complete and return all their
    /// results together.
    Continue(C),
}

#[cfg(test)]
impl<B, C> ControlFlow<B, C> {
    #[track_caller]
    fn unwrap_break(self) -> B
    where
        C: Debug,
    {
        match self {
            Self::Continue(c) => panic!("Expected `Break`, found `Continue({:?})`", c),
            Self::Break(b) => b,
        }
    }

    #[track_caller]
    fn unwrap_continue(self) -> C
    where
        B: Debug,
    {
        match self {
            Self::Continue(c) => c,
            Self::Break(b) => panic!("Expected `Continue`, found `Break({:?})`", b),
        }
    }
}

/// A future that outputs a `ControlFlow`.
pub trait ControlFlowFuture:
    CompletionFuture<
    Output = ControlFlow<<Self as ControlFlowFuture>::Break, <Self as ControlFlowFuture>::Continue>,
>
{
    type Continue;
    type Break;
}
impl<C, B, F> ControlFlowFuture for F
where
    F: CompletionFuture<Output = ControlFlow<B, C>> + ?Sized,
{
    type Continue = C;
    type Break = B;
}

/// The state of a `ControlFlowFuture`.
#[derive(Debug)]
pub enum FutureState<F: ControlFlowFuture> {
    /// The future is still running.
    Running(F),
    /// The future has completed and wishes to continue control flow.
    Completed(F::Continue),
    /// The future has been cancelled.
    Cancelled,
    /// The future's output has been taken.
    Taken,
}
impl<F: ControlFlowFuture + Unpin> Unpin for FutureState<F> {}

impl<F: ControlFlowFuture> FutureState<F> {
    /// The the output of a `FutureState`, if present.
    fn take_output(self: Pin<&mut Self>) -> Option<F::Continue> {
        if let Self::Completed(_) = &*self {
            Some(
                match mem::replace(unsafe { Pin::into_inner_unchecked(self) }, Self::Taken) {
                    Self::Completed(c) => c,
                    _ => unreachable!(),
                },
            )
        } else {
            None
        }
    }

    /// Poll the inner future, catching any panics that can occur.
    unsafe fn poll(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<ControlFlow<Result<F::Break, Panic>>> {
        let this = Pin::into_inner_unchecked(self);
        match this {
            Self::Running(fut) => {
                let fut = Pin::new_unchecked(fut);
                match catch_unwind(AssertUnwindSafe(|| fut.poll(cx))) {
                    Ok(Poll::Ready(ControlFlow::Continue(c))) => {
                        *this = Self::Completed(c);
                        Poll::Ready(ControlFlow::Continue(()))
                    }
                    Ok(Poll::Ready(ControlFlow::Break(b))) => {
                        *this = Self::Cancelled;
                        Poll::Ready(ControlFlow::Break(Ok(b)))
                    }
                    Ok(Poll::Pending) => Poll::Pending,
                    Err(panic) => Poll::Ready(ControlFlow::Break(Err(Panic(panic)))),
                }
            }
            Self::Completed(_) => Poll::Ready(ControlFlow::Continue(())),
            // Panic when the future has been cancelled as at that point the joiner should be
            // calling `poll_cancel` or `poll_panicked`.
            Self::Taken | Self::Cancelled => panic!(),
        }
    }

    /// Attempt to cancel the inner future, catching any panics that can occur.
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<ControlFlow<Panic>> {
        let this = Pin::into_inner_unchecked(self);
        match this {
            Self::Running(fut) => {
                let fut = Pin::new_unchecked(fut);
                match catch_unwind(AssertUnwindSafe(|| fut.poll_cancel(cx))) {
                    Ok(Poll::Ready(())) => {
                        *this = Self::Cancelled;
                        Poll::Ready(ControlFlow::Continue(()))
                    }
                    Ok(Poll::Pending) => Poll::Pending,
                    Err(e) => {
                        *this = Self::Cancelled;
                        Poll::Ready(ControlFlow::Break(Panic(e)))
                    }
                }
            }
            Self::Completed(_) | Self::Cancelled => Poll::Ready(ControlFlow::Continue(())),
            Self::Taken => panic!(),
        }
    }

    /// Attempt to cancel the inner future, ignoring panics.
    unsafe fn poll_panicked(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<ControlFlow<Infallible>> {
        let this = Pin::into_inner_unchecked(self);
        match this {
            Self::Running(fut) => {
                let fut = Pin::new_unchecked(fut);
                match catch_unwind(AssertUnwindSafe(|| fut.poll_cancel(cx))) {
                    Ok(Poll::Ready(())) | Err(_) => {
                        *this = Self::Cancelled;
                        Poll::Ready(ControlFlow::Continue(()))
                    }
                    Ok(Poll::Pending) => Poll::Pending,
                }
            }
            Self::Completed(_) | Self::Cancelled => Poll::Ready(ControlFlow::Continue(())),
            Self::Taken => panic!(),
        }
    }
}

pin_project! {
    /// A wrapper for a future inside `zip` or `zip_all`.
    #[derive(Debug)]
    pub struct ZipFuture<F> {
        #[pin]
        inner: F,
    }
}
impl<F> ZipFuture<F> {
    fn new(inner: F) -> Self {
        Self { inner }
    }
}
impl<F: CompletionFuture> CompletionFuture for ZipFuture<F> {
    type Output = ControlFlow<Infallible, F::Output>;
    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().inner.poll(cx).map(ControlFlow::Continue)
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.project().inner.poll_cancel(cx)
    }
}

pin_project! {
    /// A wrapper for a future inside `try_zip` or `try_zip_all`.
    #[derive(Debug)]
    pub struct TryZipFuture<F> {
        #[pin]
        inner: F,
    }
}
impl<F> TryZipFuture<F> {
    fn new(inner: F) -> Self {
        Self { inner }
    }
}
impl<F, T, E> CompletionFuture for TryZipFuture<F>
where
    F: CompletionFuture<Output = Result<T, E>>,
{
    type Output = ControlFlow<E, T>;
    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().inner.poll(cx).map(|res| match res {
            Ok(val) => ControlFlow::Continue(val),
            Err(e) => ControlFlow::Break(e),
        })
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.project().inner.poll_cancel(cx)
    }
}

pin_project! {
    /// A wrapper for a future inside `race` or `race_all`.
    #[derive(Debug)]
    pub struct RaceFuture<F> {
        #[pin]
        inner: F,
    }
}
impl<F> RaceFuture<F> {
    fn new(inner: F) -> Self {
        Self { inner }
    }
}
impl<F: CompletionFuture> CompletionFuture for RaceFuture<F> {
    type Output = ControlFlow<F::Output, Infallible>;
    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().inner.poll(cx).map(ControlFlow::Break)
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.project().inner.poll_cancel(cx)
    }
}

pin_project! {
    /// A wrapper for a future inside `race_ok` or `race_ok_all`.
    #[derive(Debug)]
    pub struct RaceOkFuture<F> {
        #[pin]
        inner: F,
    }
}
impl<F> RaceOkFuture<F> {
    fn new(inner: F) -> Self {
        Self { inner }
    }
}
impl<F, T, E> CompletionFuture for RaceOkFuture<F>
where
    F: CompletionFuture<Output = Result<T, E>>,
{
    type Output = ControlFlow<T, E>;
    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().inner.poll(cx).map(|res| match res {
            Ok(val) => ControlFlow::Break(val),
            Err(e) => ControlFlow::Continue(e),
        })
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.project().inner.poll_cancel(cx)
    }
}
