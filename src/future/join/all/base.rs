use core::fmt::{self, Debug, Formatter};
use core::iter::FusedIterator;
use core::marker::PhantomData;
use core::mem;
use core::pin::Pin;
use core::ptr;
use core::sync::atomic::{self, AtomicBool};
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use std::panic::resume_unwind;
use std::vec;

use atomic_waker::AtomicWaker;
use completion_core::CompletionFuture;
use concurrent_queue::ConcurrentQueue;

use crate::stream::{FromCompletionStream, FromCompletionStreamInner};

use super::super::{ControlFlow, ControlFlowFuture, FutureState, Panic};

/// This is to ensure we create the correct `Arc` when dealing with type-erased pointers.
type ArcShared = std::sync::Arc<Shared>;

/// The generic implementation behind all the `_all` futures.
#[derive(Debug)]
pub(super) struct JoinAll<F: ControlFlowFuture> {
    // The futures themselves.
    futures: Pin<Vec<FutureState<F>>>,
    // This `JoinAll`'s state.
    state: State<F>,
    // Whether we have polled all the futures once in this state.
    polled_once: bool,
    // The number of futures that are done in the current state.
    done: usize,
    // Shared state between this and the wakers.
    shared: ArcShared,
    _correct_debug_bounds: PhantomData<(F::Continue, F::Break)>,
}
impl<F: ControlFlowFuture> Unpin for JoinAll<F> {}

/// The state of a `JoinAll`.
#[derive(Debug)]
enum State<F: ControlFlowFuture> {
    /// We are currently running all the futures.
    Running,
    /// A future has decided to break, we are cancelling the futures.
    Broken(F::Break),
    /// The user is calling `poll_cancel`.
    Cancelling,
    /// A future has panicked. We are cancelling the futures, and then will propagate the panic.
    Panicked(Panic),
    /// This future is complete.
    Done,
}

/// Shared state between the `JoinAll` itself and all the wakers. This is placed in an `Arc`.
#[derive(Debug)]
struct Shared {
    /// The queue of futures that need to be polled.
    ///
    /// TODO: Switch to a local implementation, so that we can take advantage of:
    /// - It being MPSC not MPMC.
    /// - Knowing its size at construction time, so it can be stored locally.
    /// - `usize` being `Copy` and potentially atomic.
    /// - It not being able to overflow.
    to_poll: ConcurrentQueue<usize>,
    /// The waker passed to `JoinAll`. Should be woken after pushing to the poll queue.
    waker: AtomicWaker,
    /// The state of each of the futures' wakers.
    ///
    /// TODO: Store this locally instead of on the heap.
    waker_states: Box<[WakerState]>,
}

/// The state of a waker passed to a future inside a `JoinAll`.
#[derive(Debug)]
pub struct WakerState {
    /// Whether we have already notified the `JoinAll` that we need to be polled. This avoids
    /// overflowing the poll queue.
    notified: AtomicBool,
    /// The pointer to the base of the allocation. Using the offset of the waker's pointer to the
    /// start of the list of waker states in the `Shared`, each waker can calculate its index.
    base: *const Shared,
}
unsafe impl Send for WakerState {}
unsafe impl Sync for WakerState {}

impl WakerState {
    fn waker(&self) -> RawWaker {
        unsafe fn drop_waker(state: *const ()) {
            let base_ptr = (*(state as *const WakerState)).base;
            ArcShared::from_raw(base_ptr);
        }

        const VTABLE: RawWakerVTable = RawWakerVTable::new(
            |ptr| unsafe { &*(ptr as *const WakerState) }.waker(),
            |ptr| unsafe {
                (*(ptr as *const WakerState)).wake();
                drop_waker(ptr);
            },
            |ptr| unsafe { &*(ptr as *const WakerState) }.wake(),
            drop_waker,
        );

        let arc = unsafe { ArcShared::from_raw(self.base) };
        mem::forget(ArcShared::clone(&arc));
        mem::forget(arc);
        RawWaker::new(self as *const _ as *const (), &VTABLE)
    }

    fn wake(&self) {
        if !self.notified.swap(true, atomic::Ordering::SeqCst) {
            let shared = unsafe { &*self.base };
            #[allow(clippy::cast_sign_loss)]
            let index = unsafe {
                (self as *const Self)
                    .offset_from(&*shared.waker_states as *const [Self] as *const Self)
            } as usize;

            shared.to_poll.push(index).unwrap();
            shared.waker.wake();
        }
    }
}

impl<F: ControlFlowFuture> JoinAll<F> {
    pub(super) fn new(futures: impl IntoIterator<Item = F>) -> Self {
        Self::new_inner(futures.into_iter().map(FutureState::Running).collect())
    }
    fn new_inner(futures: Vec<FutureState<F>>) -> Self {
        let mut shared = ArcShared::new(Shared {
            to_poll: ConcurrentQueue::bounded(futures.len()),
            waker: AtomicWaker::new(),
            waker_states: (0..futures.len())
                .map(|_| WakerState {
                    notified: AtomicBool::new(false),
                    base: ptr::null(),
                })
                .collect(),
        });
        let ptr = ArcShared::as_ptr(&shared);
        for waker_state in &mut *ArcShared::get_mut(&mut shared).unwrap().waker_states {
            waker_state.base = ptr;
        }

        Self {
            futures: pin_vec(futures),
            state: State::Running,
            polled_once: false,
            done: 0,
            shared,
            _correct_debug_bounds: PhantomData,
        }
    }
}

impl<F: ControlFlowFuture> FromCompletionStream<F> for JoinAll<F> {}
impl<F: ControlFlowFuture> FromCompletionStreamInner<F> for JoinAll<F> {
    type Intermediate = Vec<FutureState<F>>;
    fn start(lower: usize, _upper: Option<usize>) -> Self::Intermediate {
        Vec::with_capacity(lower)
    }
    fn push(mut intermediate: Self::Intermediate, item: F) -> Result<Self::Intermediate, Self> {
        intermediate.push(FutureState::Running(item));
        Ok(intermediate)
    }
    fn finalize(intermediate: Self::Intermediate) -> Self {
        JoinAll::new_inner(intermediate)
    }
}

type FutureStatePollFn<F, B> =
    unsafe fn(Pin<&mut FutureState<F>>, &mut Context<'_>) -> Poll<ControlFlow<B>>;

impl<F: ControlFlowFuture> JoinAll<F> {
    /// Poll the futures with the given function.
    fn poll_with<B>(&mut self, f: FutureStatePollFn<F, B>) -> ControlFlow<B, Poll<State<F>>> {
        if !self.polled_once {
            self.done = 0;
        }

        let mut full_range = 0..self.futures.len();

        loop {
            let i = if self.polled_once {
                self.shared.to_poll.pop().ok()
            } else {
                full_range.next()
            };
            let i = match i {
                Some(i) => i,
                None => break,
            };

            let waker_state = &self.shared.waker_states.as_ref()[i];

            if self.polled_once {
                waker_state.notified.store(false, atomic::Ordering::SeqCst);
            }

            let waker = unsafe { Waker::from_raw(waker_state.waker()) };
            let mut cx = Context::from_waker(&waker);

            match unsafe { f(slice_index_pin_mut(self.futures.as_mut(), i), &mut cx) } {
                Poll::Ready(ControlFlow::Continue(())) => self.done += 1,
                Poll::Ready(ControlFlow::Break(b)) => {
                    return ControlFlow::Break(b);
                }
                Poll::Pending => {}
            }
        }

        self.polled_once = true;

        ControlFlow::Continue(if self.done == self.futures.len() {
            Poll::Ready(mem::replace(&mut self.state, State::Done))
        } else {
            Poll::Pending
        })
    }

    fn poll_panicked(&mut self) {
        match self.poll_with(FutureState::poll_panicked) {
            ControlFlow::Continue(Poll::Ready(state)) => resume_unwind(match state {
                State::Panicked(payload) => payload.into_inner(),
                _ => panic!(),
            }),
            ControlFlow::Continue(Poll::Pending) => {}
            ControlFlow::Break(infallible) => match infallible {},
        }
    }
}

impl<F: ControlFlowFuture> CompletionFuture for JoinAll<F> {
    type Output = ControlFlow<F::Break, JoinAllOutput<F>>;

    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.shared.waker.register(cx.waker());

        if let State::Running = self.state {
            match self.poll_with(FutureState::poll) {
                ControlFlow::Continue(poll) => {
                    return poll.map(|_| {
                        let futures = mem::replace(&mut self.futures, pin_vec(Vec::new()));
                        // SAFETY: All the futures are done, so this vector can be moved around
                        // in memory freely.
                        let futures = Pin::into_inner_unchecked(futures);
                        ControlFlow::Continue(JoinAllOutput {
                            inner: futures.into_iter(),
                            _correct_debug_bounds: PhantomData,
                        })
                    });
                }
                ControlFlow::Break(Ok(val)) => {
                    self.polled_once = false;
                    self.state = State::Broken(val);
                }
                ControlFlow::Break(Err(panic)) => {
                    self.polled_once = false;
                    self.state = State::Panicked(panic);
                }
            }
        }

        if let State::Broken(_) = &self.state {
            match self.poll_with(FutureState::poll_cancel) {
                ControlFlow::Continue(poll) => {
                    return poll.map(|state| {
                        ControlFlow::Break(match state {
                            State::Broken(val) => val,
                            _ => unreachable!(),
                        })
                    });
                }
                ControlFlow::Break(panic) => {
                    self.state = State::Panicked(panic);
                }
            }
        }

        self.poll_panicked();
        Poll::Pending
    }
    unsafe fn poll_cancel(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.shared.waker.register(cx.waker());

        if let State::Running | State::Broken(_) = self.state {
            self.state = State::Cancelling;
            self.polled_once = false;
        }

        if let State::Cancelling = self.state {
            match self.poll_with(FutureState::poll_cancel) {
                ControlFlow::Continue(poll) => return poll.map(|_| ()),
                ControlFlow::Break(panic) => {
                    self.state = State::Panicked(panic);
                }
            }
        }

        self.poll_panicked();
        Poll::Pending
    }
}

/// An iterator over the outputs of a `JoinAll`.
pub(super) struct JoinAllOutput<F: ControlFlowFuture> {
    inner: vec::IntoIter<FutureState<F>>,
    _correct_debug_bounds: PhantomData<F::Continue>,
}

impl<F: ControlFlowFuture> Iterator for JoinAllOutput<F> {
    type Item = F::Continue;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|item| match item {
            FutureState::Completed(val) => val,
            _ => unreachable!(),
        })
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
    fn count(self) -> usize {
        self.inner.count()
    }
}
impl<F: ControlFlowFuture> ExactSizeIterator for JoinAllOutput<F> {
    fn len(&self) -> usize {
        self.inner.len()
    }
}
impl<F: ControlFlowFuture> DoubleEndedIterator for JoinAllOutput<F> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|item| match item {
            FutureState::Completed(val) => val,
            _ => unreachable!(),
        })
    }
}
impl<F: ControlFlowFuture> FusedIterator for JoinAllOutput<F> {}

impl<F: ControlFlowFuture> Debug for JoinAllOutput<F>
where
    F::Continue: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        struct JoinAllOutputInner<'a, F: ControlFlowFuture>(&'a [FutureState<F>]);
        impl<F: ControlFlowFuture> Debug for JoinAllOutputInner<'_, F>
        where
            F::Continue: Debug,
        {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                f.debug_list()
                    .entries(self.0.iter().map(|future_state| match future_state {
                        FutureState::Completed(val) => val,
                        _ => unreachable!(),
                    }))
                    .finish()
            }
        }

        f.debug_tuple("JoinAllOutput")
            .field(&JoinAllOutputInner(self.inner.as_slice()))
            .finish()
    }
}

fn pin_vec<T>(vec: Vec<T>) -> Pin<Vec<T>> {
    // SAFETY: `Vec` provides no methods for pin projection.
    unsafe { Pin::new_unchecked(vec) }
}

fn slice_index_pin_mut<T>(slice: Pin<&mut [T]>, i: usize) -> Pin<&mut T> {
    unsafe { Pin::new_unchecked(&mut Pin::into_inner_unchecked(slice)[i]) }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::convert::Infallible;
    use std::future::ready;
    #[cfg(not(miri))]
    use std::panic::{catch_unwind, AssertUnwindSafe};
    #[cfg(not(miri))]
    use std::time::Duration;

    use crate::future::block_on;
    #[cfg(not(miri))]
    use crate::future::{CompletionFutureExt, FutureExt};
    use crate::test_utils::CompletionFutureExt as _;
    #[cfg(not(miri))]
    use crate::test_utils::{sleep, Yield};

    #[test]
    fn ready_continue() {
        assert_eq!(
            block_on::<ControlFlow<Infallible, _>, _>(JoinAll::new(vec![
                ready(ControlFlow::Continue(Box::new(0)))
                    .check()
                    .max_cancels(0),
                ready(ControlFlow::Continue(Box::new(1)))
                    .check()
                    .max_cancels(0),
                ready(ControlFlow::Continue(Box::new(2)))
                    .check()
                    .max_cancels(0),
            ]))
            .unwrap_continue()
            .collect::<Vec<_>>(),
            vec![Box::new(0), Box::new(1), Box::new(2)],
        );
    }

    #[test]
    fn ready_break() {
        assert_eq!(
            block_on(JoinAll::new(vec![
                ready(ControlFlow::Continue(vec![0])).check(),
                ready(ControlFlow::Continue(vec![1, 2, 3])).check(),
                ready(ControlFlow::Break(Box::new(4_i64)))
                    .check()
                    .max_cancels(0),
                ready(ControlFlow::Break(Box::new(5))).check().max_polls(0),
                ready(ControlFlow::Continue(vec![6, 7, 8]))
                    .check()
                    .max_polls(0),
            ]))
            .unwrap_break(),
            Box::new(4_i64),
        );
    }

    #[test]
    // Miri doesn't support boxed futures
    #[cfg(not(miri))]
    fn pending_continue() {
        assert_eq!(
            block_on::<ControlFlow<Infallible, _>, _>(JoinAll::new(vec![
                ready(ControlFlow::Continue(Box::new(0)))
                    .boxed()
                    .check()
                    .max_cancels(0),
                Yield::once(ready(ControlFlow::Continue(Box::new(1))))
                    .boxed()
                    .check()
                    .max_cancels(0),
                async {
                    sleep(Duration::from_millis(10)).await;
                    ControlFlow::Continue(Box::new(2))
                }
                .into_completion()
                .boxed()
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
                .boxed()
                .check()
                .max_polls(4)
                .max_cancels(0),
            ]))
            .unwrap_continue()
            .collect::<Vec<_>>(),
            vec![Box::new(0), Box::new(1), Box::new(2), Box::new(3)],
        );
    }

    #[test]
    // Miri doesn't support boxed futures
    #[cfg(not(miri))]
    fn all_pending_continue() {
        assert_eq!(
            block_on::<ControlFlow<Infallible, _>, _>(JoinAll::new(vec![
                async {
                    sleep(Duration::from_millis(10)).await;
                    ControlFlow::Continue(Box::new(0))
                }
                .into_completion()
                .boxed()
                .check()
                .max_polls(2),
                async {
                    sleep(Duration::from_millis(9)).await;
                    sleep(Duration::from_millis(2)).await;
                    ControlFlow::Continue(Box::new(1))
                }
                .into_completion()
                .boxed()
                .check()
                .max_polls(3),
                async {
                    sleep(Duration::from_millis(11)).await;
                    ControlFlow::Continue(Box::new(2))
                }
                .into_completion()
                .boxed()
                .check()
                .max_polls(2),
            ]))
            .unwrap_continue()
            .collect::<Vec<_>>(),
            vec![Box::new(0), Box::new(1), Box::new(2)],
        );
    }

    #[test]
    // Miri doesn't support boxed futures
    #[cfg(not(miri))]
    fn pending_break() {
        let mut x = false;
        assert_eq!(
            block_on(JoinAll::new(vec![
                async {
                    sleep(Duration::from_millis(2)).await;
                    x = true;
                    ControlFlow::Continue(())
                }
                .into_completion()
                .must_complete()
                .boxed()
                .check()
                .max_polls(1),
                async {
                    Yield::once(ready(())).await;
                    ControlFlow::Break(Box::new(6_i64))
                }
                .into_completion()
                .boxed()
                .check()
                .max_cancels(0),
            ]))
            .unwrap_break(),
            Box::new(6_i64),
        );
        assert!(x);
    }

    #[test]
    // Miri doesn't support boxed futures
    #[cfg(not(miri))]
    fn panic() {
        let mut x = false;
        let res = catch_unwind(AssertUnwindSafe(|| {
            block_on(JoinAll::new(vec![
                async {
                    Yield::once(ready(())).await;
                    x = true;
                    <ControlFlow<Infallible, ()>>::Continue(())
                }
                .into_completion()
                .must_complete()
                .boxed()
                .check()
                .max_polls(1),
                async { panic!(0) }.into_completion().boxed().check(),
                async { panic!(1) }.into_completion().boxed().check(),
            ]));
            panic!()
        }));

        assert_eq!(*res.unwrap_err().downcast::<i32>().unwrap(), 0);
        assert!(x);
    }

    #[test]
    // Miri doesn't support boxed futures
    #[cfg(not(miri))]
    fn panic_from_broken() {
        let mut x = false;
        let res = catch_unwind(AssertUnwindSafe(|| {
            block_on(JoinAll::new(vec![
                ready(ControlFlow::Break(())).boxed().check(),
                async {
                    Yield::once(ready(())).await;
                    x = true;
                    ControlFlow::Continue(())
                }
                .into_completion()
                .must_complete()
                .boxed()
                .check(),
                async { panic!(0) }
                    .into_completion()
                    .must_complete()
                    .boxed()
                    .check(),
            ]));
            panic!()
        }));

        assert_eq!(*res.unwrap_err().downcast::<i32>().unwrap(), 0);
        assert!(x);
    }

    #[test]
    // Miri is too slow
    #[cfg(not(miri))]
    fn many() {
        let count = 1_000_000;

        let res = block_on(JoinAll::new((0..count).map(|i| {
            async move {
                Yield::once(ready(())).await;
                <ControlFlow<Infallible, _>>::Continue(Box::new(i))
            }
            .into_completion()
            .check()
            .max_cancels(0)
        })));

        for (i, v) in res.unwrap_continue().enumerate() {
            assert_eq!(Box::new(i), v);
        }
    }
}
