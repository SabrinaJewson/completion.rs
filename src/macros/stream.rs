use alloc::boxed::Box;
use core::cell::Cell;
use core::fmt::{self, Debug, Formatter};
use core::future::Future;
use core::marker::PhantomPinned;
use core::pin::Pin;
use core::task::{Context, Poll};

use aliasable::boxed::AliasableBox;
#[cfg(doc)]
use completion_core::CompletionFuture;
use completion_core::CompletionStream;
use pin_project_lite::pin_project;

#[doc(hidden)]
pub use completion_macro::completion_stream_inner as __completion_stream_inner;

/// A bang macro to generate completion async streams.
///
/// These async streams evalute to a [`CompletionStream`], and you can `.await`
/// [`CompletionFuture`]s inside of them. You can return values using a `yield` expression. The `?`
/// operator works in the stream if it yields an [`Option`] or [`Result`] - if an error occurs the
/// stream will yield that single error and then exit.
///
/// Requires the `macro` and `alloc` features.
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
#[macro_export]
macro_rules! completion_stream {
    ($($tt:tt)*) => {
        $crate::__completion_stream_inner!(($crate) $($tt)*)
    }
}

/// A stable version of the try trait, for use with `?` in `completion_stream!`s.
#[doc(hidden)]
pub trait __Try {
    /// The type of this value on success.
    type Ok;
    /// The type of this value on failure.
    type Error;

    /// Applies the `?` operator.
    ///
    /// # Errors
    ///
    /// Fails if the try value is an error.
    fn into_result(self) -> Result<Self::Ok, Self::Error>;

    /// Wrap an error value.
    fn from_error(v: Self::Error) -> Self;

    /// Wrap an OK value.
    fn from_ok(v: Self::Ok) -> Self;
}

impl<T> __Try for Option<T> {
    type Ok = T;
    type Error = NoneError;

    fn into_result(self) -> Result<Self::Ok, Self::Error> {
        self.ok_or(NoneError)
    }
    fn from_error(_: Self::Error) -> Self {
        None
    }
    fn from_ok(v: Self::Ok) -> Self {
        Some(v)
    }
}

mod none_error {
    #[doc(hidden)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct NoneError;
}
use none_error::NoneError;

impl<T, E> __Try for Result<T, E> {
    type Ok = T;
    type Error = E;

    fn into_result(self) -> Result<<Self as __Try>::Ok, Self::Error> {
        self
    }
    fn from_error(v: Self::Error) -> Self {
        Err(v)
    }
    fn from_ok(v: <Self as __Try>::Ok) -> Self {
        Ok(v)
    }
}

pin_project! {
    /// An asynchronous stream backed by a future.
    #[doc(hidden)]
    pub struct __AsyncStream<T, F, Fut> {
        // The function used to create the generator.
        f: Option<F>,

        // The generator itself.
        #[pin]
        generator: Option<Fut>,

        // The last yielded item. The generator holds a pointer to this.
        yielded: AliasableBox<Cell<Option<T>>>,

        // We want to support unboxing `yielded` in the future.
        #[pin]
        _pinned: PhantomPinned,
    }
}

impl<T, F, Fut> __AsyncStream<T, F, Fut>
where
    F: FnOnce(Sender<T>) -> Fut,
    Fut: Future<Output = ()>,
{
    #[doc(hidden)]
    pub fn new(f: F) -> Self {
        Self {
            f: Some(f),
            generator: None,
            yielded: AliasableBox::from_unique(Box::new(Cell::new(None))),
            _pinned: PhantomPinned,
        }
    }
}

impl<T: Debug, F, Fut: Debug> Debug for __AsyncStream<T, F, Fut> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        struct F;
        impl Debug for F {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                f.write_str("<closure>")
            }
        }

        f.debug_struct("AsyncStream")
            .field("f", &self.f.as_ref().map(|_| F))
            .field("generator", &self.generator)
            .finish()
    }
}

impl<T, F, Fut> CompletionStream for __AsyncStream<T, F, Fut>
where
    F: FnOnce(Sender<T>) -> Fut,
    Fut: Future<Output = ()>,
{
    type Item = T;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        if this.generator.as_mut().as_pin_mut().is_none() {
            let sender = Sender {
                ptr: &**this.yielded,
            };
            this.generator
                .as_mut()
                .set(Some(this.f.take().unwrap()(sender)));
        }

        let res = this.generator.as_pin_mut().unwrap().poll(cx);

        match res {
            Poll::Ready(()) => Poll::Ready(None),
            Poll::Pending => this
                .yielded
                .take()
                // The future is not necessarily complete at this point and yet we are telling the
                // caller that we can be dropped.
                //
                // This is safe however because the macro asserts that no other non-droppable
                // completion futures can exist in the generator at yield points.
                .map_or(Poll::Pending, |val| Poll::Ready(Some(val))),
        }
    }
}

mod sender {
    use core::cell::Cell;

    pub struct Sender<T> {
        pub(super) ptr: *const Cell<Option<T>>,
    }
}
use sender::Sender;

impl<T> Sender<T> {
    pub fn send(&mut self, value: T) -> impl Future<Output = ()> {
        unsafe { &*self.ptr }.set(Some(value));

        SendFut { yielded: false }
    }
}

impl<T> Debug for Sender<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.pad("Sender")
    }
}

// Sender alone wouldn't be Send, however since we know it is only ever inside the generator if it
// is sent to another thread the __AsyncStream it is inside will be too.
unsafe impl<T> Send for Sender<T> {}

struct SendFut {
    yielded: bool,
}
impl Future for SendFut {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.yielded {
            Poll::Ready(())
        } else {
            self.yielded = true;
            Poll::Pending
        }
    }
}
