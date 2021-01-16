#![cfg(all(feature = "macro", feature = "std"))]
#![deny(warnings)]
#![deny(
    absolute_paths_not_starting_with_crate,
    box_pointers,
    elided_lifetimes_in_paths,
    explicit_outlives_requirements,
    keyword_idents,
    macro_use_extern_crate,
    meta_variable_misuse,
    missing_copy_implementations,
    missing_crate_level_docs,
    missing_debug_implementations,
    missing_doc_code_examples,
    missing_docs,
    non_ascii_idents,
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

use completion::{completion, completion_async, completion_async_move, future::block_on};
use completion_core::CompletionFuture;
use futures_lite::future::yield_now;

#[completion]
async fn empty_async() {}

#[test]
fn empty() {
    block_on(empty_async());
}

#[completion]
async fn nested_async() {
    #[completion]
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

#[completion]
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
    #[completion]
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
    #[completion]
    async fn x() {}

    requires_send(completion_async! {});
    requires_send(completion_async_move! {
        yield_now().await;
    });
    requires_send(x());
}
