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
/// use ::completion as my_completion;
/// use my_completion::completion;
///
/// #[completion(crate = "my_completion")]
/// async fn async_completion(x: &i32) -> i32 {
///     *x
/// }
/// ```
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
