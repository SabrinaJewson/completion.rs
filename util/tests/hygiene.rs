#![cfg(feature = "macro")]
#![no_implicit_prelude]
#![no_std]

#[::completion_util::completion]
async fn _abc() {
    async {}.await;

    ::completion_util::completion_async!(
        async {}.await;
    )
    .await;
    ::completion_util::completion_async_move!(
        async {}.await;
    )
    .await;

    ::completion_util::completion_stream! {
        async {}.await;
        yield 1;
    };
}
