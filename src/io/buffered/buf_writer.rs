use std::future::Future;
use std::io::{Error, ErrorKind, IoSlice, Result};
use std::pin::Pin;
use std::task::{Context, Poll};

use aliasable::AliasableMut;
use completion_core::CompletionFuture;
use completion_io::{
    AsyncBufRead, AsyncBufReadWith, AsyncRead, AsyncReadWith, ReadBufRef, ReadBufsRef,
};
use completion_io::{AsyncWrite, AsyncWriteWith};
use futures_core::ready;
use pin_project_lite::pin_project;

use super::{extend_lifetime, extend_lifetime_mut};

/// Buffer the output of a [writer](AsyncWrite).
///
/// It can be excessively inefficient to work directly with an [`AsyncWrite`] instance - often,
/// every single call to [`write`](AsyncWriteWith::write) will result in a system call. A
/// `BufWriter` performs large, infrequent reads on the underlying [`AsyncWrite`] by maintaining an
/// in-memory buffer of the results.
///
/// It can improve the speed of programs that make small and repeated calls to the same I/O
/// resource, such as a file or network socket. It does not help when writing very large amounts at
/// once, or writing just one or a few times. It also provides no advantage when writing to a
/// destination that is already in memory, such as a [`Vec`]`<`[`u8`]`>`.
///
/// When the `BufWriter` if dropped, the contents of its buffer will be discarded. Creating multiple
/// instances of a `BufWriter` on the same stream can cause data loss. If you need to write out the
/// contents of its buffer, you must manually call [`flush`](AsyncWriteWith::flush) before the
/// writer is dropped.
///
/// # The static bound
///
/// The inner writer is currently required to live for `'static`. This is due to limitations with
/// Rust, and we can hopefully remove it in the future.
#[derive(Debug)]
pub struct BufWriter<W> {
    inner: W,
    // This should never reallocate.
    buf: Vec<u8>,
}

impl<W> BufWriter<W> {
    /// Create a new `BufWriter` with a default buffer capacity. The default is currently 8KB, but
    /// may change in the future.
    pub fn new(inner: W) -> Self {
        Self::with_capacity(8192, inner)
    }

    /// Create a new `BufWriter` with the specified buffer capacity.
    pub fn with_capacity(capacity: usize, inner: W) -> Self {
        Self {
            inner,
            buf: Vec::with_capacity(capacity),
        }
    }

    /// Get a shared reference to the underlying writer.
    ///
    /// It is inadvisable to directly write to the underlying writer.
    pub fn get_ref(&self) -> &W {
        &self.inner
    }

    /// Get a mutable reference to the underlying writer.
    ///
    /// It is inadvisable to directly write to the underlying writer.
    pub fn get_mut(&mut self) -> &mut W {
        &mut self.inner
    }

    /// Get a shared reference to the internally buffered data.
    pub fn buffer(&self) -> &[u8] {
        &self.buf
    }

    /// Get the number of bytes the internal buffer can hold without flushing.
    pub fn capacity(&self) -> usize {
        self.buf.capacity()
    }

    /// Unwraps this `BufWriter`, returning the underlying writer.
    ///
    /// Note that any leftover data in the internal buffer is lost.
    pub fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: AsyncWrite + 'static> BufWriter<W> {
    /// Write all the buffered data this `BufWriter` to the inner writer. This is like `write_all`,
    /// but in case the writer cannot write any more bytes or an error occurs it will remove any
    /// written bytes from the buffer and keep any unwritten ones.
    fn flush_buf(&mut self) -> FlushBuf<'_, W> {
        FlushBuf {
            fut: None,
            writer: Some(AliasableMut::from_unique(&mut self.inner)),
            buf: Some(AliasableMut::from_unique(&mut self.buf)),
            written: 0,
        }
    }
}
pin_project! {
    /// Future for `BufWriter::flush_buf`.
    struct FlushBuf<'a, W: AsyncWrite>
    where
        W: 'static,
    {
        #[pin]
        fut: Option<<W as AsyncWriteWith<'a>>::WriteFuture>,
        // `writer` and `buf` are only None when the future has completed.
        writer: Option<AliasableMut<'a, W>>,
        buf: Option<AliasableMut<'a, Vec<u8>>>,
        written: usize,
    }
}
impl<'a, W: AsyncWrite + 'static> CompletionFuture for FlushBuf<'a, W> {
    /// Return a mutable reference to the writer and internal buffer to make it easier to reuse
    /// those types.
    type Output = Result<(&'a mut W, &'a mut Vec<u8>)>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        let err;
        loop {
            if let Some(fut) = this.fut.as_mut().as_pin_mut() {
                let result = ready!(fut.poll(cx));
                this.fut.set(None);

                match result {
                    Ok(0) => {
                        err = Error::new(ErrorKind::WriteZero, "failed to write the buffered data");
                        break;
                    }
                    Ok(n) => {
                        *this.written += n;
                    }
                    Err(e) if e.kind() == ErrorKind::Interrupted => {}
                    Err(e) => {
                        err = e;
                        break;
                    }
                }
            }

            let writer = this.writer.as_deref_mut().expect("polled after completion");
            let buf = this.buf.as_deref_mut().expect("polled after completion");

            if *this.written >= buf.len() {
                let writer = AliasableMut::into_unique(this.writer.take().unwrap());
                let buf = AliasableMut::into_unique(this.buf.take().unwrap());
                buf.clear();
                return Poll::Ready(Ok((writer, buf)));
            }

            let writer = extend_lifetime_mut(writer);
            let buffer = extend_lifetime(&buf[*this.written..]);
            this.fut.set(Some(writer.write(buffer)));
        }

        if *this.written > 0 {
            this.buf
                .as_deref_mut()
                .expect("polled after completion")
                .drain(..*this.written);
        }

        Poll::Ready(Err(err))
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if let Some(fut) = self.project().fut.as_pin_mut() {
            fut.poll_cancel(cx)
        } else {
            Poll::Ready(())
        }
    }
}

impl<'a, W: AsyncWrite + 'static> AsyncWriteWith<'a> for BufWriter<W> {
    type WriteFuture = WriteBufWriter<'a, W>;
    type WriteVectoredFuture = WriteVectoredBufWriter<'a, W>;
    type FlushFuture = FlushBufWriter<'a, W>;

    fn write(&'a mut self, buf: &'a [u8]) -> Self::WriteFuture {
        // If there's no space for the data in our internal buffer, flush it.
        let state = if self.buf.len() + buf.len() > self.buf.capacity() {
            WriteState::FlushBuf {
                fut: self.flush_buf(),
            }
        } else {
            WriteState::Mid {
                writer: &mut self.inner,
                internal_buf: &mut self.buf,
            }
        };
        WriteBufWriter { state, buf }
    }
    fn write_vectored(&'a mut self, bufs: &'a [IoSlice<'a>]) -> Self::WriteVectoredFuture {
        let total_len: usize = bufs.iter().map(|b| b.len()).sum();
        WriteVectoredBufWriter {
            state: if self.buf.len() + total_len > self.buf.capacity() {
                WriteVState::FlushBuf {
                    fut: self.flush_buf(),
                }
            } else {
                WriteVState::Mid {
                    writer: &mut self.inner,
                    internal_buf: &mut self.buf,
                }
            },
            bufs,
            total_len,
        }
    }
    fn is_write_vectored(&self) -> bool {
        true
    }
    fn flush(&'a mut self) -> Self::FlushFuture {
        FlushBufWriter {
            buf: Some(self.flush_buf()),
            writer: None,
        }
    }
}

pin_project! {
    /// Future for [`write`](AsyncWriteWith::write) on a [`BufWriter`].
    pub struct WriteBufWriter<'a, W: AsyncWrite>
    where
        W: 'static,
    {
        #[pin]
        state: WriteState<'a, W>,
        buf: &'a [u8],
    }
}
pin_project! {
    #[project = WriteStateProj]
    #[project_replace = WriteStateProjReplace]
    enum WriteState<'a, W: AsyncWrite>
    where
        W: 'static,
    {
        FlushBuf {
            #[pin]
            fut: FlushBuf<'a, W>,
        },
        Mid {
            writer: &'a mut W,
            internal_buf: &'a mut Vec<u8>,
        },
        // We are trying to write a buffer whose size is larger than ours. Bypass our buffer.
        Bypass {
            #[pin]
            fut: <W as AsyncWriteWith<'a>>::WriteFuture,
        },
        Temporary,
    }
}

impl<W: AsyncWrite + 'static> CompletionFuture for WriteBufWriter<'_, W> {
    type Output = Result<usize>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        if let WriteStateProj::FlushBuf { fut, .. } = this.state.as_mut().project() {
            let (writer, internal_buf) = ready!(fut.poll(cx))?;
            this.state.set(WriteState::Mid {
                writer,
                internal_buf,
            });
        }
        if let WriteStateProj::Mid { .. } = this.state.as_mut().project() {
            let (writer, internal_buf) =
                match this.state.as_mut().project_replace(WriteState::Temporary) {
                    WriteStateProjReplace::Mid {
                        writer,
                        internal_buf,
                    } => (writer, internal_buf),
                    _ => unreachable!(),
                };

            // If the buffer is larger than our internal buffer, bypass our internal buffer
            // entirely.
            if this.buf.len() >= internal_buf.capacity() {
                this.state.set(WriteState::Bypass {
                    fut: writer.write(this.buf),
                });
            } else {
                internal_buf.extend_from_slice(this.buf);
                return Poll::Ready(Ok(this.buf.len()));
            }
        }
        match this.state.project() {
            WriteStateProj::Bypass { fut } => fut.poll(cx),
            WriteStateProj::Temporary => panic!("polled after completion"),
            _ => unreachable!(),
        }
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        match self.project().state.project() {
            WriteStateProj::FlushBuf { fut } => fut.poll_cancel(cx),
            WriteStateProj::Bypass { fut } => fut.poll_cancel(cx),
            _ => Poll::Ready(()),
        }
    }
}
impl<'a, W: AsyncWrite + 'static> Future for WriteBufWriter<'a, W>
where
    <W as AsyncWriteWith<'a>>::WriteFuture: Future<Output = Result<usize>>,
{
    type Output = Result<usize>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

pin_project! {
    /// Future for [`write_vectored`](AsyncWriteWith::write_vectored) on a [`BufWriter`].
    pub struct WriteVectoredBufWriter<'a, W: AsyncWrite>
    where
        W: 'static,
    {
        #[pin]
        state: WriteVState<'a, W>,
        bufs: &'a [IoSlice<'a>],
        // The combined length of all the buffers
        total_len: usize,
    }
}
pin_project! {
    #[project = WriteVStateProj]
    #[project_replace = WriteVStateProjReplace]
    enum WriteVState<'a, W: AsyncWrite>
    where
        W: 'static,
    {
        FlushBuf {
            #[pin]
            fut: FlushBuf<'a, W>,
        },
        Mid {
            writer: &'a mut W,
            internal_buf: &'a mut Vec<u8>,
        },
        // We are trying to write a buffer whose size is larger than ours. Bypass our buffer.
        Bypass {
            #[pin]
            fut: <W as AsyncWriteWith<'a>>::WriteVectoredFuture,
        },
        Temporary
    }
}

impl<W: AsyncWrite + 'static> CompletionFuture for WriteVectoredBufWriter<'_, W> {
    type Output = Result<usize>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        if let WriteVStateProj::FlushBuf { fut, .. } = this.state.as_mut().project() {
            let (writer, internal_buf) = ready!(fut.poll(cx))?;
            this.state.set(WriteVState::Mid {
                writer,
                internal_buf,
            });
        }
        if let WriteVStateProj::Mid { .. } = this.state.as_mut().project() {
            let (writer, internal_buf) =
                match this.state.as_mut().project_replace(WriteVState::Temporary) {
                    WriteVStateProjReplace::Mid {
                        writer,
                        internal_buf,
                    } => (writer, internal_buf),
                    _ => unreachable!(),
                };

            // If the writer is vectored and the total length of the buffers is larger than the
            // internal buffer, or the first nonempty buffer is larger than the internal buffer,
            // bypass the internal buffer. Otherwise we write it to the internal buffer.
            if writer.is_write_vectored() && *this.total_len >= internal_buf.capacity()
                || this
                    .bufs
                    .iter()
                    .find(|buf| !buf.is_empty())
                    .map_or(false, |buf| buf.len() >= internal_buf.capacity())
            {
                this.state.set(WriteVState::Bypass {
                    fut: writer.write_vectored(this.bufs),
                });
            } else {
                // If we know we can fit all the buffers in our internal buffer, simply append them
                // all. Otherwise it requires checking each time to make sure we won't overflow.
                let bytes = if *this.total_len <= internal_buf.capacity() {
                    for buf in *this.bufs {
                        internal_buf.extend_from_slice(buf);
                    }
                    *this.total_len
                } else {
                    let mut written = 0;
                    for buf in *this.bufs {
                        if internal_buf.len() + buf.len() > internal_buf.capacity() {
                            break;
                        }
                        internal_buf.extend_from_slice(buf);
                        written += buf.len();
                    }
                    written
                };
                return Poll::Ready(Ok(bytes));
            }
        }
        match this.state.project() {
            WriteVStateProj::Bypass { fut } => fut.poll(cx),
            WriteVStateProj::Temporary => panic!("polled after completion"),
            _ => unreachable!(),
        }
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        match self.project().state.project() {
            WriteVStateProj::FlushBuf { fut } => fut.poll_cancel(cx),
            WriteVStateProj::Bypass { fut } => fut.poll_cancel(cx),
            _ => Poll::Ready(()),
        }
    }
}
impl<'a, W: AsyncWrite + 'static> Future for WriteVectoredBufWriter<'a, W>
where
    <W as AsyncWriteWith<'a>>::WriteVectoredFuture: Future<Output = Result<usize>>,
{
    type Output = Result<usize>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

pin_project! {
    /// Future for [`flush`](AsyncWriteWith::flush) on a [`BufWriter`].
    pub struct FlushBufWriter<'a, W: AsyncWrite>
    where
        W: 'static,
    {
        // First we flush the buffer...
        #[pin]
        buf: Option<FlushBuf<'a, W>>,
        // Then we flush the writer.
        #[pin]
        writer: Option<<W as AsyncWriteWith<'a>>::FlushFuture>,

        // This could be an enum, but using options is easier.
    }
}

impl<W: AsyncWrite + 'static> CompletionFuture for FlushBufWriter<'_, W> {
    type Output = Result<()>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        if let Some(fut) = this.buf.as_mut().as_pin_mut() {
            let (writer, _) = ready!(fut.poll(cx))?;
            this.buf.set(None);
            this.writer.set(Some(writer.flush()));
        }
        this.writer.as_pin_mut().unwrap().poll(cx)
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let this = self.project();
        if let Some(buf) = this.buf.as_pin_mut() {
            buf.poll_cancel(cx)
        } else if let Some(writer) = this.writer.as_pin_mut() {
            writer.poll_cancel(cx)
        } else {
            unreachable!()
        }
    }
}
impl<'a, W: AsyncWrite + 'static> Future for FlushBufWriter<'a, W>
where
    <W as AsyncWriteWith<'a>>::WriteFuture: Future<Output = Result<usize>>,
    <W as AsyncWriteWith<'a>>::FlushFuture: Future<Output = Result<()>>,
{
    type Output = Result<()>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

impl<'a, 'b, W: AsyncRead> AsyncReadWith<'a, 'b> for BufWriter<W> {
    type ReadFuture = <W as AsyncReadWith<'a, 'b>>::ReadFuture;

    fn read(&'a mut self, buf: ReadBufRef<'a>) -> <W as AsyncReadWith<'a, 'b>>::ReadFuture {
        self.inner.read(buf)
    }

    type ReadVectoredFuture = <W as AsyncReadWith<'a, 'b>>::ReadVectoredFuture;

    fn read_vectored(&'a mut self, bufs: ReadBufsRef<'a, 'b>) -> Self::ReadVectoredFuture {
        self.inner.read_vectored(bufs)
    }

    fn is_read_vectored(&self) -> bool {
        self.inner.is_read_vectored()
    }
}

impl<'a, W: AsyncBufRead> AsyncBufReadWith<'a> for BufWriter<W> {
    type FillBufFuture = <W as AsyncBufReadWith<'a>>::FillBufFuture;

    fn fill_buf(&'a mut self) -> Self::FillBufFuture {
        self.inner.fill_buf()
    }
    fn consume(&mut self, amt: usize) {
        self.inner.consume(amt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::future::block_on;

    use super::super::test_utils::YieldingWriter;

    #[test]
    fn writing() {
        let mut buffered = BufWriter::with_capacity(50, YieldingWriter::new(vec![Ok(50)]));
        assert_eq!(buffered.capacity(), 50);

        assert_eq!(block_on(buffered.write(&[5; 40])).unwrap(), 40);
        assert_eq!(buffered.buffer(), &[5; 40]);

        assert_eq!(block_on(buffered.write(&[5; 3])).unwrap(), 3);
        assert_eq!(buffered.buffer(), &[5; 43]);

        assert_eq!(block_on(buffered.write(&[5; 7])).unwrap(), 7);
        assert_eq!(buffered.buffer(), &[5; 50]);

        assert_eq!(block_on(buffered.write(&[180])).unwrap(), 1);
        assert_eq!(buffered.buffer(), &[180]);

        assert_eq!(
            block_on(buffered.write(&[180; 52])).unwrap_err().kind(),
            ErrorKind::WriteZero
        );
        assert_eq!(buffered.buffer(), &[180]);

        assert_eq!(
            buffered.into_inner().into_items(),
            vec![vec![5; 50], vec![]],
        );
    }

    #[test]
    fn bypass() {
        let mut buffered = BufWriter::with_capacity(5, YieldingWriter::new(vec![Ok(3), Ok(18)]));

        assert_eq!(block_on(buffered.write(&[8; 3])).unwrap(), 3);
        assert_eq!(buffered.buffer(), &[8; 3]);

        assert_eq!(block_on(buffered.write(&[9; 20])).unwrap(), 18);
        assert_eq!(buffered.buffer(), &[]);

        assert_eq!(
            buffered.into_inner().into_items(),
            vec![vec![8; 3], vec![9; 18]]
        );
    }

    #[test]
    fn interrupted() {
        let mut buffered = BufWriter::new(YieldingWriter::new(vec![
            Err(Error::from(ErrorKind::Interrupted)),
            Ok(10),
            Err(Error::from(ErrorKind::Interrupted)),
            Err(Error::from(ErrorKind::Interrupted)),
            Err(Error::from(ErrorKind::Interrupted)),
            Ok(13),
            Err(Error::from(ErrorKind::Interrupted)),
        ]));

        assert_eq!(block_on(buffered.write(&[19; 5])).unwrap(), 5);
        assert_eq!(buffered.buffer(), &[19; 5]);
        assert_eq!(block_on(buffered.write(&[19; 18])).unwrap(), 18);
        assert_eq!(buffered.buffer(), &[19; 23]);

        block_on(buffered.flush()).unwrap();
        assert_eq!(buffered.buffer(), &[]);
        assert_eq!(
            buffered.into_inner().into_items(),
            vec![vec![19; 10], vec![19; 13]]
        );
    }
}
