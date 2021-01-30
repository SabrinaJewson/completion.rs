use core::cell::Cell;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::CompletionFuture;
use pin_project_lite::pin_project;

/// An attribute macro to generate completion `async fn`s. These async functions evaluate to a
/// [`CompletionFuture`], and you can `.await` other [`CompletionFuture`]s inside of them.
///
/// Requires the `macro` and `std` features.
///
/// # Examples
///
/// ```
/// use completion::{completion, completion_async};
///
/// #[completion]
/// async fn async_completion() -> i32 {
///     let fut = async { 3 };
///     let completion_fut = completion_async! { 5 };
///     fut.await + completion_fut.await
/// }
/// ```
///
/// This macro needs to know a path to this crate in order to work. By default it uses
/// `::completion`, but you can change it using the `crate` option:
///
/// ```
/// mod path {
///     pub mod to {
///         pub extern crate completion as completion_crate;
///     }
/// }
/// use path::to::completion_crate::completion;
///
/// #[completion(crate = path::to::completion_crate)]
/// async fn async_completion(x: &i32) -> i32 {
///     *x
/// }
/// ```
///
/// You can return a boxed completion future from the function by using the `box` option. For
/// example this can be used to implement async traits:
///
/// ```
/// use completion::completion;
///
/// trait MyTrait {
///     #[completion(box)]
///     async fn run(&self);
/// }
///
/// struct Item;
/// impl MyTrait for Item {
///     #[completion(box)]
///     async fn run(&self) {
///         println!("Hello!");
///     }
/// }
/// ```
///
/// By default the `box` option will require the futures to implement `Send`. If you don't want
/// this, you can pass in the `?Send` option:
///
/// ```
/// use completion::completion;
///
/// trait MyTrait {
///     #[completion(box(?Send))]
///     async fn run(&self);
/// }
/// ```
///
/// That will allow you to hold `!Send` types across await points in the future, but will only
/// allow it to be executed on a single-threaded executor.
pub use completion_macro::completion;

#[doc(hidden)]
pub use completion_macro::completion_async_inner as __completion_async_inner;
#[doc(hidden)]
pub use completion_macro::completion_async_move_inner as __completion_async_move_inner;

/// A bang macro to generate completion `async` blocks.
///
/// These async blocks evaluate to a [`CompletionFuture`], and you can `.await` other
/// [`CompletionFuture`]s inside of them.
///
/// Requires the `macro` and `std` features.
///
/// # Examples
///
/// ```
/// use completion::{FutureExt, completion_async};
///
/// let fut = completion_async! {
///     let fut = async { 3 };
///     let completion_fut = async { 5 }.into_completion();
///     fut.await + completion_fut.await
/// };
/// ```
#[macro_export]
macro_rules! completion_async {
    ($($tt:tt)*) => {
        $crate::__completion_async_inner!(($crate) $($tt)*)
    }
}
/// A bang macro to generate completion `async move` blocks.
///
/// These async blocks evaluate to a [`CompletionFuture`], and you can `.await` other
/// [`CompletionFuture`]s inside of them.
///
/// Requires the `macro` and `std` features.
///
/// # Examples
///
/// ```
/// use completion::{FutureExt, completion_async_move};
///
/// let fut = completion_async_move! {
///     let fut = async { 3 };
///     let completion_fut = async { 5 }.into_completion();
///     fut.await + completion_fut.await
/// };
/// ```
#[macro_export]
macro_rules! completion_async_move {
    ($($tt:tt)*) => {
        $crate::__completion_async_move_inner!(($crate) $($tt)*)
    }
}

thread_local! {
    /// When the current async block is awaiting on a completion future, this will be set if it
    /// wishes to call `poll_cancel` and will be cleared if it wishes to call `poll`. If it is
    /// awaiting on a regular future, it will be cleared (`poll_cancel` cannot be called on a
    /// regular future).
    ///
    /// When a completion future returns `Poll::Pending` from either `poll` or `poll_cancel` this
    /// will be set, otherwise it will be cleared.
    static FLAG: Cell<bool> = Cell::new(false);
}

/// Make a completion future async block.
#[doc(hidden)]
pub fn __completion_async<F: Future>(fut: F) -> impl CompletionFuture<Output = F::Output> {
    pin_project! {
        #[doc(hidden)]
        struct Wrapper<F> {
            #[pin]
            fut: F,
            // Whether we are currently `.await`ing on a completion future.
            awaiting_on_completion: bool,
        }
    }
    impl<F: Future> CompletionFuture for Wrapper<F> {
        type Output = F::Output;

        unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let this = self.project();

            FLAG.with(|cell| cell.set(false));

            let poll = this.fut.poll(cx);
            if poll.is_pending() {
                *this.awaiting_on_completion = FLAG.with(Cell::get);
            }
            poll
        }
        unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            let this = self.project();
            if *this.awaiting_on_completion {
                FLAG.with(|cell| cell.set(true));

                let poll = this.fut.poll(cx);
                debug_assert!(poll.is_pending());

                if FLAG.with(Cell::get) {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            } else {
                Poll::Ready(())
            }
        }
    }

    Wrapper {
        fut,
        awaiting_on_completion: false,
    }
}

mod r#await {
    use pin_project_lite::pin_project;

    pin_project! {
        #[derive(Debug)]
        pub struct Await<F> {
            #[pin]
            pub(super) fut: F,
        }
    }
}
use r#await::Await;

impl<F: CompletionFuture> Future for Await<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        if FLAG.with(Cell::get) {
            let poll = unsafe { this.fut.poll_cancel(cx) };
            FLAG.with(|cell| cell.set(poll.is_ready()));
            Poll::Pending
        } else {
            unsafe { this.fut.poll(cx) }
        }
    }
}

// We want to be able to `.await` both regular futures and completion futures in blocks generated
// by the macro. To do this, we use inherent method-based specialization.

#[doc(hidden)]
#[allow(missing_debug_implementations)]
pub struct __FutureOrCompletionFuture<F>(pub F);

impl<F: Future> __FutureOrCompletionFuture<F> {
    /// This method will be called when the type implements `Future`.
    pub fn __into_awaitable(self) -> F {
        self.0
    }
}

#[doc(hidden)]
pub trait __CompletionFutureIntoAwaitable {
    type Future;
    /// This method will be called when the type doesn't implement `Future`. As it's a trait it has
    /// lower priority than the inherent impl.
    fn __into_awaitable(self) -> Self::Future;
}
impl<F> __CompletionFutureIntoAwaitable for __FutureOrCompletionFuture<F> {
    type Future = Await<F>;
    fn __into_awaitable(self) -> Self::Future {
        Await { fut: self.0 }
    }
}

#[doc(hidden)]
pub mod __special_macros {
    pub use core::{
        assert, assert_eq, assert_ne, debug_assert, debug_assert_eq, debug_assert_ne, format_args,
        matches, panic, todo, unimplemented, unreachable, write, writeln,
    };

    pub use alloc::{format, vec};

    pub use std::{dbg, eprint, eprintln, print, println};
}
