use std::future::Future;
use std::io::Result;
use std::pin::Pin;
use std::task::{Context, Poll};

use aliasable::AliasableMut;
use completion_core::CompletionFuture;
use completion_io::{AsyncBufRead, AsyncBufReadWith, AsyncReadWith, ReadBufMut};
use futures_core::ready;
use pin_project_lite::pin_project;

use super::extend_lifetime_mut;

/// Reader for [`AsyncReadExt::take_until`](super::AsyncBufReadExt::take_until).
#[derive(Debug)]
pub struct TakeUntil<R> {
    reader: R,
    delim: u8,
    // The number of bytes in the reader's internal buffer until the delimiter is reached. Some(0)
    // means we are at EOF.
    bytes_to_delim: Option<usize>,
}

impl<R> TakeUntil<R> {
    pub(super) fn new(reader: R, delim: u8) -> Self {
        Self {
            reader,
            delim,
            bytes_to_delim: None,
        }
    }
}

impl<'a, R: AsyncBufRead> AsyncReadWith<'a> for TakeUntil<R> {
    type ReadFuture = ReadTakeUntil<'a, R>;

    fn read(&'a mut self, buf: ReadBufMut<'a>) -> Self::ReadFuture {
        let mut this = AliasableMut::from_unique(self);
        ReadTakeUntil {
            inner: unsafe { extend_lifetime_mut(&mut *this) }.fill_buf(),
            reader: this,
            buf,
        }
    }
}

pin_project! {
    /// Future for [`read`](AsyncReadWith::read) on a [`TakeUntil`].
    pub struct ReadTakeUntil<'a, R: AsyncBufRead> {
        #[pin]
        inner: FillBufTakeUntil<'a, R>,
        reader: AliasableMut<'a, TakeUntil<R>>,
        buf: ReadBufMut<'a>,
    }
}

impl<R: AsyncBufRead> CompletionFuture for ReadTakeUntil<'_, R> {
    type Output = Result<()>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let available = ready!(this.inner.poll(cx))?;
        let amt = std::cmp::min(this.buf.remaining(), available.len());
        this.buf.append(&available[..amt]);
        this.reader.consume(amt);
        Poll::Ready(Ok(()))
    }
}

impl<'a, R: AsyncBufRead> Future for ReadTakeUntil<'a, R>
where
    <R as AsyncBufReadWith<'a>>::FillBufFuture: Future<Output = Result<&'a [u8]>>,
{
    type Output = Result<()>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

impl<'a, R: AsyncBufRead> AsyncBufReadWith<'a> for TakeUntil<R> {
    type FillBufFuture = FillBufTakeUntil<'a, R>;

    fn fill_buf(&'a mut self) -> Self::FillBufFuture {
        FillBufTakeUntil {
            fut: if self.bytes_to_delim == Some(0) {
                None
            } else {
                Some(self.reader.fill_buf())
            },
            delim: self.delim,
            bytes_to_delim: &mut self.bytes_to_delim,
        }
    }
    fn consume(&mut self, amt: usize) {
        self.reader.consume(amt);
        if let Some(bytes_to_delim) = &mut self.bytes_to_delim {
            *bytes_to_delim -= amt;
        }
    }
}

pin_project! {
    /// Future for [`fill_buf`](AsyncBufReadWith::fill_buf) on a [`TakeUntil`].
    pub struct FillBufTakeUntil<'a, R: AsyncBufRead> {
        #[pin]
        fut: Option<<R as AsyncBufReadWith<'a>>::FillBufFuture>,
        delim: u8,
        bytes_to_delim: &'a mut Option<usize>,
    }
}

impl<'a, R: AsyncBufRead> CompletionFuture for FillBufTakeUntil<'a, R> {
    type Output = Result<&'a [u8]>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        Poll::Ready(Ok(if let Some(fut) = this.fut.as_mut().as_pin_mut() {
            let buf = ready!(fut.poll(cx))?;
            this.fut.set(None);

            if let Some(index) = memchr::memchr(*this.delim, buf) {
                **this.bytes_to_delim = Some(index + 1);
                &buf[..=index]
            } else {
                buf
            }
        } else {
            &[]
        }))
    }
}

impl<'a, R: AsyncBufRead> Future for FillBufTakeUntil<'a, R>
where
    <R as AsyncBufReadWith<'a>>::FillBufFuture: Future<Output = Result<&'a [u8]>>,
{
    type Output = Result<&'a [u8]>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Error, ErrorKind};

    use crate::future::block_on;

    use super::super::super::AsyncReadExt;
    use super::super::{test_utils::YieldingReader, AsyncBufReadExt};

    #[test]
    fn no_yield() {
        let mut reader = Cursor::new((0..100).collect::<Vec<_>>()).take_until(82);

        let mut v = Vec::new();
        assert_eq!(block_on(reader.read_to_end(&mut v)).unwrap(), 83);
        assert_eq!(v, (0..=82).collect::<Vec<_>>());
    }

    #[test]
    fn yielding() {
        let mut reader =
            YieldingReader::new((0..50).map(|n| Ok([n * 2, n * 2 + 1]))).take_until(82);

        let mut v = Vec::new();
        assert_eq!(block_on(reader.read_to_end(&mut v)).unwrap(), 83);
        assert_eq!(v, (0..=82).collect::<Vec<_>>());
    }

    #[test]
    fn not_found() {
        let mut reader = YieldingReader::new((0..100).map(|n| Ok([n]))).take_until(100);

        let mut v = Vec::new();
        assert_eq!(block_on(reader.read_to_end(&mut v)).unwrap(), 100);
        assert_eq!(v, (0..100).collect::<Vec<_>>());
    }

    #[test]
    fn error() {
        let mut reader = YieldingReader::new(vec![
            Ok([1, 2, 3]),
            Err(Error::new(ErrorKind::Other, "Some error")),
            Ok([4, 5, 6]),
        ])
        .take_until(5);

        let mut v = Vec::new();
        assert_eq!(
            block_on(reader.read_to_end(&mut v))
                .unwrap_err()
                .to_string(),
            "Some error"
        );
        assert_eq!(v, [1, 2, 3]);
    }
}
