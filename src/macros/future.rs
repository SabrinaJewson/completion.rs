use core::future::Future;

use completion_core::CompletionFuture;

use crate::{AssertCompletes, MustComplete};

/// An attribute macro to generate completion `async fn`s. These async functions evaluate to a
/// [`CompletionFuture`], and you can `.await` other [`CompletionFuture`]s inside of them.
///
/// Requires the `macro` feature.
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
/// Requires the `macro` feature.
///
/// # Examples
///
/// ```
/// use completion::{FutureExt, completion_async};
///
/// let fut = completion_async! {
///     let fut = async { 3 };
///     let completion_fut = async { 5 }.must_complete();
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
/// Requires the `macro` feature.
///
/// # Examples
///
/// ```
/// use completion::{FutureExt, completion_async_move};
///
/// let fut = completion_async_move! {
///     let fut = async { 3 };
///     let completion_fut = async { 5 }.must_complete();
///     fut.await + completion_fut.await
/// };
/// ```
#[macro_export]
macro_rules! completion_async_move {
    ($($tt:tt)*) => {
        $crate::__completion_async_move_inner!(($crate) $($tt)*)
    }
}

/// Wrapper around `MustComplete::new` to avoid displaying `MustComplete` in error messages.
#[doc(hidden)]
pub fn __make_completion_future<F: Future>(future: F) -> impl CompletionFuture<Output = F::Output> {
    MustComplete::new(future)
}

// We want to be able to `.await` both regular futures and completion futures in blocks generated
// by the macro. To do this, we use inherent method-based specialization.

#[doc(hidden)]
#[allow(missing_debug_implementations)]
pub struct __FutureOrCompletionFuture<F>(pub F);

impl<F: Future> __FutureOrCompletionFuture<F> {
    /// This method will be called when the type implements `Future`.
    pub fn __into_future_unsafe(self) -> F {
        self.0
    }
}

#[doc(hidden)]
pub trait __CompletionFutureIntoFutureUnsafe {
    type Future;
    /// This method will be called when the type doesn't implement `Future`. As it's a trait it has
    /// lower priority than the inherent impl.
    fn __into_future_unsafe(self) -> Self::Future;
}
impl<F> __CompletionFutureIntoFutureUnsafe for __FutureOrCompletionFuture<F> {
    type Future = AssertCompletes<F>;
    fn __into_future_unsafe(self) -> Self::Future {
        unsafe { AssertCompletes::new(self.0) }
    }
}

#[doc(hidden)]
pub mod __special_macros {
    pub use core::{
        assert, assert_eq, assert_ne, debug_assert, debug_assert_eq, debug_assert_ne, format_args,
        matches, panic, todo, unimplemented, unreachable, write, writeln,
    };

    #[cfg(feature = "alloc")]
    pub use alloc::{format, vec};

    #[cfg(feature = "std")]
    pub use std::{dbg, eprint, eprintln, print, println};
}

#[doc(hidden)]
pub mod __reexports {
    #[cfg(feature = "alloc")]
    pub use alloc::boxed::Box;
}
