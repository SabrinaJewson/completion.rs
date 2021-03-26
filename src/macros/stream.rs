use core::cell::Cell;
use core::future::Future;
use core::marker::PhantomData;
use core::pin::Pin;
use core::ptr;
use core::task::{Context, Poll};

use completion_core::{CompletionFuture, CompletionStream};
use pin_project_lite::pin_project;

#[doc(hidden)]
pub use completion_macro::completion_stream_inner as __completion_stream_inner;

#[cfg(test)]
mod tests;

/// A bang macro to generate completion async streams.
///
/// These async streams evaluate to a [`CompletionStream`], and you can `.await`
/// [`CompletionFuture`]s inside of them. You can return values using a `yield` expression. The `?`
/// operator works in the stream if it yields an [`Option`] or [`Result`] - if an error occurs the
/// stream will yield that single error and then exit.
///
/// # Examples
///
/// ```
/// use completion::{completion_stream, CompletionStreamExt};
///
/// # completion::future::block_on(completion::completion_async! {
/// let stream = completion_stream! {
///     for i in 0..3 {
///         yield i;
///     }
/// };
///
/// # use futures_lite::pin;
/// pin!(stream);
///
/// assert_eq!(stream.next().await, Some(0));
/// assert_eq!(stream.next().await, Some(1));
/// assert_eq!(stream.next().await, Some(2));
/// assert_eq!(stream.next().await, None);
/// # });
/// ```
#[cfg_attr(doc_cfg, doc(cfg(all(feature = "macro", feature = "std"))))]
#[macro_export]
macro_rules! completion_stream {
    ($($tt:tt)*) => {
        $crate::__completion_stream_inner!(($crate) $($tt)*)
    }
}

#[doc(hidden)]
pub fn __completion_stream<T, F>(
    generator: F,
    item: PhantomData<T>,
) -> impl CompletionStream<Item = T>
where
    F: CompletionFuture<Output = ()>,
{
    Wrapper {
        generator,
        _item: item,
    }
}

thread_local! {
    static YIELDED_VALUE: Cell<*mut ()> = Cell::new(ptr::null_mut());
}

pin_project! {
    struct Wrapper<T, F> {
        #[pin]
        generator: F,
        _item: PhantomData<T>,
    }
}

impl<T, F> CompletionStream for Wrapper<T, F>
where
    F: CompletionFuture<Output = ()>,
{
    type Item = T;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        struct ResetYieldedValueGuard(*mut ());
        impl Drop for ResetYieldedValueGuard {
            fn drop(&mut self) {
                YIELDED_VALUE.with(|cell| cell.set(self.0));
            }
        }

        let this = self.project();

        let mut yielded = None;
        let yielded_ptr: *mut () = (&mut yielded as *mut Option<T>).cast();

        let guard = ResetYieldedValueGuard(YIELDED_VALUE.with(|cell| cell.replace(yielded_ptr)));
        let res = this.generator.poll(cx);
        drop(guard);

        match (yielded, res) {
            (Some(yielded), Poll::Pending) => Poll::Ready(Some(yielded)),
            (None, Poll::Ready(())) => Poll::Ready(None),
            (None, Poll::Pending) => Poll::Pending,
            _ => unreachable!(),
        }
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.project().generator.poll_cancel(cx)
    }
}

#[doc(hidden)]
pub fn __yield_value<T>(_item: PhantomData<T>, value: T) -> impl Future<Output = ()> {
    let ptr = YIELDED_VALUE.with(Cell::get).cast::<Option<T>>();
    debug_assert!(!ptr.is_null());

    *unsafe { &mut *ptr } = Some(value);

    YieldFut(true)
}

struct YieldFut(bool);
impl Future for YieldFut {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.0 {
            self.0 = false;
            // We don't need to wake the waker as the outer stream is going to return Poll::Ready.
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }
}
