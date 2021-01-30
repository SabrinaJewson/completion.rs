use std::future::Future;
use std::io::Result;
use std::marker::PhantomPinned;
use std::pin::Pin;
use std::task::{Context, Poll};

use aliasable::boxed::AliasableBox;
use completion_core::CompletionFuture;
use completion_io::{AsyncBufRead, AsyncBufReadWith};
use pin_project_lite::pin_project;

use super::super::{AsyncReadExt, ReadToString};
use super::{extend_lifetime_mut, AsyncBufReadExt, TakeUntil};

pin_project! {
    /// Future for [`AsyncReadExt::read_line`](super::AsyncReadExt::read_line).
    pub struct ReadLine<'a, R: ?Sized>
    where
        R: AsyncBufRead,
    {
        #[pin]
        inner: Option<ReadToString<'a, TakeUntil<&'a mut R>>>,
        take_until: AliasableBox<TakeUntil<&'a mut R>>,
        // We want to support `take_until` being stored inline in the future
        #[pin]
        _pinned: PhantomPinned,
        buf: Option<&'a mut String>,
    }
}

impl<'a, R: AsyncBufRead + ?Sized + 'a> ReadLine<'a, R> {
    pub(super) fn new(reader: &'a mut R, buf: &'a mut String) -> Self {
        Self {
            inner: None,
            take_until: AliasableBox::from_unique(Box::new(reader.take_until(b'\n'))),
            _pinned: PhantomPinned,
            buf: Some(buf),
        }
    }
}

impl<'a, R: AsyncBufRead + ?Sized + 'a> CompletionFuture for ReadLine<'a, R> {
    type Output = Result<usize>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        if this.inner.as_mut().as_pin_mut().is_none() {
            let buf = this.buf.take().expect("polled after completion");
            let take_until = extend_lifetime_mut(this.take_until);
            this.inner
                .as_mut()
                .set(Some(take_until.read_to_string(buf)));
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
impl<'a, R: AsyncBufRead + ?Sized + 'a> Future for ReadLine<'a, R>
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

    use super::{test_utils::YieldingReader, AsyncBufReadExt};

    let mut reader = YieldingReader::new(vec![
        Ok("First Line\n"),
        Ok("Second Line"),
        Ok("\n"),
        Ok("Third Line\nFourth Line"),
    ]);

    let lines = [
        "First Line\n",
        "Second Line\n",
        "Third Line\n",
        "Fourth Line",
    ];

    for &line in &lines {
        let mut s = String::new();
        assert_eq!(block_on(reader.read_line(&mut s)).unwrap(), line.len());
        assert_eq!(s, line);
    }
}
