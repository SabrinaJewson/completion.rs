use std::future::{self, Future};
use std::io::{IoSlice, Result, Sink};
use std::pin::Pin;
use std::task::{Context, Poll};

use completion_core::CompletionFuture;

/// Write bytes to a source asynchronously.
///
/// This is an async version of [`std::io::Write`].
///
/// You should not implement this trait manually, instead implement [`AsyncWriteWith`].
// https://github.com/rust-lang/rust/issues/55058
#[allow(single_use_lifetimes)]
pub trait AsyncWrite: for<'a> AsyncWriteWith<'a> {}

#[allow(single_use_lifetimes)]
impl<T: for<'a> AsyncWriteWith<'a> + ?Sized> AsyncWrite for T {}

/// Write bytes to a source asynchronously with a specific lifetime.
pub trait AsyncWriteWith<'a> {
    /// The future that writes to the source, and ouputs the number of bytes written.
    type WriteFuture: CompletionFuture<Output = Result<usize>>;

    /// The future that writes a vector of buffers to the source, and outputs the number of bytes
    /// written. If your writer does not have efficient vectored writes, set this to
    /// [`DefaultWriteVectored<'a, Self>`](DefaultWriteVectored).
    type WriteVectoredFuture: CompletionFuture<Output = Result<usize>>;

    /// The future that flushes the output stream.
    type FlushFuture: CompletionFuture<Output = Result<()>>;

    /// Write a buffer to the writer, returning how many bytes were written.
    fn write(&'a mut self, buf: &'a [u8]) -> Self::WriteFuture;

    /// Like [`write`](Self::write), except that it writes from a slice of buffers.
    ///
    /// Data is copied from each buffer in order, with the final buffer read from possibly being
    /// only partially consumed. This method must behave as a call to [`write`](Self::write) with
    /// the buffers concatenated would.
    ///
    /// If your writer does not have efficient vectored writes, call
    /// [`DefaultWriteVectored::new(self, bufs)`](DefaultWriteVectored::new).
    fn write_vectored(&'a mut self, bufs: &'a [IoSlice<'a>]) -> Self::WriteVectoredFuture;

    /// Determines if this `AsyncWrite`r has an efficient [`write_vectored`](Self::write_vectored)
    /// implementation.
    ///
    /// The default implementation returns `false`.
    fn is_write_vectored(&self) -> bool {
        false
    }

    /// Flush this output stream, ensuring that all intermediately buffered contents reach their
    /// destination.
    fn flush(&'a mut self) -> Self::FlushFuture;
}

impl<'a, W: AsyncWriteWith<'a> + ?Sized> AsyncWriteWith<'a> for &mut W {
    type WriteFuture = W::WriteFuture;
    type WriteVectoredFuture = W::WriteVectoredFuture;
    type FlushFuture = W::FlushFuture;

    #[inline]
    fn write(&'a mut self, buf: &'a [u8]) -> Self::WriteFuture {
        (**self).write(buf)
    }
    #[inline]
    fn write_vectored(&'a mut self, bufs: &'a [IoSlice<'a>]) -> Self::WriteVectoredFuture {
        (**self).write_vectored(bufs)
    }
    #[inline]
    fn is_write_vectored(&self) -> bool {
        (**self).is_write_vectored()
    }
    #[inline]
    fn flush(&'a mut self) -> Self::FlushFuture {
        (**self).flush()
    }
}

impl<'a, W: AsyncWriteWith<'a> + ?Sized> AsyncWriteWith<'a> for Box<W> {
    type WriteFuture = W::WriteFuture;
    type WriteVectoredFuture = W::WriteVectoredFuture;
    type FlushFuture = W::FlushFuture;

    #[inline]
    fn write(&'a mut self, buf: &'a [u8]) -> Self::WriteFuture {
        (**self).write(buf)
    }
    #[inline]
    fn write_vectored(&'a mut self, bufs: &'a [IoSlice<'a>]) -> Self::WriteVectoredFuture {
        (**self).write_vectored(bufs)
    }
    #[inline]
    fn is_write_vectored(&self) -> bool {
        (**self).is_write_vectored()
    }
    #[inline]
    fn flush(&'a mut self) -> Self::FlushFuture {
        (**self).flush()
    }
}

impl<'a> AsyncWriteWith<'a> for Sink {
    type WriteFuture = future::Ready<Result<usize>>;
    type WriteVectoredFuture = future::Ready<Result<usize>>;
    type FlushFuture = future::Ready<Result<()>>;

    #[inline]
    fn write(&'a mut self, buf: &'a [u8]) -> Self::WriteFuture {
        future::ready(Ok(buf.len()))
    }
    #[inline]
    fn write_vectored(&'a mut self, bufs: &'a [IoSlice<'a>]) -> Self::WriteVectoredFuture {
        future::ready(Ok(bufs.iter().map(|b| b.len()).sum()))
    }
    #[inline]
    fn is_write_vectored(&self) -> bool {
        true
    }
    #[inline]
    fn flush(&'a mut self) -> Self::FlushFuture {
        future::ready(Ok(()))
    }
}
impl<'a> AsyncWriteWith<'a> for &Sink {
    type WriteFuture = future::Ready<Result<usize>>;
    type WriteVectoredFuture = future::Ready<Result<usize>>;
    type FlushFuture = future::Ready<Result<()>>;

    #[inline]
    fn write(&'a mut self, buf: &'a [u8]) -> Self::WriteFuture {
        future::ready(Ok(buf.len()))
    }
    #[inline]
    fn write_vectored(&'a mut self, bufs: &'a [IoSlice<'a>]) -> Self::WriteVectoredFuture {
        future::ready(Ok(bufs.iter().map(|b| b.len()).sum()))
    }
    #[inline]
    fn is_write_vectored(&self) -> bool {
        true
    }
    #[inline]
    fn flush(&'a mut self) -> Self::FlushFuture {
        future::ready(Ok(()))
    }
}

impl<'a, 's> AsyncWriteWith<'a> for &'s mut [u8] {
    type WriteFuture = WriteSliceFuture<'a, 's>;
    type WriteVectoredFuture = WriteVectoredSliceFuture<'a, 's>;
    type FlushFuture = future::Ready<Result<()>>;

    #[inline]
    fn write(&'a mut self, buf: &'a [u8]) -> Self::WriteFuture {
        WriteSliceFuture {
            slice: unsafe { &mut *(self as *mut _) },
            buf,
        }
    }
    #[inline]
    fn write_vectored(&'a mut self, bufs: &'a [IoSlice<'a>]) -> Self::WriteVectoredFuture {
        WriteVectoredSliceFuture {
            slice: unsafe { &mut *(self as *mut _) },
            bufs,
        }
    }
    #[inline]
    fn is_write_vectored(&self) -> bool {
        true
    }
    #[inline]
    fn flush(&'a mut self) -> Self::FlushFuture {
        future::ready(Ok(()))
    }
}

/// Future produced when writing to a byte slice.
#[derive(Debug)]
pub struct WriteSliceFuture<'a, 's> {
    // This is conceptually an &'a mut &'s mut [u8]. However, that would add the implicit bound
    // 's: 'a which is incompatible with AsyncWriteWith.
    slice: &'s mut &'s mut [u8],
    buf: &'a [u8],
}
impl Future for WriteSliceFuture<'_, '_> {
    type Output = Result<usize>;

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        Poll::Ready(std::io::Write::write(this.slice, this.buf))
    }
}
impl CompletionFuture for WriteSliceFuture<'_, '_> {
    type Output = Result<usize>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Future::poll(self, cx)
    }
}

/// Future produced when writing to a byte slice with vectored operations.
#[derive(Debug)]
pub struct WriteVectoredSliceFuture<'a, 's> {
    // This is conceptually an &'a mut &'s mut [u8]. However, that would add the implicit bound
    // 's: 'a which is incompatible with AsyncWriteWith.
    slice: &'s mut &'s mut [u8],
    bufs: &'a [IoSlice<'a>],
}
impl Future for WriteVectoredSliceFuture<'_, '_> {
    type Output = Result<usize>;

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        Poll::Ready(std::io::Write::write_vectored(this.slice, this.bufs))
    }
}
impl CompletionFuture for WriteVectoredSliceFuture<'_, '_> {
    type Output = Result<usize>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Future::poll(self, cx)
    }
}

impl<'a> AsyncWriteWith<'a> for Vec<u8> {
    type WriteFuture = WriteVecFuture<'a>;
    type WriteVectoredFuture = WriteVectoredVecFuture<'a>;
    type FlushFuture = future::Ready<Result<()>>;

    #[inline]
    fn write(&'a mut self, buf: &'a [u8]) -> Self::WriteFuture {
        WriteVecFuture { vec: self, buf }
    }
    #[inline]
    fn write_vectored(&'a mut self, bufs: &'a [IoSlice<'a>]) -> Self::WriteVectoredFuture {
        WriteVectoredVecFuture { vec: self, bufs }
    }
    #[inline]
    fn is_write_vectored(&self) -> bool {
        true
    }
    #[inline]
    fn flush(&'a mut self) -> Self::FlushFuture {
        future::ready(Ok(()))
    }
}

/// Future produced when writing to a vector.
#[derive(Debug)]
pub struct WriteVecFuture<'a> {
    vec: &'a mut Vec<u8>,
    buf: &'a [u8],
}
impl Future for WriteVecFuture<'_> {
    type Output = Result<usize>;

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        Poll::Ready(std::io::Write::write(this.vec, this.buf))
    }
}
impl CompletionFuture for WriteVecFuture<'_> {
    type Output = Result<usize>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Future::poll(self, cx)
    }
}

/// Future produced when writing to a vector with vectored operations.
#[derive(Debug)]
pub struct WriteVectoredVecFuture<'a> {
    vec: &'a mut Vec<u8>,
    bufs: &'a [IoSlice<'a>],
}
impl Future for WriteVectoredVecFuture<'_> {
    type Output = Result<usize>;

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        Poll::Ready(std::io::Write::write_vectored(this.vec, this.bufs))
    }
}
impl CompletionFuture for WriteVectoredVecFuture<'_> {
    type Output = Result<usize>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Future::poll(self, cx)
    }
}

// TODO: implement AsyncWrite for:
// - Cursor<&mut [u8]>
// - Cursor<&mut Vec<u8>>
// - Cursor<Box<[u8]>>
// - Cursor<Vec<u8>>

#[cfg(test)]
#[allow(dead_code, clippy::extra_unused_lifetimes)]
fn test_impls_traits<'a>() {
    fn assert_impls<R: AsyncWrite>() {}

    assert_impls::<Sink>();
    assert_impls::<&'a mut Sink>();
    assert_impls::<Box<Sink>>();
    assert_impls::<&'a mut Box<&'a mut Sink>>();

    assert_impls::<&'a mut [u8]>();
    assert_impls::<&'a mut &'a mut [u8]>();

    assert_impls::<Vec<u8>>();
}

/// A default implementation of [`WriteVectoredFuture`](AsyncWriteWith::WriteVectoredFuture) for
/// types that don't have efficient vectored writes.
///
/// This will forward to [`write`](AsyncWriteWith::write) with the first nonempty buffer provided,
/// or an empty one if none exists.
#[derive(Debug)]
#[allow(single_use_lifetimes)]
pub struct DefaultWriteVectored<'a, T: AsyncWriteWith<'a>> {
    future: T::WriteFuture,
}

impl<'a, T: AsyncWriteWith<'a>> DefaultWriteVectored<'a, T> {
    /// Create a new `DefaultWriteVectored` future.
    pub fn new(writer: &'a mut T, bufs: &'a [IoSlice<'a>]) -> Self {
        Self {
            future: writer.write(bufs.iter().find(|b| !b.is_empty()).map_or(&[], |b| &**b)),
        }
    }
}

impl<'a, T: AsyncWriteWith<'a>> CompletionFuture for DefaultWriteVectored<'a, T> {
    type Output = Result<usize>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::map_unchecked_mut(self, |this| &mut this.future).poll(cx)
    }
}
impl<'a, T: AsyncWriteWith<'a>> Future for DefaultWriteVectored<'a, T>
where
    <T as AsyncWriteWith<'a>>::WriteFuture: Future<Output = Result<usize>>,
{
    type Output = Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}
