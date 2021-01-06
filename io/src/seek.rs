use std::future::Future;
use std::io::{Cursor, Result, SeekFrom};
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use completion_core::CompletionFuture;

/// A cursor which can be moved within a stream of bytes.
///
/// This is an asynchronous version of [`std::io::Seek`].
///
/// You should not implement this trait manually, instead implement [`AsyncSeekWith`].
// https://github.com/rust-lang/rust/issues/55058
#[allow(single_use_lifetimes)]
pub trait AsyncSeek: for<'a> AsyncSeekWith<'a> {}

#[allow(single_use_lifetimes)]
impl<T: for<'a> AsyncSeekWith<'a> + ?Sized> AsyncSeek for T {}

/// A cursor which can be moved within a stream of bytes with a specific lifetime.
#[allow(single_use_lifetimes)]
pub trait AsyncSeekWith<'a> {
    /// Future that seeks to an offset a stream. If successful, resolves to the new position from
    /// the start of the stream.
    type SeekFuture: CompletionFuture<Output = Result<u64>>;

    /// Seek to an offset in bytes in a stream.
    fn seek(&'a mut self, pos: SeekFrom) -> Self::SeekFuture;
}

impl<'a, S: AsyncSeekWith<'a> + ?Sized> AsyncSeekWith<'a> for &mut S {
    type SeekFuture = S::SeekFuture;

    #[inline]
    fn seek(&'a mut self, pos: SeekFrom) -> Self::SeekFuture {
        (**self).seek(pos)
    }
}

impl<'a, S: AsyncSeekWith<'a> + ?Sized> AsyncSeekWith<'a> for Box<S> {
    type SeekFuture = S::SeekFuture;

    #[inline]
    fn seek(&'a mut self, pos: SeekFrom) -> Self::SeekFuture {
        (**self).seek(pos)
    }
}

impl<'a, T: AsRef<[u8]>> AsyncSeekWith<'a> for Cursor<T> {
    type SeekFuture = SeekCursorFuture<'a, T>;

    #[inline]
    fn seek(&'a mut self, pos: SeekFrom) -> Self::SeekFuture {
        SeekCursorFuture {
            cursor: self,
            pos,
            _lifetime: PhantomData,
        }
    }
}

/// Future produced when seeking in a [`Cursor`].
#[derive(Debug)]
pub struct SeekCursorFuture<'a, T> {
    // This is conceptually an &'a mut Cursor<T>. However, that would add the implicit bound T: 'a
    // which is incompatible with AsyncReadWith.
    cursor: *mut Cursor<T>,
    pos: SeekFrom,
    _lifetime: PhantomData<&'a ()>,
}
// SeekFrom is always Send+Sync, and we hold a mutable reference to Cursor.
unsafe impl<T: Send> Send for SeekCursorFuture<'_, T> {}
unsafe impl<T: Sync> Sync for SeekCursorFuture<'_, T> {}

impl<T: AsRef<[u8]>> Future for SeekCursorFuture<'_, T> {
    type Output = Result<u64>;

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        Poll::Ready(std::io::Seek::seek(unsafe { &mut *this.cursor }, this.pos))
    }
}
impl<T: AsRef<[u8]>> CompletionFuture for SeekCursorFuture<'_, T> {
    type Output = Result<u64>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Future::poll(self, cx)
    }
}

#[cfg(test)]
#[allow(dead_code, clippy::extra_unused_lifetimes)]
fn test_impls_traits<'a>() {
    fn assert_impls<R: AsyncSeek>() {}

    assert_impls::<Cursor<&'a [u8]>>();
    assert_impls::<Cursor<Vec<u8>>>();
    assert_impls::<&'a mut Cursor<&'a [u8]>>();
    assert_impls::<&'a mut Cursor<Vec<u8>>>();
    assert_impls::<Box<Cursor<&'a [u8]>>>();
    assert_impls::<Box<Cursor<Vec<u8>>>>();
}
