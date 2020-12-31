use core::cell::{Cell, UnsafeCell};
use core::fmt::{self, Debug, Formatter};
use core::future::Future;
use core::marker::PhantomPinned;
use core::mem::{self, MaybeUninit};
use core::pin::Pin;
use core::task::{Context, Poll};

#[cfg(doc)]
use completion_core::CompletionFuture;
use completion_core::CompletionStream;

#[doc(hidden)]
pub use completion_macro::completion_stream_inner as __completion_stream_inner;

/// A bang macro to generate completion async streams.
///
/// These async streams evalute to a [`CompletionStream`], and you can `.await`
/// [`CompletionFuture`]s inside of them. You can return values using a `yield` expression. The `?`
/// operator works in the stream if it yields an [`Option`] or [`Result`] - if an error occurs the
/// stream will yield that single error and then exit.
///
/// Requires the `macro` feature.
///
/// # Examples
///
/// ```
/// use completion_util::{completion_stream, CompletionStreamExt};
///
/// # completion_util::future::block_on(completion_util::completion_async! {
/// let stream = completion_stream! {
///     for i in 0..3 {
///         yield i;
///     }
/// };
///
/// futures_lite::pin!(stream);
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

/// An asynchronous stream backed by a future.
///
/// This implementation is stolen from Tokio's `async-stream` crate and adapted to work on
/// completion futures.
#[doc(hidden)]
pub struct __AsyncStream<T, F, Fut> {
    state: State<T, F, Fut>,
}

impl<T: Debug, F, Fut: Debug> Debug for __AsyncStream<T, F, Fut> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("__AsyncStream")
            .field("state", &self.state)
            .finish()
    }
}

/// The state of the async stream.
enum State<T, F, Fut> {
    /// The async stream has not yet been initialized. This holds the function that is used to
    /// initialize it.
    ///
    /// This state is necessary to allow the async stream to be soundly moved before `poll_next` is
    /// called for the first time.
    Uninit(F),

    /// The async stream has been initialized.
    ///
    /// This is an `UnsafeCell` to force the immutable borrow of its contents even when we have a
    /// mutable reference to it so that our mutable reference to it doesn't alias with the inner
    /// generator's reference to `Init::yielded`.
    Init(UnsafeCell<Init<T, Fut>>),
}

impl<T: Debug, F, Fut: Debug> Debug for State<T, F, Fut> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Uninit(_) => f.pad("Uninit"),
            Self::Init(init) => f
                .debug_tuple("Init")
                .field(unsafe { &*init.get() })
                .finish(),
        }
    }
}

/// An initialized async stream.
struct Init<T, Fut> {
    /// The last yielded item. The generator holds a pointer to this.
    yielded: Cell<Option<T>>,

    /// The generator itself. This is a `MaybeUninit` so that this type can be constructed with
    /// partial initialization - through all regular usage this is initialized. It is an
    /// `UnsafeCell` so we can get a mutable reference to it through the immutable reference
    /// provided by the outer `UnsafeCell`.
    generator: UnsafeCell<MaybeUninit<Fut>>,

    /// Whether the generator is done.
    done: bool,

    /// As the generator holds a pointer to `yielded`, this type cannot move in memory.
    _pinned: PhantomPinned,
}

impl<T: Debug, Fut: Debug> Debug for Init<T, Fut> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Init")
            .field(
                "generator",
                // Safety: We only create a shared reference to the data, and the only time a
                // mutable reference to this data is created is inside `poll_next` where we have a
                // `&mut Self` anyway.
                unsafe { &*(*self.generator.get()).as_ptr() },
            )
            .field("done", &self.done)
            .finish()
    }
}

impl<T, F, Fut> __AsyncStream<T, F, Fut>
where
    F: FnOnce(Sender<T>) -> Fut,
    Fut: Future<Output = ()>,
{
    #[doc(hidden)]
    pub fn new(f: F) -> __AsyncStream<T, F, Fut> {
        __AsyncStream {
            state: State::Uninit(f),
        }
    }
}

impl<T, F, Fut> CompletionStream for __AsyncStream<T, F, Fut>
where
    F: FnOnce(Sender<T>) -> Fut,
    Fut: Future<Output = ()>,
{
    type Item = T;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let me = Pin::into_inner_unchecked(self);

        let init_cell = match &mut me.state {
            State::Uninit(_) => {
                let old_state = mem::replace(
                    &mut me.state,
                    State::Init(UnsafeCell::new(Init {
                        yielded: Cell::new(None),
                        generator: UnsafeCell::new(MaybeUninit::uninit()),
                        done: false,
                        _pinned: PhantomPinned,
                    })),
                );
                let f = match old_state {
                    State::Uninit(f) => f,
                    State::Init(_) => unreachable!(),
                };
                let init_cell = match &mut me.state {
                    State::Init(init) => init,
                    State::Uninit(_) => unreachable!(),
                };
                let init = unsafe_cell_get_mut(init_cell);
                let sender = Sender {
                    ptr: &init.yielded as *const _,
                };
                init.generator = UnsafeCell::new(MaybeUninit::new(f(sender)));
                init_cell
            }
            State::Init(init) => {
                if unsafe_cell_get_mut(init).done {
                    return Poll::Ready(None);
                }
                init
            }
        };

        // Immutably borrow `init`. If we mutably borrowed `init` here it would cause UB as this
        // mutable reference to `init.yielded` would alias with the generator's.
        let init = &*init_cell.get();

        let generator = &mut *(*init.generator.get()).as_mut_ptr();

        // Miri sometimes does not like this because of
        // <https://github.com/rust-lang/rust/issues/63818>. However this is acceptable because the
        // same unsoundness can be triggered with safe code:
        // https://play.rust-lang.org/?version=stable&mode=debug&edition=2018&gist=43b4a4f3d7e9287c73821b27084fb179
        // And so we know that once the safe code stops being UB (which will happen), this code will
        // also stop being UB.
        let res = Pin::new_unchecked(generator).poll(cx);

        // Now that the generator no longer will use its pointer to `init.yielded`, we can create a
        // mutable reference.
        let init = unsafe_cell_get_mut(init_cell);

        if res.is_ready() {
            init.done = true;
        }

        if let Some(yielded) = init.yielded.take() {
            // The future is not necessarily complete at this point and yet we are telling the
            // caller that we can be dropped.
            //
            // This is safe however because the macro asserts that no other non-droppable
            // completion futures can exist in the generator at yield points.
            return Poll::Ready(Some(yielded));
        }

        match res {
            Poll::Ready(()) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (
            0,
            match &self.state {
                State::Init(init) if unsafe { &*init.get() }.done => Some(0),
                _ => None,
            },
        )
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

/// A helper function to reduce usages of `unsafe`.
fn unsafe_cell_get_mut<T: ?Sized>(cell: &mut UnsafeCell<T>) -> &mut T {
    unsafe { &mut *cell.get() }
}
