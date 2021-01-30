use core::any::Any;
use core::marker::PhantomData;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};

use completion_core::CompletionFuture;
use futures_core::ready;
use pin_project_lite::pin_project;

/// Joins two futures, waiting for them both to complete.
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
///     future::zip(
///         completion_async!(5),
///         completion_async!(6),
///     )
///     .await,
///     (5, 6),
/// );
/// # });
/// ```
pub fn zip<F1: CompletionFuture, F2: CompletionFuture>(future1: F1, future2: F2) -> Zip<F1, F2> {
    Zip {
        future1,
        future2,
        state: ZipState::Running,
        _correct_debug_bounds: PhantomData,
    }
}

pin_project! {
    /// Future for [`zip`].
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you use them"]
    pub struct Zip<F1: CompletionFuture, F2: CompletionFuture> {
        #[pin]
        future1: F1,
        #[pin]
        future2: F2,
        state: ZipState<F1, F2>,
        _correct_debug_bounds: PhantomData<(F1::Output, F2::Output)>,
    }
}

#[derive(Debug)]
enum ZipState<F1: CompletionFuture, F2: CompletionFuture> {
    Running,
    F1Complete(F1::Output),
    F2Complete(F2::Output),
    F1Cancelled,
    F2Cancelled,
    F1Panicked(Box<dyn Any + Send>),
    F2Panicked(Box<dyn Any + Send>),
}

impl<F1: CompletionFuture, F2: CompletionFuture> CompletionFuture for Zip<F1, F2> {
    type Output = (F1::Output, F2::Output);

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        match &*this.state {
            ZipState::Running => {
                match catch_unwind(AssertUnwindSafe(|| this.future1.as_mut().poll(cx))) {
                    Ok(Poll::Ready(output)) => *this.state = ZipState::F1Complete(output),
                    Ok(Poll::Pending) => {}
                    Err(payload) => *this.state = ZipState::F1Panicked(payload),
                }
            }
            ZipState::F2Complete(_) => {
                let f1_output = ready!(this.future1.as_mut().poll(cx));
                let f2_output = match std::mem::replace(this.state, ZipState::Running) {
                    ZipState::F2Complete(output) => output,
                    _ => unreachable!(),
                };
                return Poll::Ready((f1_output, f2_output));
            }
            ZipState::F2Panicked(_) => {
                ready!(this.future1.as_mut().poll_cancel(cx));
                match std::mem::replace(this.state, ZipState::Running) {
                    ZipState::F2Panicked(payload) => resume_unwind(payload),
                    _ => unreachable!(),
                }
            }
            _ => {}
        }

        match &*this.state {
            ZipState::Running => {
                match catch_unwind(AssertUnwindSafe(|| this.future2.as_mut().poll(cx))) {
                    Ok(Poll::Ready(output)) => *this.state = ZipState::F2Complete(output),
                    Ok(Poll::Pending) => {}
                    Err(payload) => {
                        if let Poll::Ready(()) = this.future1.as_mut().poll_cancel(cx) {
                            resume_unwind(payload);
                        }
                        *this.state = ZipState::F2Panicked(payload);
                    }
                }
            }
            ZipState::F1Complete(_) => {
                let f2_output = ready!(this.future2.as_mut().poll(cx));
                let f1_output = match std::mem::replace(this.state, ZipState::Running) {
                    ZipState::F1Complete(output) => output,
                    _ => unreachable!(),
                };
                return Poll::Ready((f1_output, f2_output));
            }
            ZipState::F1Panicked(_) => {
                ready!(this.future2.as_mut().poll_cancel(cx));
                match std::mem::replace(this.state, ZipState::Running) {
                    ZipState::F1Panicked(payload) => resume_unwind(payload),
                    _ => unreachable!(),
                }
            }
            #[cfg(debug_assertions)]
            ZipState::F1Cancelled | ZipState::F2Cancelled => {
                panic!("Called `poll` after `poll_cancel` on `Zip`");
            }
            _ => {}
        }

        Poll::Pending
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut this = self.project();

        match &*this.state {
            ZipState::Running => {
                match catch_unwind(AssertUnwindSafe(|| this.future1.as_mut().poll_cancel(cx))) {
                    Ok(Poll::Ready(())) => *this.state = ZipState::F1Cancelled,
                    Ok(Poll::Pending) => {}
                    Err(payload) => *this.state = ZipState::F1Panicked(payload),
                }
            }
            ZipState::F2Complete(_) | ZipState::F2Cancelled => {
                return this.future1.as_mut().poll_cancel(cx);
            }
            ZipState::F2Panicked(_) => {
                ready!(this.future1.as_mut().poll_cancel(cx));
                match std::mem::replace(this.state, ZipState::Running) {
                    ZipState::F2Panicked(payload) => resume_unwind(payload),
                    _ => unreachable!(),
                }
            }
            _ => {}
        }

        match &*this.state {
            ZipState::Running => {
                match catch_unwind(AssertUnwindSafe(|| this.future2.as_mut().poll_cancel(cx))) {
                    Ok(Poll::Ready(())) => *this.state = ZipState::F2Cancelled,
                    Ok(Poll::Pending) => {}
                    Err(payload) => *this.state = ZipState::F2Panicked(payload),
                }
            }
            ZipState::F1Complete(_) | ZipState::F1Cancelled => {
                return this.future2.as_mut().poll_cancel(cx);
            }
            ZipState::F1Panicked(_) => {
                ready!(this.future2.as_mut().poll_cancel(cx));
                match std::mem::replace(this.state, ZipState::Running) {
                    ZipState::F1Panicked(payload) => resume_unwind(payload),
                    _ => unreachable!(),
                }
            }
            _ => {}
        }

        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use std::future::ready;
    use std::panic::AssertUnwindSafe;

    use crate::future::{block_on, CompletionFutureExt, FutureExt};
    use crate::test_utils::Yield;

    use super::zip;

    #[test]
    fn success() {
        for i in 0..10 {
            for j in 0..10 {
                let f1 = Yield::new(i, ready(Box::new(5)));
                let f2 = Yield::new(j, ready(Box::new(7)));
                assert_eq!(block_on(zip(f1, f2)), (Box::new(5), Box::new(7)));
            }
        }
    }

    #[test]
    fn panics() {
        let mut x = 0;

        assert_eq!(
            *block_on(
                AssertUnwindSafe(zip(
                    async { panic!(5) }.into_completion(),
                    async {
                        x += 1;
                    }
                    .into_completion(),
                ))
                .catch_unwind()
            )
            .unwrap_err()
            .downcast::<i32>()
            .unwrap(),
            5
        );
        assert_eq!(x, 0);

        assert_eq!(
            *block_on(
                AssertUnwindSafe(zip(
                    async {
                        x = 10;
                        Yield::once(ready(())).await;
                        x += 1;
                    }
                    .into_completion(),
                    async { panic!(6) }.into_completion(),
                ))
                .catch_unwind()
            )
            .unwrap_err()
            .downcast::<i32>()
            .unwrap(),
            6
        );
        assert_eq!(x, 10);
    }

    #[test]
    fn nested() {
        assert_eq!(
            block_on(zip(
                zip(
                    zip(ready(Box::new(0)), ready(Box::new(1))),
                    ready(Box::new(2)),
                ),
                zip(
                    ready(Box::new(3)),
                    zip(ready(Box::new(4)), ready(Box::new(5)))
                ),
            )),
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
                AssertUnwindSafe(zip(
                    zip(
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
                    ),
                    zip(
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
                    ),
                ))
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
