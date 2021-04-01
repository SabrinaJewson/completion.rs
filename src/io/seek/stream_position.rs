use std::future::Future;
use std::io::Result;
use std::pin::Pin;
use std::task::{Context, Poll};

use completion_core::CompletionFuture;
use completion_io::{AsyncSeek, AsyncSeekWith, SeekFrom};

use pin_project_lite::pin_project;

pin_project! {
    /// Future for [`AsyncSeekExt::stream_position`](super::AsyncSeekExt::stream_position).
    pub struct StreamPosition<'a, T>
    where
        T: AsyncSeek,
        T: ?Sized,
    {
        #[pin]
        fut: <T as AsyncSeekWith<'a>>::SeekFuture,
    }
}

impl<'a, T: AsyncSeek + ?Sized> StreamPosition<'a, T> {
    pub(super) fn new(inner: &'a mut T) -> Self {
        Self {
            fut: inner.seek(SeekFrom::Current(0)),
        }
    }
}

impl<T: AsyncSeek + ?Sized> CompletionFuture for StreamPosition<'_, T> {
    type Output = Result<u64>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().fut.poll(cx)
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.project().fut.poll_cancel(cx)
    }
}
impl<'a, T: AsyncSeek + ?Sized> Future for StreamPosition<'a, T>
where
    <T as AsyncSeekWith<'a>>::SeekFuture: Future<Output = Result<u64>>,
{
    type Output = Result<u64>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}
