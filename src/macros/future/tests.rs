#![deny(warnings)]
#![deny(
    absolute_paths_not_starting_with_crate,
    box_pointers,
    explicit_outlives_requirements,
    keyword_idents,
    macro_use_extern_crate,
    meta_variable_misuse,
    missing_copy_implementations,
    missing_crate_level_docs,
    missing_debug_implementations,
    missing_doc_code_examples,
    missing_docs,
    pointer_structural_match,
    private_doc_tests,
    single_use_lifetimes,
    trivial_casts,
    trivial_numeric_casts,
    unaligned_references,
    unreachable_pub,
    unsafe_code,
    unstable_features,
    unused_extern_crates,
    unused_import_braces,
    unused_lifetimes,
    unused_qualifications,
    unused_results,
    variant_size_differences
)]
#![deny(clippy::pedantic)]

use std::borrow::Cow;
use std::cell::Cell;
use std::future::{self, Future};
use std::pin::Pin;
use std::task::{Context, Poll};

use completion_core::CompletionFuture;
use futures_lite::{future::yield_now, pin};

use crate::test_utils;
use crate::CompletionFutureExt;
use crate::{completion, completion_async, completion_async_move, future::block_on};

#[completion(crate = crate)]
async fn empty_async() {}

#[test]
fn empty() {
    block_on(empty_async());
}

#[completion(crate = crate)]
async fn nested_async() {
    #[completion(crate = crate)]
    async fn inner_function() {
        yield_now().await;
        empty_async().await;
        yield_now().await;
    }
    async fn inner_function2() {
        yield_now().await;
        yield_now().await;
        yield_now().await;
    }

    empty_async().await;
    inner_function().await;
    inner_function2().await;
    inner_function().await;
}

#[test]
fn nested() {
    block_on(nested_async());
}

#[test]
#[cfg(not(miri))]
fn await_parens() {
    block_on(completion_async! {
        let mut future = yield_now();
        (&mut future).await;
        { &mut future }.await;
    });
}

#[completion(crate = crate)]
#[allow(single_use_lifetimes, clippy::trivially_copy_pass_by_ref)]
async fn lifetimes_async<'a, 'b, T>(x: &'a T, y: &&&String, z: &mut Cow<'a, str>) -> &'a T {
    let _ = y;
    let _ = z;
    x
}

#[test]
fn lifetimes() {
    let _: &i32 = block_on(lifetimes_async(
        &5,
        &&&String::new(),
        &mut Cow::Borrowed(""),
    ));
}

struct X;
impl X {
    #[completion(crate = crate)]
    #[allow(clippy::trivially_copy_pass_by_ref)]
    async fn method(&self, param: &'static i32) -> impl Clone {
        let _ = self;
        let _ = param;
        5
    }
}

#[test]
fn method() {
    let x = block_on(X.method(&9));
    drop(x.clone());
}

#[test]
fn block_yielding() {
    assert_eq!(
        block_on(completion_async! {
            yield_now().await;
            yield_now().await;
            5
        }),
        5
    );
}

#[test]
fn block_move() {
    fn block_on_static<F: CompletionFuture + 'static>(fut: F) -> F::Output {
        block_on(fut)
    }

    let data = "abc".to_owned();

    assert_eq!(block_on(completion_async! { data.clone() }), "abc");
    assert_eq!(
        block_on_static(completion_async_move! { data.clone() }),
        "abc"
    );
}

#[test]
fn block_try() {
    let res = block_on(completion_async! {
        let res = Ok::<_, i8>(5);
        assert_eq!(res?, 5);
        let res = Err(6);
        res?;
        unreachable!();

        #[allow(unreachable_code)]
        Ok(())
    });
    assert_eq!(res, Err(6));
}

#[test]
fn send() {
    fn requires_send<T: Send>(_: T) {}
    #[completion(crate = crate)]
    async fn x() {}

    requires_send(completion_async! {});
    requires_send(completion_async_move! {
        yield_now().await;
    });
    requires_send(x());
}

#[test]
fn cancel_completion_future() {
    struct Fut<'a> {
        stage: u8,
        number: &'a Cell<i32>,
    }
    #[allow(unsafe_code)]
    impl CompletionFuture for Fut<'_> {
        type Output = ();
        unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            self.number.set(1);
            cx.waker().wake_by_ref();
            Poll::Pending
        }
        unsafe fn poll_cancel(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            match self.stage {
                0 => self.number.set(2),
                1 => {
                    self.number.set(3);
                    return Poll::Ready(());
                }
                _ => unreachable!(),
            }
            self.stage += 1;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }

    let number = Cell::new(0);

    let fut = completion_async! {
        Fut { stage: 0, number: &number }.await;
    };
    futures_lite::pin!(fut);

    assert_eq!(test_utils::poll_once(fut.as_mut()), None);
    assert_eq!(number.get(), 1);

    assert!(!test_utils::poll_cancel_once(fut.as_mut()));
    assert_eq!(number.get(), 2);

    assert!(test_utils::poll_cancel_once(fut.as_mut()));
    assert_eq!(number.get(), 3);
}

#[test]
fn cancel_future() {
    struct Fut<'a> {
        number: &'a Cell<i32>,
    }
    #[allow(unsafe_code)]
    impl CompletionFuture for Fut<'_> {
        type Output = ();
        unsafe fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            panic!()
        }
        unsafe fn poll_cancel(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
            panic!()
        }
    }
    impl Future for Fut<'_> {
        type Output = ();
        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            self.number.set(self.number.get() + 1);
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }

    let number = Cell::new(0);

    let fut = completion_async! {
        number.set(1000);
        let _ = future::ready(5).must_complete().await;
        number.set(1);
        Fut { number: &number }.await;
    };
    pin!(fut);

    assert_eq!(test_utils::poll_once(fut.as_mut()), None);
    assert_eq!(number.get(), 2);

    assert!(test_utils::poll_cancel_once(fut.as_mut()));
    assert_eq!(number.get(), 2);
}
