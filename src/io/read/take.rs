#![allow(clippy::cast_possible_truncation)]

use std::future::Future;
use std::io::Result;
use std::marker::PhantomPinned;
use std::pin::Pin;
use std::task::{Context, Poll};

use aliasable::{boxed::AliasableBox, AliasableMut};
use completion_core::CompletionFuture;
use completion_io::{
    AsyncBufRead, AsyncBufReadWith, AsyncRead, AsyncReadWith, ReadBuf, ReadBufMut,
};
use futures_core::ready;
use pin_project_lite::pin_project;

use super::extend_lifetime_mut;

/// Reader for [`AsyncReadExt::take`](super::AsyncReadExt::take).
#[derive(Debug)]
pub struct Take<T> {
    inner: T,
    limit: u64,
}

impl<T> Take<T> {
    pub(super) fn new(inner: T, limit: u64) -> Self {
        Self { inner, limit }
    }

    /// Get the number of bytes that can be read before this instance will return EOF.
    ///
    /// # Note
    ///
    /// The instance may reach EOF after reading fewer bytes than indicated by this method if the
    /// underlying [`AsyncRead`] instance reaches EOF.
    pub fn limit(&self) -> u64 {
        self.limit
    }

    /// Set the number of bytes that can be read before this instance will return EOF. This is the
    /// same as constructing a new `Take` instance, so the amount of bytes read and the previous
    /// limit value don't matter when calling this method.
    pub fn set_limit(&mut self, limit: u64) {
        self.limit = limit;
    }

    /// Consume the `Take`, returning the wrapped reader.
    pub fn into_inner(self) -> T {
        self.inner
    }

    /// Get a shared reference to the underlying reader.
    pub fn get_ref(&self) -> &T {
        &self.inner
    }

    /// Get a mutable reference to the underlying reader.
    ///
    /// Care should be taken to avoid modifying the internal I/O state of the underlying reader as
    /// doing so may corrupt the internal limit of this `Take`.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<'a, T: AsyncRead> AsyncReadWith<'a> for Take<T> {
    type ReadFuture = ReadTake<'a, T>;

    fn read(&'a mut self, buf: ReadBufMut<'a>) -> Self::ReadFuture {
        let mut buf = AliasableMut::from_unique(unsafe { buf.into_mut() });

        // If we have reached EOF, bypass the reader entirely.
        if self.limit == 0 {
            ReadTake {
                fut: None,
                short_buf: None,
                _pinned: PhantomPinned,
                buf,
                initial_filled: 0,
                limit: &mut self.limit,
            }
        } else {
            let initial_initialized = buf.initialized().len();
            let initial_filled = buf.filled().len();

            // If there is more space in the buffer than our limit allows, we have to shorten it.
            let (short_buf, used_buf) = if buf.remaining() as u64 > self.limit {
                let limit = self.limit as usize;

                let shortened = &mut unsafe { buf.all_mut() }[..initial_filled + limit];
                let mut short_buf = ReadBuf::uninit(unsafe { extend_lifetime_mut(shortened) });
                unsafe {
                    short_buf
                        .assume_init(std::cmp::min(limit, initial_initialized) - initial_filled)
                };
                short_buf.set_filled(initial_filled);

                let mut short_buf = AliasableBox::from_unique(Box::new(short_buf));
                let short_buf_mut = unsafe { extend_lifetime_mut(&mut *short_buf) };
                (Some(short_buf), short_buf_mut)
            } else {
                (None, unsafe { extend_lifetime_mut(&mut *buf) })
            };

            ReadTake {
                fut: Some(self.inner.read(used_buf.as_mut())),
                short_buf,
                _pinned: PhantomPinned,
                buf,
                initial_filled,
                limit: &mut self.limit,
            }
        }
    }
}

pin_project! {
    /// Future for [`read`](AsyncReadWith::read) on a [`Take`].
    pub struct ReadTake<'a, T: AsyncRead> {
        #[pin]
        fut: Option<<T as AsyncReadWith<'a>>::ReadFuture>,
        // The shortened buffer held by the future, `None` if `limit` is large enough that it isn't
        // necessary.
        short_buf: Option<AliasableBox<ReadBuf<'a>>>,
        // We want to support unboxing `short_buf` in the future.
        #[pin]
        _pinned: PhantomPinned,
        // The actual buffer passed to `read`. If `short_buf` is `None` the future uses this.
        buf: AliasableMut<'a, ReadBuf<'a>>,
        initial_filled: usize,
        limit: &'a mut u64,
    }
}

impl<T: AsyncRead> CompletionFuture for ReadTake<'_, T> {
    type Output = Result<()>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        if let Some(fut) = this.fut.as_mut().as_pin_mut() {
            ready!(fut.poll(cx))?;
            this.fut.set(None);

            // Sync the buffers if the future has written the other buffer.
            if let Some(short_buf) = this.short_buf.take() {
                let initialized = short_buf.initialized().len();
                let filled = short_buf.filled().len();
                drop(short_buf);

                this.buf.assume_init(initialized - *this.initial_filled);
                this.buf.set_filled(filled);
            }

            **this.limit -= (this.buf.filled().len() - *this.initial_filled) as u64;
        }

        Poll::Ready(Ok(()))
    }
}
impl<'a, T: AsyncRead> Future for ReadTake<'a, T>
where
    <T as AsyncReadWith<'a>>::ReadFuture: Future<Output = Result<()>>,
{
    type Output = Result<()>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

impl<'a, T: AsyncBufRead> AsyncBufReadWith<'a> for Take<T> {
    type FillBufFuture = FillBufTake<'a, T>;

    fn fill_buf(&'a mut self) -> Self::FillBufFuture {
        if self.limit == 0 {
            FillBufTake {
                fut: None,
                limit: 0,
            }
        } else {
            FillBufTake {
                fut: Some(self.inner.fill_buf()),
                limit: self.limit,
            }
        }
    }
    fn consume(&mut self, amt: usize) {
        // Don't let callers reset the limit by passing an overlarge value
        let amt = std::cmp::min(amt as u64, self.limit) as usize;
        self.limit -= amt as u64;
        self.inner.consume(amt);
    }
}

pin_project! {
    /// Future for [`fill_buf`](AsyncBufReadWith::fill_buf) on a [`Take`].
    pub struct FillBufTake<'a, T: AsyncBufRead> {
        #[pin]
        fut: Option<<T as AsyncBufReadWith<'a>>::FillBufFuture>,
        limit: u64,
    }
}

impl<'a, T: AsyncBufRead> CompletionFuture for FillBufTake<'a, T> {
    type Output = Result<&'a [u8]>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        Poll::Ready(Ok(match this.fut.as_pin_mut() {
            Some(fut) => {
                let buf = ready!(fut.poll(cx))?;
                let cap = std::cmp::min(buf.len() as u64, *this.limit) as usize;
                &buf[..cap]
            }
            None => &[],
        }))
    }
}
impl<'a, T: AsyncBufRead> Future for FillBufTake<'a, T>
where
    <T as AsyncBufReadWith<'a>>::FillBufFuture: Future<Output = Result<&'a [u8]>>,
{
    type Output = Result<&'a [u8]>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::mem::MaybeUninit;

    use crate::future::block_on;

    use super::super::{test_utils::YieldingReader, AsyncReadExt};

    #[test]
    fn reader_is_smaller() {
        let mut reader = YieldingReader::new(vec![Ok("Hello "), Ok("World!")]).take(13);

        let mut storage = [MaybeUninit::uninit(); 4];
        let mut buf = ReadBuf::uninit(&mut storage);
        block_on(reader.read(buf.as_mut())).unwrap();
        assert_eq!(buf.into_filled(), b"Hell");

        let mut storage = [0; 8];
        let mut buf = ReadBuf::new(&mut storage);

        block_on(reader.read(buf.as_mut())).unwrap();
        assert_eq!(buf.filled(), b"o ");

        buf.clear();
        block_on(reader.read(buf.as_mut())).unwrap();
        assert_eq!(buf.filled(), b"World!");

        buf.clear();
        block_on(reader.read(buf.as_mut())).unwrap();
        assert_eq!(buf.filled(), b"");
    }

    #[test]
    fn reader_is_larger() {
        let mut reader = YieldingReader::new(vec![Ok("Hello "), Ok("World!")]).take(8);

        let mut s = String::new();
        block_on(reader.read_to_string(&mut s)).unwrap();
        assert_eq!(s, "Hello Wo");
    }
}
