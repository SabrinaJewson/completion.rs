use std::future;
use std::io::{Cursor, Empty, Result};

use completion_core::CompletionFuture;

use crate::AsyncRead;

/// Read bytes from a source that has an internal buffer asynchronously.
///
/// This is an asynchronous version of [`std::io::BufRead`].
///
/// You should not implement this trait manually, instead implement [`AsyncBufReadWith`].
pub trait AsyncBufRead: for<'a> AsyncBufReadWith<'a> {}
impl<T: for<'a> AsyncBufReadWith<'a> + ?Sized> AsyncBufRead for T {}

/// Read bytes from a source that has an internal buffer asynchronously with a specific lifetime.
pub trait AsyncBufReadWith<'a>: AsyncRead {
    /// Future that returns the contents of the internal buffer.
    type FillBufFuture: CompletionFuture<Output = Result<&'a [u8]>>;

    /// Attempt to return the contents of the internal buffer, filling it with more data from the
    /// inner reader if it is empty.
    ///
    /// This function is a lower-level call. It needs to be paired with the [`consume`] method to
    /// function properly. When calling this method, none of the contents will be "read" in the
    /// sense that later calling read may return the same contents. As such, [`consume`] must be
    /// called with the number of bytes that are consumed from this buffer to ensure that the bytes
    /// are never returned twice.
    ///
    /// An empty buffer returned indicates that the stream has reached EOF.
    ///
    /// # Errors
    ///
    /// This function will return an I/O error if the underlying reader was read, but returned an
    /// error.
    ///
    /// [`consume`]: Self::consume
    fn fill_buf(&'a mut self) -> Self::FillBufFuture;

    /// Tell this buffer that `amt` bytes have been consumed from the buffer, and so should no
    /// longer be returned in calls to [`read`](crate::AsyncReadWith::read).
    ///
    /// This function is a lower-level call. It needs to be paired with the [`fill_buf`] method to
    /// function properly. This function does not perform any I/O, it simply informs this object
    /// that some amount of its buffer, returned from [`fill_buf`], has been consumed and should no
    /// longer be returned. As such, this function may do odd things if [`fill_buf`] isn't called
    /// before calling it.
    ///
    /// The `amt` must be `<=` the number of bytes in the buffer returned by [`fill_buf`].
    ///
    /// [`fill_buf`]: Self::fill_buf
    fn consume(&mut self, amt: usize);
}

impl<'a, R: AsyncBufReadWith<'a> + ?Sized> AsyncBufReadWith<'a> for &mut R {
    type FillBufFuture = R::FillBufFuture;

    #[inline]
    fn fill_buf(&'a mut self) -> Self::FillBufFuture {
        (**self).fill_buf()
    }
    #[inline]
    fn consume(&mut self, amt: usize) {
        (**self).consume(amt)
    }
}

impl<'a, R: AsyncBufReadWith<'a> + ?Sized> AsyncBufReadWith<'a> for Box<R> {
    type FillBufFuture = R::FillBufFuture;

    #[inline]
    fn fill_buf(&'a mut self) -> Self::FillBufFuture {
        (**self).fill_buf()
    }
    #[inline]
    fn consume(&mut self, amt: usize) {
        (**self).consume(amt)
    }
}

impl<'a> AsyncBufReadWith<'a> for Empty {
    type FillBufFuture = future::Ready<Result<&'a [u8]>>;

    #[inline]
    fn fill_buf(&'a mut self) -> Self::FillBufFuture {
        future::ready(Ok(&[]))
    }
    #[inline]
    fn consume(&mut self, _amt: usize) {}
}

impl<'a> AsyncBufReadWith<'a> for &[u8] {
    type FillBufFuture = future::Ready<Result<&'a [u8]>>;

    #[inline]
    fn fill_buf(&'a mut self) -> Self::FillBufFuture {
        future::ready(Ok(*self))
    }
    #[inline]
    fn consume(&mut self, amt: usize) {
        *self = &self[amt..];
    }
}

impl<'a, T: AsRef<[u8]>> AsyncBufReadWith<'a> for Cursor<T> {
    type FillBufFuture = future::Ready<Result<&'a [u8]>>;

    #[inline]
    fn fill_buf(&'a mut self) -> Self::FillBufFuture {
        future::ready(std::io::BufRead::fill_buf(self))
    }
    #[inline]
    fn consume(&mut self, amt: usize) {
        std::io::BufRead::consume(self, amt);
    }
}

#[cfg(test)]
#[allow(dead_code, clippy::extra_unused_lifetimes)]
fn test_impls_traits<'a>() {
    fn assert_impls<R: AsyncBufRead>() {}

    assert_impls::<Empty>();
    assert_impls::<&'a mut Empty>();
    assert_impls::<Box<Empty>>();
    assert_impls::<&'a mut Box<&'a mut Empty>>();

    assert_impls::<&'a [u8]>();
    assert_impls::<&'a mut &'a [u8]>();

    assert_impls::<Cursor<Vec<u8>>>();
    assert_impls::<Cursor<&'a [u8]>>();
    assert_impls::<&'a mut Cursor<&'a [u8]>>();
}
