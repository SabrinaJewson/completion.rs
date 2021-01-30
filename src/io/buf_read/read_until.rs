use std::future::Future;
use std::io::Result;
use std::marker::PhantomPinned;
use std::pin::Pin;
use std::task::{Context, Poll};

use aliasable::boxed::AliasableBox;
use completion_core::CompletionFuture;
use completion_io::{AsyncBufRead, AsyncBufReadWith};
use pin_project_lite::pin_project;

use super::super::{AsyncReadExt, ReadToEnd};
use super::{extend_lifetime_mut, AsyncBufReadExt, TakeUntil};

pin_project! {
    /// Future for [`AsyncReadExt::read_until`](super::AsyncReadExt::read_until).
    pub struct ReadUntil<'a, R: ?Sized>
    where
        R: AsyncBufRead,
    {
        #[pin]
        inner: Option<ReadToEnd<'a, TakeUntil<&'a mut R>>>,
        take_until: AliasableBox<TakeUntil<&'a mut R>>,
        // In the future we want to allow `take_until` to be stored inline
        #[pin]
        _pinned: PhantomPinned,
        buf: Option<&'a mut Vec<u8>>,
    }
}

impl<'a, R: AsyncBufRead + ?Sized + 'a> ReadUntil<'a, R> {
    pub(super) fn new(reader: &'a mut R, delim: u8, buf: &'a mut Vec<u8>) -> Self {
        Self {
            inner: None,
            take_until: AliasableBox::from_unique(Box::new(reader.take_until(delim))),
            _pinned: PhantomPinned,
            buf: Some(buf),
        }
    }
}

impl<'a, R: AsyncBufRead + ?Sized + 'a> CompletionFuture for ReadUntil<'a, R> {
    type Output = Result<usize>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        if this.inner.as_mut().as_pin_mut().is_none() {
            let buf = this.buf.take().expect("polled after completion");
            let take_until = extend_lifetime_mut(this.take_until);
            this.inner.as_mut().set(Some(take_until.read_to_end(buf)));
        }
        this.inner.as_pin_mut().unwrap().poll(cx)
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if let Some(inner) = self.project().inner.as_pin_mut() {
            inner.poll_cancel(cx)
        } else {
            Poll::Ready(())
        }
    }
}
impl<'a, R: AsyncBufRead + ?Sized + 'a> Future for ReadUntil<'a, R>
where
    <R as AsyncBufReadWith<'a>>::FillBufFuture: Future<Output = Result<&'a [u8]>>,
{
    type Output = Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

#[test]
fn test() {
    use crate::future::block_on;

    use super::super::{test_utils::YieldingReader, AsyncBufReadExt};

    let mut reader = YieldingReader::new((0..50).map(|n| Ok([n * 2, n * 2 + 1])));

    let mut v = Vec::new();
    assert_eq!(block_on(reader.read_until(82, &mut v)).unwrap(), 83);
    assert_eq!(v, (0..=82).collect::<Vec<_>>());
}
