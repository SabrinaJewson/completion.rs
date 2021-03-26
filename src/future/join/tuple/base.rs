use core::convert::Infallible;
use core::fmt::{self, Debug, Formatter};
use core::marker::PhantomData;
use core::mem;
use core::pin::Pin;
use core::ptr;
use core::sync::atomic::{self, AtomicBool};
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use std::panic::resume_unwind;

use atomic_waker::AtomicWaker;
use completion_core::CompletionFuture;
use concurrent_queue::ConcurrentQueue;
use pin_project_lite::pin_project;

use super::super::{ControlFlow, ControlFlowFuture, FutureState, Panic};

/// This is to ensure we create the correct `Arc` when dealing with type-erased pointers.
type ArcShared<T> = std::sync::Arc<Shared<T>>;

pin_project! {
    /// The generic implementation behind all the tuple joining futures.
    #[derive(Debug)]
    pub(super) struct Join<T: JoinTuple> {
        // The futures themselves. Each one stores its own state to avoid polling it after it has
        // completed.
        #[pin]
        futures: T::Futures,
        // This `Join`'s state.
        state: State<T>,
        // Whether we have polled all the futures once in this state.
        polled_once: bool,
        // The number of futures that are done in the current state.
        done: u8,
        // Shared state between this and the wakers.
        shared: ArcShared<T>,
        _correct_debug_bounds: PhantomData<T::Break>,
    }
}

/// The state of a `Join`.
#[derive(Debug)]
enum State<T: JoinTuple> {
    /// We are currently running all the futures.
    Running,
    /// A future has decided to break, we are cancelling the futures.
    Broken(T::Break),
    /// The user is calling `poll_cancel`.
    Cancelling,
    /// A future has panicked. We are cancelling the futures, and then will propagate the panic.
    Panicked(Panic),
    /// This future is complete.
    Done,
}

/// Shared state between the `Join` itself and all the wakers. This is placed in an `Arc`.
#[derive(Debug)]
struct Shared<T: JoinTuple> {
    /// The queue of futures that need to be polled.
    ///
    /// TODO: Switch to a local implementation, so that we can take advantage of:
    /// - It being MPSC not MPMC.
    /// - Knowing its size at compile time, so it can be stored locally instead of on the heap.
    /// - Its size being small.
    /// - Each `u8` not using many of the upper bits.
    /// - `u8`s being `Copy` and potentially atomic.
    /// - It not being able to overflow.
    to_poll: ConcurrentQueue<u8>,
    /// The waker passed to `Join`. Should be woken after pushing to the poll queue.
    waker: AtomicWaker,
    /// The state of each of the futures' wakers. This is an array of `WakerState`s.
    waker_states: T::WakerStates,
}

/// The state of a waker passed to a future inside a `Join`.
pub struct WakerState<T: JoinTuple> {
    /// Whether we have already notified the `Join` that we need to be polled. This avoids
    /// overflowing the poll queue.
    notified: AtomicBool,
    /// The pointer to the base of the allocation. Using the offset of the waker's pointer to the
    /// start of the list of waker states in the `Shared`, each waker can calculate its index.
    base: *const Shared<T>,
}
unsafe impl<T: JoinTuple> Send for WakerState<T> {}
unsafe impl<T: JoinTuple> Sync for WakerState<T> {}

impl<T: JoinTuple> WakerState<T> {
    unsafe fn drop_waker(state: *const ()) {
        ArcShared::decrement_strong_count((*state.cast::<Self>()).base);
    }

    const WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(
        |ptr| unsafe { &*ptr.cast::<Self>() }.waker(),
        |ptr| unsafe {
            (*ptr.cast::<Self>()).wake();
            Self::drop_waker(ptr);
        },
        |ptr| unsafe { &*ptr.cast::<Self>() }.wake(),
        Self::drop_waker,
    );

    fn waker(&self) -> RawWaker {
        unsafe { ArcShared::increment_strong_count(self.base) };
        RawWaker::new((self as *const Self).cast(), &Self::WAKER_VTABLE)
    }

    fn wake(&self) {
        if !self.notified.swap(true, atomic::Ordering::SeqCst) {
            let shared = unsafe { &*self.base };
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let index = unsafe {
                (self as *const Self)
                    .offset_from((shared.waker_states.as_ref() as *const [Self]).cast::<Self>())
            } as u8;
            shared.to_poll.push(index).unwrap();
            shared.waker.wake();
        }
    }
}

impl<T: JoinTuple> Debug for WakerState<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("WakerState")
            .field("notified", &self.notified)
            .finish()
    }
}

impl<T: JoinTuple> Join<T> {
    pub(super) fn new(tuple: T) -> Self {
        let mut shared = ArcShared::new(Shared {
            to_poll: ConcurrentQueue::bounded(usize::from(T::LEN)),
            waker: AtomicWaker::new(),
            waker_states: T::new_waker_states(),
        });
        let ptr = ArcShared::as_ptr(&shared);
        let waker_states: &mut T::WakerStates =
            &mut ArcShared::get_mut(&mut shared).unwrap().waker_states;
        for waker_state in waker_states.as_mut() {
            waker_state.base = ptr;
        }

        Self {
            futures: tuple.into_futures(),
            state: State::Running,
            polled_once: false,
            done: 0,
            shared,
            _correct_debug_bounds: PhantomData,
        }
    }

    /// Poll the futures with the given function.
    fn poll_with<B, F>(mut self: Pin<&mut Self>, mut f: F) -> ControlFlow<B, Poll<State<T>>>
    where
        F: FnMut(Pin<&mut T::Futures>, &mut Context<'_>, u8) -> Poll<ControlFlow<B>>,
    {
        let mut this = self.as_mut().project();

        if !*this.polled_once {
            *this.done = 0;
        }

        let mut full_range = 0..T::LEN;

        loop {
            let i = if *this.polled_once {
                this.shared.to_poll.pop().ok()
            } else {
                full_range.next()
            };
            let i = match i {
                Some(i) => i,
                None => break,
            };

            let waker_state = &this.shared.waker_states.as_ref()[i as usize];

            if *this.polled_once {
                waker_state.notified.store(false, atomic::Ordering::SeqCst);
            }

            let waker = unsafe { Waker::from_raw(waker_state.waker()) };
            let mut cx = Context::from_waker(&waker);

            match f(this.futures.as_mut(), &mut cx, i) {
                Poll::Ready(ControlFlow::Continue(())) => *this.done += 1,
                Poll::Ready(ControlFlow::Break(b)) => {
                    return ControlFlow::Break(b);
                }
                Poll::Pending => {}
            }
        }

        *this.polled_once = true;

        ControlFlow::Continue(if *this.done == T::LEN {
            Poll::Ready(mem::replace(this.state, State::Done))
        } else {
            Poll::Pending
        })
    }

    fn poll_panicked(self: Pin<&mut Self>) {
        match self.poll_with(T::poll_panicked) {
            ControlFlow::Continue(Poll::Ready(state)) => resume_unwind(match state {
                State::Panicked(payload) => payload.into_inner(),
                _ => panic!("Polled `Join` after completion"),
            }),
            ControlFlow::Continue(Poll::Pending) => {}
            ControlFlow::Break(infallible) => match infallible {},
        }
    }
}

impl<T: JoinTuple> CompletionFuture for Join<T> {
    type Output = ControlFlow<T::Break, T::Output>;

    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().project();
        this.shared.waker.register(cx.waker());

        if let State::Running = this.state {
            match self.as_mut().poll_with(T::poll_one) {
                ControlFlow::Continue(poll) => {
                    return poll
                        .map(|_| ControlFlow::Continue(T::take_output(self.project().futures)))
                }
                ControlFlow::Break(Ok(val)) => {
                    let this = self.as_mut().project();
                    *this.polled_once = false;
                    *this.state = State::Broken(val);
                }
                ControlFlow::Break(Err(panic)) => {
                    let this = self.as_mut().project();
                    *this.polled_once = false;
                    *this.state = State::Panicked(panic);
                }
            }
        }

        if let State::Broken(_) = self.as_mut().project().state {
            match self.as_mut().poll_with(T::poll_cancel) {
                ControlFlow::Continue(poll) => {
                    return poll.map(|state| {
                        ControlFlow::Break(match state {
                            State::Broken(val) => val,
                            _ => unreachable!(),
                        })
                    });
                }
                ControlFlow::Break(panic) => {
                    *self.as_mut().project().state = State::Panicked(panic);
                }
            }
        }

        self.poll_panicked();
        Poll::Pending
    }
    unsafe fn poll_cancel(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.as_mut().project();
        this.shared.waker.register(cx.waker());

        if let State::Running | State::Broken(_) = this.state {
            *this.state = State::Cancelling;
            *this.polled_once = false;
        }

        if let State::Cancelling = this.state {
            match self.as_mut().poll_with(T::poll_cancel) {
                ControlFlow::Continue(poll) => return poll.map(|_| ()),
                ControlFlow::Break(panic) => {
                    *self.as_mut().project().state = State::Panicked(panic);
                }
            }
        }

        self.poll_panicked();
        Poll::Pending
    }
}

/// A tuple of futures that can be used in `Join`.
pub trait JoinTuple: Sized {
    /// The tuple of futures stored in `FutureState`s.
    type Futures;
    /// Convert this to its futures.
    fn into_futures(self) -> Self::Futures;

    /// The number of futures in the tuple.
    const LEN: u8;

    /// Array of `WakerState<Self>`s, one for each future.
    type WakerStates: AsRef<[WakerState<Self>]> + AsMut<[WakerState<Self>]> + Send + Sync + Debug;
    /// Create the waker states.
    fn new_waker_states() -> Self::WakerStates;

    /// The tuple of results outputted when all the futures complete.
    type Output;
    /// The single result outputted when one future decides to break control flow.
    type Break;

    /// Poll the future with index `i` in the tuple.
    fn poll_one(
        futures: Pin<&mut Self::Futures>,
        cx: &mut Context<'_>,
        i: u8,
    ) -> Poll<ControlFlow<Result<Self::Break, Panic>>>;

    /// Attempt to cancel the future with index `i` in the tuple.
    fn poll_cancel(
        futures: Pin<&mut Self::Futures>,
        cx: &mut Context<'_>,
        i: u8,
    ) -> Poll<ControlFlow<Panic>>;

    /// Attempt to cancel the future with index `i` in the tuple, ignoring any panics.
    fn poll_panicked(
        futures: Pin<&mut Self::Futures>,
        cx: &mut Context<'_>,
        i: u8,
    ) -> Poll<ControlFlow<Infallible>>;

    /// Take the output of all the futures.
    ///
    /// # Panics
    ///
    /// Panics if not all the futures are complete.
    fn take_output(futures: Pin<&mut Self::Futures>) -> Self::Output;
}

macro_rules! with { ($_:tt, $($tt:tt)*) => { $($tt)* } }

/// `macro_rules!` implementation of `count_tts`.
/// Source: <https://github.com/camsteffen/count-tts>
macro_rules! count_tts {
    () => (0);
    ($one:tt) => (1);
    ($($a:tt $b:tt)+) => (count_tts!($($a)+) << 1);
    ($odd:tt $($a:tt $b:tt)+) => (count_tts!($($a)+) << 1 | 1);
}

macro_rules! impl_tuple {
    ($($param:ident),*) => { const _: () = {
        fn project_tuple<$($param,)*>(
            tuple: Pin<&mut ($($param,)*)>
        ) -> ($(Pin<&mut $param>,)*) {
            let ($($param,)*) = unsafe { Pin::into_inner_unchecked(tuple) };
            ($(unsafe { Pin::new_unchecked($param) },)*)
        }

        impl<Break, $($param,)*> JoinTuple for ($($param,)*)
        where
            $($param: ControlFlowFuture<Break = Break>,)*
        {
            type Futures = ($(FutureState<$param>,)*);
            fn into_futures(self) -> Self::Futures {
                let ($($param,)*) = self;
                ($(FutureState::Running($param),)*)
            }

            const LEN: u8 = count_tts!($($param)*);

            type WakerStates = [WakerState<Self>; count_tts!($($param)*)];
            fn new_waker_states() -> Self::WakerStates {
                [$(with!($param, WakerState {
                    notified: AtomicBool::new(false),
                    base: ptr::null(),
                }),)*]
            }

            type Output = ($(<$param as ControlFlowFuture>::Continue,)*);
            type Break = Break;

            fn poll_one(
                futures: Pin<&mut Self::Futures>,
                cx: &mut Context<'_>,
                i: u8,
            ) -> Poll<ControlFlow<Result<Self::Break, Panic>>> {
                let ($($param,)*) = project_tuple(futures);
                let mut pos = 0;
                $(
                    if pos == i {
                        return unsafe { $param.poll(cx) };
                    }
                    #[allow(unused_assignments)]
                    {pos += 1}
                )*
                unreachable!("Called `poll_one` on tuple with out of range index")
            }

            fn poll_cancel(
                futures: Pin<&mut Self::Futures>,
                cx: &mut Context<'_>,
                i: u8,
            ) -> Poll<ControlFlow<Panic>> {
                let ($($param,)*) = project_tuple(futures);
                let mut pos = 0;
                $(
                    if pos == i {
                        return unsafe { $param.poll_cancel(cx) };
                    }
                    #[allow(unused_assignments)]
                    {pos += 1}
                )*
                unreachable!("Called `poll_cancel` on tuple with out of range index")
            }

            fn poll_panicked(
                futures: Pin<&mut Self::Futures>,
                cx: &mut Context<'_>,
                i: u8,
            ) -> Poll<ControlFlow<Infallible>> {
                let ($($param,)*) = project_tuple(futures);
                let mut pos = 0;
                $(
                    if pos == i {
                        return unsafe { $param.poll_panicked(cx) };
                    }
                    #[allow(unused_assignments)]
                    {pos += 1}
                )*
                unreachable!("Called `poll_panicked` on tuple with out of range index")
            }

            fn take_output(futures: Pin<&mut Self::Futures>) -> Self::Output {
                let ($($param,)*) = project_tuple(futures);
                ($($param.take_output().unwrap(),)*)
            }
        }
    };}
}
apply_on_tuples!(impl_tuple!);

#[cfg(test)]
mod tests {
    use super::*;

    use std::future::ready;
    use std::panic::{catch_unwind, panic_any, AssertUnwindSafe};
    use std::time::Duration;

    use crate::future::{block_on, CompletionFutureExt, FutureExt};
    use crate::test_utils::{sleep, CompletionFutureExt as _, Yield};

    #[test]
    fn ready_continue() {
        assert_eq!(
            block_on::<ControlFlow<Infallible, _>, _>(Join::new((
                ready(ControlFlow::Continue(Box::new(0)))
                    .check()
                    .max_cancels(0),
                ready(ControlFlow::Continue(Box::new(1)))
                    .check()
                    .max_cancels(0),
                ready(ControlFlow::Continue(vec![2, 3]))
                    .check()
                    .max_cancels(0),
            ))),
            ControlFlow::Continue((Box::new(0), Box::new(1), vec![2, 3])),
        );
    }

    #[test]
    fn ready_break() {
        assert_eq!(
            block_on(Join::new((
                ready(ControlFlow::Continue(Box::new([0]))).check(),
                ready(ControlFlow::Continue(vec![1, 2, 3])).check(),
                ready(<ControlFlow<_, ()>>::Break(Box::new(4_i64)))
                    .check()
                    .max_cancels(0),
                ready(<ControlFlow<_, ()>>::Break(Box::new(5)))
                    .check()
                    .max_polls(0),
                ready(ControlFlow::Continue(vec![6, 7, 8]))
                    .check()
                    .max_polls(0),
            ))),
            ControlFlow::Break(Box::new(4_i64)),
        );
    }

    #[test]
    fn pending_continue() {
        assert_eq!(
            block_on::<ControlFlow<Infallible, _>, _>(Join::new((
                ready(ControlFlow::Continue(Box::new(0)))
                    .check()
                    .max_cancels(0),
                Yield::once(ready(ControlFlow::Continue(Box::new(1))))
                    .check()
                    .max_cancels(0),
                async {
                    sleep(Duration::from_millis(10)).await;
                    ControlFlow::Continue(Box::new(2))
                }
                .into_completion()
                .check()
                .max_polls(2)
                .max_cancels(0),
                async {
                    sleep(Duration::from_millis(5)).await;
                    sleep(Duration::from_millis(2)).await;
                    sleep(Duration::from_millis(17)).await;
                    ControlFlow::Continue(Box::new(3))
                }
                .into_completion()
                .check()
                .max_polls(4)
                .max_cancels(0),
            ))),
            ControlFlow::Continue((Box::new(0), Box::new(1), Box::new(2), Box::new(3))),
        );
    }

    #[test]
    fn all_pending_continue() {
        assert_eq!(
            block_on::<ControlFlow<Infallible, _>, _>(Join::new((
                async {
                    sleep(Duration::from_millis(10)).await;
                    ControlFlow::Continue(Box::new(0))
                }
                .into_completion()
                .check()
                .max_polls(2),
                async {
                    sleep(Duration::from_millis(9)).await;
                    sleep(Duration::from_millis(2)).await;
                    ControlFlow::Continue(Box::new(1))
                }
                .into_completion()
                .check()
                .max_polls(3),
                async {
                    sleep(Duration::from_millis(11)).await;
                    ControlFlow::Continue(Box::new(2))
                }
                .into_completion()
                .check()
                .max_polls(2),
            ))),
            ControlFlow::Continue((Box::new(0), Box::new(1), Box::new(2))),
        );
    }

    #[test]
    fn pending_break() {
        let mut x = false;
        assert_eq!(
            block_on(Join::new((
                async {
                    sleep(Duration::from_millis(2)).await;
                    x = true;
                    ControlFlow::Continue(())
                }
                .into_completion()
                .must_complete()
                .check()
                .max_polls(1),
                async {
                    Yield::once(ready(())).await;
                    <ControlFlow<_, Infallible>>::Break(Box::new(6_i64))
                }
                .into_completion()
                .check()
                .max_cancels(0),
            ))),
            ControlFlow::Break(Box::new(6_i64)),
        );
        assert!(x);
    }

    #[test]
    fn panic() {
        let mut x = false;
        let res: Result<ControlFlow<Infallible, ((), Infallible, Infallible)>, _> =
            catch_unwind(AssertUnwindSafe(|| {
                block_on(Join::new((
                    async {
                        Yield::once(ready(())).await;
                        x = true;
                        ControlFlow::Continue(())
                    }
                    .into_completion()
                    .must_complete()
                    .check()
                    .max_polls(1),
                    async { panic_any(0) }.into_completion().check(),
                    async { panic_any(1) }.into_completion().check(),
                )))
            }));

        assert_eq!(*res.unwrap_err().downcast::<i32>().unwrap(), 0);
        assert!(x);
    }

    #[test]
    fn panic_from_broken() {
        let mut x = false;
        let res: Result<ControlFlow<_, (Infallible, (), Infallible)>, _> =
            catch_unwind(AssertUnwindSafe(|| {
                block_on(Join::new((
                    ready(ControlFlow::Break(())).check(),
                    async {
                        Yield::once(ready(())).await;
                        x = true;
                        ControlFlow::Continue(())
                    }
                    .into_completion()
                    .must_complete()
                    .check(),
                    async { panic_any(0) }
                        .into_completion()
                        .must_complete()
                        .check(),
                )))
            }));

        assert_eq!(*res.unwrap_err().downcast::<i32>().unwrap(), 0);
        assert!(x);
    }

    #[test]
    fn nested() {
        assert_eq!(
            block_on::<ControlFlow<Infallible, _>, _>(Join::new((
                Join::new((
                    Join::new((
                        ready(ControlFlow::Continue(Box::new(0))).check(),
                        async {
                            sleep(Duration::from_millis(2)).await;
                            sleep(Duration::from_millis(4)).await;
                            ControlFlow::Continue(Box::new(1))
                        }
                        .into_completion()
                        .check()
                    ))
                    .check(),
                    ready(ControlFlow::Continue(Box::new(2))).check(),
                ))
                .check(),
                Join::new((
                    async {
                        Yield::once(ready(())).await;
                        ControlFlow::Continue(Box::new(3))
                    }
                    .into_completion()
                    .check(),
                    Join::new((
                        ready(ControlFlow::Continue(Box::new(4))).check(),
                        ready(ControlFlow::Continue(Box::new(5))).check(),
                    ))
                    .check(),
                ))
                .check(),
            ))),
            ControlFlow::Continue((
                ((Box::new(0), Box::new(1)), Box::new(2)),
                (Box::new(3), (Box::new(4), Box::new(5)))
            )),
        );
    }
}
