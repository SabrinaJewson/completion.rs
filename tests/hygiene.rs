#![cfg(all(feature = "macro", feature = "std"))]
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

    let _ = ::completion::completion_stream! {
        async {}.await;
        yield 1;
    };
}
