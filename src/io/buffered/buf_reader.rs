use std::future::Future;
use std::io::{IoSlice, Result};
use std::mem::{self, MaybeUninit};
use std::ops::Deref;
use std::pin::Pin;
use std::slice;
use std::task::{Context, Poll};

use super::extend_lifetime_mut;
use aliasable::AliasableMut;
use completion_core::CompletionFuture;
use completion_io::{
    AsyncBufReadWith, AsyncRead, AsyncReadWith, AsyncWriteWith, ReadBuf, ReadBufMut,
};
use futures_core::ready;
use pin_project_lite::pin_project;

/// Add buffering to any [reader](AsyncRead).
///
/// It can be excessively inefficient to work directly with an [`AsyncRead`] instance - often, every
/// single call to [`read`](AsyncReadWith::read) will result in a system call. A `BufReader`
/// performs large, infrequent reads on the underlying [`AsyncRead`] by maintaining an in-memory
/// buffer of the results.
///
/// It can improve the speed of programs that make small and repeated calls to the same I/O
/// resource, such as a file or network socket. It does not help when reading very large amounts at
/// once, or reading just one or a few times. It also provides no advantage when reading from a
/// source that is already in memory, such as a [`Vec`]`<`[`u8`]`>`.
///
/// When the `BufReader` is dropped, the contents of its buffer will be discarded. Creating
/// multiple instances of a `BufReader` on the same stream can cause data loss. Reading from the
/// underlying reader after unwrapping the `BufReader` with [`BufReader::into_inner`] can also
/// cause data loss.
#[derive(Debug)]
pub struct BufReader<R> {
    inner: R,
    buf: OwnedReadBuf,
    pos: usize,
}

impl<R> BufReader<R> {
    /// Create a new `BufReader` with a default buffer capacity. This is currently 8KB, but may
    /// change in the future.
    #[must_use]
    pub fn new(inner: R) -> Self {
        Self::with_capacity(8192, inner)
    }

    /// Create a new `BufReader` with the specified buffer capacity.
    #[must_use]
    pub fn with_capacity(capacity: usize, inner: R) -> Self {
        Self {
            inner,
            buf: OwnedReadBuf::with_capacity(capacity),
            pos: 0,
        }
    }

    /// Get a shared reference to the underlying reader.
    ///
    /// It is inadvisable to directly read from the underlying reader.
    #[must_use]
    pub fn get_ref(&self) -> &R {
        &self.inner
    }

    /// Get a mutable reference to the underlying reader.
    ///
    /// It is inadvisable to directly read from the underlying reader.
    #[must_use]
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.inner
    }

    /// Get a reference to the internally buffered data.
    ///
    /// Unlike [`fill_buf`](AsyncBufReadWith::fill_buf), this will not attempt to fill the buffer if
    /// it is empty.
    #[must_use]
    pub fn buffer(&self) -> &[u8] {
        &self.buf.filled()[self.pos..]
    }

    /// Get the number of bytes the internal buffer can hold at once.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.buf.capacity()
    }

    /// Unwraps this `BufReader`, returning the underlying reader.
    ///
    /// Note that any leftover data in the internal buffer is lost. Therefore, a following read from
    /// the underlying reader may lead to data loss.
    #[must_use]
    pub fn into_inner(self) -> R {
        self.inner
    }
}

impl<'a, R: AsyncRead> AsyncReadWith<'a> for BufReader<R> {
    type ReadFuture = ReadBufReader<'a, R>;

    fn read(&'a mut self, buf: ReadBufMut<'a>) -> Self::ReadFuture {
        // If there are no bytes in our buffer, and we're trying to read more bytes than the size
        // of the buffer, bypass the buffer entirely.
        let state = if self.pos == self.buf.filled().len() && buf.capacity() >= self.buf.capacity()
        {
            ReadBufReaderState::Bypass {
                fut: self.inner.read(buf),
            }
        } else {
            let mut reader = AliasableMut::from_unique(self);
            ReadBufReaderState::FillBuf {
                fut: unsafe { extend_lifetime_mut(&mut *reader) }.fill_buf(),
                buf,
                reader,
            }
        };
        ReadBufReader { state }
    }
}

pin_project! {
    /// Future for [`read`](AsyncReadWith::read) on a [`BufReader`].
    pub struct ReadBufReader<'a, R: AsyncRead> {
        #[pin]
        state: ReadBufReaderState<'a, R>,
    }
}
pin_project! {
    #[project = ReadBufReaderStateProj]
    enum ReadBufReaderState<'a, R: AsyncRead> {
        // We are bypassing the internal buffer and reading directly with the reader.
        Bypass {
            #[pin]
            fut: <R as AsyncReadWith<'a>>::ReadFuture,
        },
        // We are filling our buffer.
        FillBuf {
            #[pin]
            fut: FillBufBufReader<'a, R>,
            buf: ReadBufMut<'a>,
            reader: AliasableMut<'a, BufReader<R>>,
        }
    }
}

impl<R: AsyncRead> CompletionFuture for ReadBufReader<'_, R> {
    type Output = Result<()>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        match this.state.project() {
            ReadBufReaderStateProj::Bypass { fut } => fut.poll(cx),
            ReadBufReaderStateProj::FillBuf { fut, buf, reader } => {
                let amt = {
                    let available = ready!(fut.poll(cx))?;
                    let amt = std::cmp::min(buf.remaining(), available.len());
                    buf.append(&available[..amt]);
                    amt
                };
                reader.consume(amt);
                Poll::Ready(Ok(()))
            }
        }
    }
}
impl<'a, R: AsyncRead> Future for ReadBufReader<'a, R>
where
    <R as AsyncReadWith<'a>>::ReadFuture: Future<Output = Result<()>>,
{
    type Output = Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

impl<'a, R: AsyncRead> AsyncBufReadWith<'a> for BufReader<R> {
    type FillBufFuture = FillBufBufReader<'a, R>;

    fn fill_buf(&'a mut self) -> Self::FillBufFuture {
        let filled = self.buf.filled().len();
        let mut buf = AliasableMut::from_unique(&mut self.buf);

        FillBufBufReader {
            fut: if self.pos == filled {
                buf.get_mut().clear();
                self.pos = 0;
                let buf = unsafe { extend_lifetime_mut(&mut *buf) }.get_mut();
                Some(self.inner.read(buf))
            } else {
                None
            },
            buf: Some(buf),
            pos: self.pos,
        }
    }
    fn consume(&mut self, amt: usize) {
        self.pos = std::cmp::min(self.pos + amt, self.buf.capacity());
    }
}

pin_project! {
    /// Future for [`fill_buf`](AsyncBufReadWith::fill_buf) on a [`BufReader`].
    pub struct FillBufBufReader<'a, R: AsyncRead> {
        #[pin]
        fut: Option<<R as AsyncReadWith<'a>>::ReadFuture>,
        buf: Option<AliasableMut<'a, OwnedReadBuf>>,
        pos: usize,
    }
}

impl<'a, R: AsyncRead> CompletionFuture for FillBufBufReader<'a, R> {
    type Output = Result<&'a [u8]>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        if let Some(fut) = this.fut.as_mut().as_pin_mut() {
            ready!(fut.poll(cx))?;
            this.fut.set(None);
        }

        Poll::Ready(Ok(&AliasableMut::into_unique(
            this.buf.take().expect("polled after completion"),
        )
        .filled()[*this.pos..]))
    }
}
impl<'a, R: AsyncRead> Future for FillBufBufReader<'a, R>
where
    <R as AsyncReadWith<'a>>::ReadFuture: Future<Output = Result<()>>,
{
    type Output = Result<&'a [u8]>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

impl<'a, R: AsyncWriteWith<'a>> AsyncWriteWith<'a> for BufReader<R> {
    type WriteFuture = R::WriteFuture;
    type WriteVectoredFuture = R::WriteVectoredFuture;
    type FlushFuture = R::FlushFuture;

    fn write(&'a mut self, buf: &'a [u8]) -> Self::WriteFuture {
        self.inner.write(buf)
    }
    fn write_vectored(&'a mut self, bufs: &'a [IoSlice<'a>]) -> Self::WriteVectoredFuture {
        self.inner.write_vectored(bufs)
    }
    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
    fn flush(&'a mut self) -> Self::FlushFuture {
        self.inner.flush()
    }
}

/// Wrapper around a `ReadBuf` that owns it contents.
#[derive(Debug)]
struct OwnedReadBuf(ReadBuf<'static>);
impl OwnedReadBuf {
    fn with_capacity(capacity: usize) -> Self {
        let mut vec = vec![MaybeUninit::uninit(); capacity];
        vec.shrink_to_fit();
        let ptr = vec.as_mut_ptr();
        std::mem::forget(vec);

        let slice = unsafe { slice::from_raw_parts_mut(ptr, capacity) };
        Self(ReadBuf::uninit(slice))
    }

    fn get_mut(&mut self) -> ReadBufMut<'_> {
        self.0.as_mut()
    }
}
impl Deref for OwnedReadBuf {
    type Target = ReadBuf<'static>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl Drop for OwnedReadBuf {
    fn drop(&mut self) {
        let buf = mem::replace(&mut self.0, ReadBuf::uninit(&mut []));
        let capacity = buf.capacity();
        let ptr = buf.into_all().as_mut_ptr();

        unsafe { Vec::from_raw_parts(ptr, 0, capacity) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Cursor;

    use crate::future::block_on;

    use super::super::test_utils::YieldingReader;

    #[test]
    fn owned_read_buf() {
        let mut owned = OwnedReadBuf::with_capacity(294);
        assert_eq!(owned.capacity(), 294);
        assert_eq!(owned.initialized(), &[]);
        assert_eq!(owned.filled(), &[]);
        owned.get_mut().append(&[1, 2, 3, 4, 5]);
        assert_eq!(owned.filled(), &[1, 2, 3, 4, 5]);
        drop(owned);
    }

    #[test]
    fn fill_buf() {
        let mut cursor = Cursor::new(b"some data");
        let mut buffered = BufReader::new(&mut cursor);

        assert_eq!(buffered.buffer(), &[]);

        assert_eq!(block_on(buffered.fill_buf()).unwrap(), b"some data");
        assert_eq!(buffered.buffer(), b"some data");
        assert_eq!(cursor.position(), 9);
    }

    #[test]
    fn consume() {
        let mut buffered = BufReader::new(YieldingReader::new(vec![
            Ok(b"some data"),
            Ok(b"more data"),
        ]));

        assert_eq!(block_on(buffered.fill_buf()).unwrap(), b"some data");

        buffered.consume(1);
        assert_eq!(buffered.buffer(), b"ome data");
        assert_eq!(block_on(buffered.fill_buf()).unwrap(), b"ome data");
        assert_eq!(block_on(buffered.fill_buf()).unwrap(), b"ome data");

        buffered.consume(8);
        assert_eq!(buffered.buffer(), b"");

        assert_eq!(block_on(buffered.fill_buf()).unwrap(), b"more data");
        assert_eq!(block_on(buffered.fill_buf()).unwrap(), b"more data");
        assert_eq!(buffered.buffer(), b"more data");

        buffered.consume(9);
        assert_eq!(buffered.buffer(), b"");
        assert_eq!(block_on(buffered.fill_buf()).unwrap(), b"");
        assert_eq!(block_on(buffered.fill_buf()).unwrap(), b"");
    }

    #[test]
    fn capacity() {
        let mut buffered = BufReader::with_capacity(
            4,
            YieldingReader::new(vec![Ok(b"some data"), Ok(b"more data")]),
        );
        assert_eq!(buffered.capacity(), 4);

        let mut buffer = [0; 3];
        block_on(buffered.read(ReadBuf::new(&mut buffer).as_mut())).unwrap();
        assert_eq!(buffer, *b"som");

        let mut buffer = [0; 3];
        block_on(buffered.read(ReadBuf::new(&mut buffer).as_mut())).unwrap();
        assert_eq!(buffer, *b"e\0\0");

        let mut buffer = [0; 4];
        block_on(buffered.read(ReadBuf::new(&mut buffer).as_mut())).unwrap();
        assert_eq!(buffer, *b" dat");

        let mut buffer = [0; 3];
        block_on(buffered.read(ReadBuf::new(&mut buffer).as_mut())).unwrap();
        assert_eq!(buffer, *b"a\0\0");

        // Bypass
        let mut buffer = [0; 9];
        block_on(buffered.read(ReadBuf::new(&mut buffer).as_mut())).unwrap();
        assert_eq!(buffer, *b"more data");
    }
}
