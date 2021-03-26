use std::cell::RefCell;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};

use completion_core::CompletionFuture;

/// Blocks the current thread on a completion future.
///
/// # Examples
///
/// ```
/// use completion::{completion_async, future};
///
/// assert_eq!(future::block_on(completion_async! { 5 + 6 }), 11);
/// ```
#[cfg_attr(doc_cfg, doc(cfg(feature = "std")))]
pub fn block_on<O, F: CompletionFuture<Output = O>>(mut future: F) -> O {
    let mut fut = unsafe { Pin::new_unchecked(&mut future) };

    thread_local! {
        static CACHE: RefCell<(Parker, Waker)> = RefCell::new(wake_pair());
    }

    CACHE.with(|cache| {
        let guard_storage;
        let new_pair_storage;

        let (parker, waker) = if let Ok(guard) = cache.try_borrow_mut() {
            guard_storage = guard;
            (&guard_storage.0, &guard_storage.1)
        } else {
            new_pair_storage = wake_pair();
            (&new_pair_storage.0, &new_pair_storage.1)
        };

        let mut cx = Context::from_waker(waker);

        loop {
            if let Poll::Ready(output) = unsafe { fut.as_mut().poll(&mut cx) } {
                break output;
            }
            parker.park();
        }
    })
}

fn wake_pair() -> (Parker, Waker) {
    let inner = Arc::new(WakerInner {
        woken: AtomicBool::new(false),
        sleeping_thread: thread::current(),
    });
    (
        Parker {
            inner: Arc::clone(&inner),
            not_send_or_sync: PhantomData,
        },
        Waker::from(inner),
    )
}

struct Parker {
    inner: Arc<WakerInner>,
    not_send_or_sync: PhantomData<*mut ()>,
}
impl Parker {
    fn park(&self) {
        while !self.inner.woken.swap(false, Ordering::Relaxed) {
            thread::park();
        }
    }
}

struct WakerInner {
    woken: AtomicBool,
    sleeping_thread: Thread,
}
impl Wake for WakerInner {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        if !self.woken.swap(true, Ordering::Relaxed) {
            self.sleeping_thread.unpark();
        }
    }
}

#[test]
fn test_block_on() {
    use futures_lite::future;

    use crate::FutureExt;

    assert_eq!(
        block_on(
            async {
                let val = 5;
                let val_ref = &val;
                let mut val_mut = 6;
                let val_mut_ref = &mut val_mut;

                future::yield_now().await;
                let v1 = async { 3 }.await;
                future::yield_now().await;
                future::yield_now().await;
                let v2 = async { 2 }.await;

                let res = async move {
                    future::yield_now().await;
                    v1 + v2
                }
                .await;

                // https://github.com/rust-lang/rust/issues/63818
                #[cfg(not(miri))]
                {
                    let _v = *val_ref;
                    *val_mut_ref += 1;
                }

                let _ = val_ref;
                let _ = val_mut_ref;

                res
            }
            .into_completion()
        ),
        5
    );
}
