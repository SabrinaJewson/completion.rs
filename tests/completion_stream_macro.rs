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

use completion::{completion_async, completion_stream, future::block_on, CompletionStreamExt};
use completion_core::CompletionStream;
use futures_lite::future::yield_now;
use futures_lite::pin;

#[test]
fn empty() {
    block_on(completion_async! {
        let stream = completion_stream!();
        pin!(stream);
        assert_eq!(stream.next().await, None);
    });
}

#[test]
fn side_effects() {
    block_on(completion_async! {
        let mut ran = false;

        {
            let ran = &mut ran;

            let stream = completion_stream! {
                *ran = true;
            };
            pin!(stream);
            assert_eq!(stream.next().await, None);
        }

        assert!(ran);
    });
}

#[test]
fn yielding() {
    block_on(completion_async! {
        let stream = completion_stream! {
            yield_now().await;
            yield_now().await;
            yield 1;
            yield_now().await;
            yield_now().await;
            yield 2;
            yield_now().await;
            yield 3;
        };

        assert_eq!(stream.collect::<Vec<_>>().await, [1, 2, 3]);
    });
}

#[test]
fn return_from_stream() {
    block_on(completion_async! {
        #[allow(unreachable_code)]
        let stream = completion_stream! {
            yield 1;
            return;
            yield 2;
        };

        assert_eq!(stream.collect::<Vec<_>>().await, [1]);
    });
}

#[test]
fn try_result() {
    block_on(completion_async! {
        let stream = completion_stream! {
            yield Ok::<_, i32>("");

            let res = Ok::<_, i8>(());
            res?;

            let res = Err(5_i8);
            res?;

            unreachable!();
        };

        pin!(stream);
        assert_eq!(stream.next().await, Some(Ok("")));
        assert_eq!(stream.next().await, Some(Err(5)));
        assert_eq!(stream.next().await, None);
    });
}

#[test]
fn try_option() {
    block_on(completion_async! {
        let stream = completion_stream! {
            yield Some(5);

            let res = Some("");
            let _ = res?;

            let res = None;
            res?;

            unreachable!();
        };

        pin!(stream);
        assert_eq!(stream.next().await, Some(Some(5)));
        assert_eq!(stream.next().await, Some(None));
        assert_eq!(stream.next().await, None);
    });
}

#[test]
fn r#return() {
    fn returns_stream() -> impl CompletionStream<Item = i32> {
        completion_stream! {
            yield 1;
            yield 2;
            yield 3;
            yield_now().await;
        }
    }

    block_on(completion_async! {
        assert_eq!(returns_stream().collect::<Vec<_>>().await, [1, 2, 3]);
    });
}

#[test]
fn nested() {
    let stream = completion_stream! {
        for i in 0..3 {
            yield i;
        }
    };
    pin!(stream);

    let stream = completion_stream! {
        while let Some(item) = stream.next().await {
            yield item * 2;
        }
    };

    assert_eq!(block_on(stream.collect::<Vec<_>>()), [0, 2, 4]);
}
