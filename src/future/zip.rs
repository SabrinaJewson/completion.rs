use core::any::Any;
use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};

use completion_core::CompletionFuture;
use pin_project_lite::pin_project;

/// Joins futures, waiting for them all to complete.
///
/// This takes any tuple of two or more futures, and outputs a tuple of the results.
///
/// Requires the `std` feature, as [`catch_unwind`] is needed when polling the futures to ensure
/// soundness.
///
/// # Examples
///
/// ```
/// use completion::{future, completion_async};
///
/// # future::block_on(completion_async! {
/// assert_eq!(
///     future::zip((
///         completion_async!(5),
///         completion_async!(6),
///     ))
///     .await,
///     (5, 6),
/// );
/// # });
/// ```
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub fn zip<T: Zippable>(futures: T) -> Zip<T> {
    Zip {
        futures,
        state: State::Running(T::Running::default()),
        _correct_debug_bounds: PhantomData,
    }
}

pin_project! {
    /// Future for [`zip`].
    #[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you use them"]
    pub struct Zip<T: Zippable> {
        #[pin]
        futures: T,
        state: State<T>,
        _correct_debug_bounds: PhantomData<(T::Running, T::Cancelling)>,
    }
}

#[derive(Debug)]
enum State<T: Zippable> {
    Running(T::Running),
    Cancelling(T::Cancelling),
    Panicked(Box<dyn Any + Send>, T::Cancelling),
    Dummy,
}

impl<T: Zippable> Zip<T> {
    fn poll_panicked(self: Pin<&mut Self>, cx: &mut Context<'_>) {
        let this = self.project();

        let cancelling = match this.state {
            State::Panicked(_, cancelling) => cancelling,
            State::Cancelling(_) => panic!("Called `poll` after `poll_cancel` on `Zip`"),
            State::Dummy => panic!("Called `poll` or `poll_cancel` after panicking on `Zip`"),
            _ => unreachable!(),
        };
        if this.futures.poll_panicked(cancelling, cx).is_ready() {
            let payload = match std::mem::replace(this.state, State::Dummy) {
                State::Panicked(payload, _) => payload,
                _ => unreachable!(),
            };
            resume_unwind(payload)
        }
    }
}

impl<T: Zippable> CompletionFuture for Zip<T> {
    type Output = T::Outputs;

    unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.as_mut().project();

        if let State::Running(running) = this.state {
            match this.futures.as_mut().poll_all(running, cx) {
                Ok(res) => return res,
                Err(Panicked { i, payload }) => {
                    let mut cancelling = T::make_cancelling(running);
                    T::set_cancelled(&mut cancelling, i);
                    *this.state = State::Panicked(payload, cancelling);
                }
            }
        }
        self.poll_panicked(cx);
        Poll::Pending
    }
    unsafe fn poll_cancel(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut this = self.as_mut().project();

        if let State::Running(running) = this.state {
            *this.state = State::Cancelling(T::make_cancelling(running));
        }
        if let State::Cancelling(cancelling) = this.state {
            match this.futures.as_mut().poll_cancel(cancelling, cx) {
                Ok(res) => return res,
                Err(Panicked { i, payload }) => {
                    let mut cancelling = match std::mem::replace(this.state, State::Dummy) {
                        State::Cancelling(cancelling) => cancelling,
                        _ => unreachable!(),
                    };
                    T::set_cancelled(&mut cancelling, i);
                    *this.state = State::Panicked(payload, cancelling);
                }
            }
        }
        self.poll_panicked(cx);
        Poll::Pending
    }
}

/// The error returned when future in a `Zippable` panics.
#[allow(missing_debug_implementations)]
pub struct Panicked {
    i: usize,
    payload: Box<dyn Any + Send>,
}

pub trait Zippable: Sized {
    type Running: Default;
    type Cancelling;
    type Outputs;

    fn make_cancelling(running: &mut Self::Running) -> Self::Cancelling;

    fn set_cancelled(cancelling: &mut Self::Cancelling, i: usize);

    fn poll_all(
        self: Pin<&mut Self>,
        running: &mut Self::Running,
        cx: &mut Context<'_>,
    ) -> Result<Poll<Self::Outputs>, Panicked>;

    fn poll_cancel(
        self: Pin<&mut Self>,
        cancelling: &mut Self::Cancelling,
        cx: &mut Context<'_>,
    ) -> Result<Poll<()>, Panicked>;

    fn poll_panicked(
        self: Pin<&mut Self>,
        cancelling: &mut Self::Cancelling,
        cx: &mut Context<'_>,
    ) -> Poll<()>;
}

macro_rules! repeat_with {
    ($repeater:tt, $($tokens:tt)*) => { $($tokens)* };
}

macro_rules! implement_zippable_for_tuples {
    ($(($($param:ident),*),)*) => { $(#[allow(non_snake_case)] const _: () = {
        struct Futures<'a, $($param,)*> {
            $($param: Pin<&'a mut $param>,)*
        }
        fn make_futures<$($param,)*>(tuple: Pin<&mut ($($param,)*)>) -> Futures<'_, $($param,)*> {
            let ($($param,)*) = unsafe { Pin::into_inner_unchecked(tuple) };
            let ($($param,)*) = ($(unsafe { Pin::new_unchecked($param) },)*);
            Futures { $($param,)* }
        }

        impl<$($param: CompletionFuture,)*> Zippable for ($($param,)*) {
            type Running = ($(Option<$param::Output>,)*);
            type Cancelling = ($(repeat_with!($param, bool),)*);
            type Outputs = ($($param::Output,)*);

            fn make_cancelling(($($param,)*): &mut Self::Running) -> Self::Cancelling {
                ($($param.take().is_some(),)*)
            }

            fn set_cancelled(($($param,)*): &mut Self::Cancelling, i: usize) {
                let mut index = 0;
                $(
                    if index == i {
                        *$param = true;
                    }
                    #[allow(unused_assignments)]
                    {index += 1}
                )*
            }

            fn poll_all(
                self: Pin<&mut Self>,
                ($($param,)*): &mut Self::Running,
                cx: &mut Context<'_>,
            ) -> Result<Poll<Self::Outputs>, Panicked> {
                let futures = make_futures(self);

                let mut all_done = true;
                let mut i = 0;
                $(
                    if $param.is_none() {
                        let future = futures.$param;
                        let poll = catch_unwind(AssertUnwindSafe(|| unsafe { future.poll(cx) }))
                            .map_err(|payload| Panicked { i, payload })?;
                        if let Poll::Ready(val) = poll {
                            *$param = Some(val);
                        } else {
                            all_done = false;
                        }
                    }
                    #[allow(unused_assignments)]
                    {i += 1}
                )*
                Ok(if all_done {
                    Poll::Ready(($($param.take().unwrap(),)*))
                } else {
                    Poll::Pending
                })
            }

            fn poll_cancel(
                self: Pin<&mut Self>,
                ($($param,)*): &mut Self::Cancelling,
                cx: &mut Context<'_>,
            ) -> Result<Poll<()>, Panicked> {
                let futures = make_futures(self);

                let mut all_done = true;
                let mut i = 0;
                $(
                    if !*$param {
                        let future = futures.$param;
                        let poll = catch_unwind(AssertUnwindSafe(|| unsafe { future.poll_cancel(cx) }))
                            .map_err(|payload| Panicked { i, payload })?;
                        if poll.is_ready() {
                            *$param = true;
                        } else {
                            all_done = false;
                        }
                    }
                    #[allow(unused_assignments)]
                    {i += 1}
                )*
                Ok(if all_done {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                })
            }

            fn poll_panicked(
                self: Pin<&mut Self>,
                ($($param,)*): &mut Self::Cancelling,
                cx: &mut Context<'_>,
            ) -> Poll<()> {
                let futures = make_futures(self);

                let mut all_done = true;
                $(
                    if !*$param {
                        let future = futures.$param;
                        let ready = catch_unwind(AssertUnwindSafe(|| unsafe { future.poll_cancel(cx) }))
                            .map_or(true, |poll| poll.is_ready());
                        if ready {
                            *$param = true;
                        } else {
                            all_done = false;
                        }
                    }
                )*
                if all_done {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            }
        }
    };)*}
}

implement_zippable_for_tuples! {
    (A, B),
    (A, B, C),
    (A, B, C, D),
    (A, B, C, D, E),
    (A, B, C, D, E, F),
    (A, B, C, D, E, F, G),
    (A, B, C, D, E, F, G, H),
    (A, B, C, D, E, F, G, H, I),
    (A, B, C, D, E, F, G, H, I, J),
    (A, B, C, D, E, F, G, H, I, J, K),
    (A, B, C, D, E, F, G, H, I, J, K, L),
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::future::ready;
    use std::panic::AssertUnwindSafe;

    use crate::future::{block_on, CompletionFutureExt, FutureExt};
    use crate::test_utils::Yield;

    use super::zip;

    #[test]
    fn success() {
        for i in 0..10 {
            for j in 0..10 {
                let f1 = Yield::new(i, ready(Box::new(1)));
                let f2 = Yield::new(j, ready(Box::new(2)));
                let f3 = Yield::new(j, ready(Box::new(3)));
                assert_eq!(
                    block_on(zip((f1, f2, f3))),
                    (Box::new(1), Box::new(2), Box::new(3))
                );
            }
        }
    }

    #[test]
    fn panics() {
        let mut x = 0;

        assert_eq!(
            *block_on(
                AssertUnwindSafe(zip((
                    async { panic!(5) }.into_completion(),
                    async {
                        x += 1;
                    }
                    .into_completion(),
                )))
                .catch_unwind()
            )
            .unwrap_err()
            .downcast::<i32>()
            .unwrap(),
            5
        );
        assert_eq!(x, 0);

        struct Fut<'a> {
            x: &'a mut i32,
            yielded: bool,
        }
        impl CompletionFuture for Fut<'_> {
            type Output = ();
            unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                assert_eq!(*self.x, 0);
                *self.x = 10;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            unsafe fn poll_cancel(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
                if self.yielded {
                    assert_eq!(*self.x, 11);
                    *self.x = 12;
                    Poll::Ready(())
                } else {
                    assert_eq!(*self.x, 10);
                    *self.x = 11;
                    cx.waker().wake_by_ref();
                    self.yielded = true;
                    Poll::Pending
                }
            }
        }

        assert_eq!(
            *block_on(
                AssertUnwindSafe(zip((
                    Fut {
                        x: &mut x,
                        yielded: false
                    },
                    async { panic!(6) }.into_completion(),
                    async { panic!(7) }.into_completion(),
                )))
                .catch_unwind()
            )
            .unwrap_err()
            .downcast::<i32>()
            .unwrap(),
            6
        );
        assert_eq!(x, 12);
    }

    #[test]
    fn nested() {
        assert_eq!(
            block_on(zip((
                zip((
                    zip((ready(Box::new(0)), ready(Box::new(1)))),
                    ready(Box::new(2)),
                )),
                zip((
                    ready(Box::new(3)),
                    zip((ready(Box::new(4)), ready(Box::new(5))))
                )),
            ))),
            (
                ((Box::new(0), Box::new(1)), Box::new(2)),
                (Box::new(3), (Box::new(4), Box::new(5)))
            ),
        );
    }

    #[test]
    #[allow(unused_assignments)]
    fn nested_panic() {
        let (mut x, mut y, mut z) = (0, 0, 0);

        assert_eq!(
            *block_on(
                AssertUnwindSafe(zip((
                    zip((
                        async {
                            Yield::once(ready(())).await;
                            x = 1;
                            Yield::once(ready(())).await;
                            x = 2;
                        }
                        .into_completion(),
                        async {
                            Yield::once(ready(())).await;
                            panic!(2)
                        }
                        .into_completion()
                    )),
                    zip((
                        async {
                            y = 1;
                            Yield::once(ready(())).await;
                            y = 2;
                        }
                        .into_completion(),
                        async {
                            z = 1;
                            Yield::once(ready(())).await;
                            z = 2;
                        }
                        .into_completion()
                    )),
                )))
                .catch_unwind()
            )
            .unwrap_err()
            .downcast::<i32>()
            .unwrap(),
            2
        );

        assert_eq!((x, y, z), (1, 1, 1));
    }
}
