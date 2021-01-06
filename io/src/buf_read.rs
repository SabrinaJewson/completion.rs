use std::future;
use std::io::{Cursor, Empty, Result};

use completion_core::CompletionFuture;

use crate::AsyncReadWith;

/// Read bytes from a source that has an internal buffer asynchronously.
///
/// This is an asynchronous version of [`std::io::BufRead`].
///
/// You should not implement this trait manually, instead implement [`AsyncBufReadWith`].
// https://github.com/rust-lang/rust/issues/55058
#[allow(single_use_lifetimes)]
pub trait AsyncBufRead: for<'a> AsyncBufReadWith<'a> {}

#[allow(single_use_lifetimes)]
impl<T: for<'a> AsyncBufReadWith<'a> + ?Sized> AsyncBufRead for T {}

/// Read bytes from a source that has an internal buffer asynchronously with a specific lifetime.
pub trait AsyncBufReadWith<'a>: AsyncReadWith<'a> {
    /// Future that returns the contents of the internal buffer.
    type FillBufFuture: CompletionFuture<Output = Result<&'a [u8]>>;

    /// Attempt to return the contents of the internal buffer, filling it with more data from the
    /// inner reader if it is empty.
    fn fill_buf(&'a mut self) -> Self::FillBufFuture;

    /// Tell this buffer that `amt` bytes have been consumed from the buffer, and so should no
    /// longer be returned in calls to [`read`](AsyncReadWith::read).
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
