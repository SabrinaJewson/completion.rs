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
#![no_implicit_prelude]
#![no_std]

#[::completion::completion]
async fn _abc() {
    async {}.await;

    ::completion::completion_async!(
        async {}.await;
    )
    .await;
    ::completion::completion_async_move!(
        async {}.await;
    )
    .await;

    ::core::mem::drop(::completion::completion_stream! {
        async {}.await;
        yield 1;
    });
}
