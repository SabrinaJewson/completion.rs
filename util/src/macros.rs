use core::future::Future;

#[cfg(doc)]
use completion_core::CompletionFuture;

use crate::AssertCompletes;

/// An attribute macro to generate completion `async fn`s. These async functions evaluate to a
/// [`CompletionFuture`], and you can `.await` other [`CompletionFuture`]s inside of them.
///
/// Requires the `macro` feature.
///
/// # Examples
///
/// ```
/// use completion_util::{FutureExt, completion};
///
/// #[completion]
/// async fn async_completion() -> i32 {
///     let fut = async { 3 };
///     let completion_fut = async { 5 }.must_complete();
///     fut.await + completion_fut.await
/// }
/// ```
///
/// This macro needs to know a path to `::completion_util` in order to work. You can change it
/// using the `crate` option:
///
/// ```
/// use completion_util as my_completion_util;
/// use my_completion_util::completion;
///
/// #[completion(crate = "my_completion_util")]
/// async fn async_completion(x: &i32) -> i32 {
///     *x
/// }
/// ```
pub use completion_macro::completion;

#[doc(hidden)]
pub use completion_macro::completion_async_inner as __completion_async_inner;
#[doc(hidden)]
pub use completion_macro::completion_async_move_inner as __completion_async_move_inner;

/// A bang macro to generate completion `async` blocks. These async blocks evaluate to a
/// [`CompletionFuture`], and you can `.await` other [`CompletionFuture`]s inside of them.
///
/// Requires the `macro` feature.
///
/// # Examples
///
/// ```
/// use completion_util::{FutureExt, completion_async };
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
/// A bang macro to generate completion `async move` blocks. These async blocks evaluate to a
/// [`CompletionFuture`], and you can `.await` other [`CompletionFuture`]s inside of them.
///
/// Requires the `macro` feature.
///
/// # Examples
///
/// ```
/// use completion_util::{FutureExt, completion_async_move};
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

// Re-exported special-cased macros for the macros to use.
#[doc(hidden)]
pub mod __special_macros {
    #[cfg(feature = "alloc")]
    pub use alloc::{format, vec};
    #[doc(hidden)]
    pub use core::{
        assert, assert_eq, assert_ne, debug_assert, debug_assert_eq, debug_assert_ne, format_args,
        matches, panic, todo, unimplemented, unreachable, write, writeln,
    };
    #[cfg(feature = "std")]
    pub use std::{dbg, eprint, eprintln, print, println};
}

// We want to be able to `.await` both regular futures and completion futures in blocks generated
// by the macro. To do this, we use inherent method-based specialization.

#[doc(hidden)]
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
