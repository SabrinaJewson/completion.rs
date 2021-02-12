//! Futures that join together several futures.

use core::any::Any;
use core::convert::Infallible;
use core::fmt::{self, Debug, Formatter};
use core::mem;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::panic::{catch_unwind, AssertUnwindSafe};

use completion_core::CompletionFuture;

mod tuple;
pub use tuple::{zip, Zip};

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

/// How a future can choose to affect control flow of its containing joined future.
#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq, Eq))]
pub enum ControlFlow<B, C = ()> {
    /// The future wishes to wait for all the other futures to complete and return all their
    /// results together.
    Continue(C),
    /// The future wishes to discard the results of all the other futures and return this value to
    /// the caller.
    Break(B),
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
