//! `copy` and `copy_buf`.

use std::future::Future;
use std::io::{Error, ErrorKind, Result};
use std::marker::PhantomPinned;
use std::mem::MaybeUninit;
use std::pin::Pin;
use std::task::{Context, Poll};

use aliasable::{boxed::AliasableBox, AliasableMut};
use completion_core::CompletionFuture;
use completion_io::{
    AsyncBufRead, AsyncBufReadWith, AsyncRead, AsyncReadWith, AsyncWrite, AsyncWriteWith, ReadBuf,
};
use futures_core::ready;
use pin_project_lite::pin_project;

use super::{extend_lifetime, extend_lifetime_mut, AsyncWriteExt, WriteAll};

const COPY_BUF_SIZE: usize = 8192;

/// Copy the entire contents of a reader into a writer.
///
/// This function will continuously read data from `reader` and then write it to `writer` in a
/// streaming fashion until `reader` returns EOF, `writer` cannot receive any more bytes or a
/// non-[`ErrorKind::Interrupted`] error occurs in either.
///
/// On success, the total number of bytes that were copied from `reader` to `writer` is returned.
///
/// If you have a buffered reader, use [`copy_buf`] instead.
///
/// # Errors
///
/// This function will return an error immediately if any call to [`read`](AsyncReadWith::read) or
/// [`write`](AsyncWriteWith::write) returns an error. All instances of [`ErrorKind::Interrupted`]
/// are handled by this function and the underlying operation is retried.
///
/// # Examples
///
/// ```
/// # completion::future::block_on(completion::completion_async! {
/// let mut reader: &[u8] = b"Lorem ipsum dolor sit amet";
/// let mut writer = Vec::new();
///
/// completion::io::copy(&mut reader, &mut writer).await?;
///
/// assert_eq!(writer, b"Lorem ipsum dolor sit amet");
/// # completion_io::Result::Ok(())
/// # }).unwrap();
/// ```
pub fn copy<'a, R: AsyncRead + ?Sized, W: AsyncWrite + ?Sized>(
    reader: &'a mut R,
    writer: &'a mut W,
) -> Copy<'a, R, W> {
    Copy {
        state: CopyState::PreRead,
        reader: AliasableMut::from_unique(reader),
        writer: AliasableMut::from_unique(writer),
        buf: AliasableBox::from_unique(Box::new(None)),
        buf_storage: AliasableBox::from_unique(Box::new([MaybeUninit::uninit(); COPY_BUF_SIZE])),
        written: 0,
        _pinned: PhantomPinned,
    }
}

pin_project! {
    /// Future for [`copy`].
    pub struct Copy<'a, R: ?Sized, W: ?Sized>
    where
        R: AsyncRead,
        W: AsyncWrite,
    {
        #[pin]
        state: CopyState<'a, R, W>,

        reader: AliasableMut<'a, R>,
        writer: AliasableMut<'a, W>,

        buf: AliasableBox<Option<ReadBuf<'static>>>,
        buf_storage: AliasableBox<[MaybeUninit<u8>; COPY_BUF_SIZE]>,

        // The number of bytes written
        written: u64,

        // We want to support storing `buf` inline in the future.
        #[pin]
        _pinned: PhantomPinned,
    }
}

pin_project! {
    #[project = CopyStateProj]
    enum CopyState<'a, R: ?Sized, W: ?Sized>
    where
        R: AsyncRead,
        W: AsyncWrite,
    {
        PreRead,
        Reading {
            #[pin]
            fut: <R as AsyncReadWith<'a>>::ReadFuture,
        },
        Writing {
            #[pin]
            fut: WriteAll<'a, W>,
            bytes: u64,
        },
    }
}

impl<'a, R, W> CompletionFuture for Copy<'a, R, W>
where
    R: 'a + ?Sized + AsyncRead,
    W: 'a + ?Sized + AsyncWrite,
{
    type Output = Result<u64>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        loop {
            let state = this.state.as_mut().project();
            match state {
                CopyStateProj::PreRead => {
                    let buf_storage = &mut *this.buf_storage;
                    let buf = this.buf.get_or_insert_with(|| {
                        ReadBuf::uninit(extend_lifetime_mut(&mut **buf_storage))
                    });
                    buf.clear();

                    let reader = extend_lifetime_mut(&mut **this.reader);
                    let buf = extend_lifetime_mut(buf);
                    this.state.set(CopyState::Reading {
                        fut: reader.read(buf.as_mut()),
                    });
                }
                CopyStateProj::Reading { fut } => {
                    let res = ready!(fut.poll(cx));
                    // Temporarily store `PreRead` so we can safely use data the future currently
                    // holds a mutable reference to.
                    this.state.set(CopyState::PreRead);

                    match res {
                        Ok(()) => {}
                        Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                        Err(e) => return Poll::Ready(Err(e)),
                    }

                    let buf = (**this.buf).as_mut().unwrap();

                    let bytes = buf.filled().len();
                    if bytes == 0 {
                        return Poll::Ready(Ok(*this.written));
                    }

                    let writer = extend_lifetime_mut(&mut **this.writer);
                    let buf = extend_lifetime(buf.filled());
                    this.state.set(CopyState::Writing {
                        fut: writer.write_all(buf),
                        bytes: bytes as u64,
                    });
                }
                CopyStateProj::Writing { fut, bytes } => {
                    ready!(fut.poll(cx))?;
                    *this.written += *bytes;
                    this.state.set(CopyState::PreRead);
                }
            }
        }
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        match self.project().state.project() {
            CopyStateProj::PreRead => Poll::Ready(()),
            CopyStateProj::Reading { fut } => fut.poll_cancel(cx),
            CopyStateProj::Writing { fut, .. } => fut.poll_cancel(cx),
        }
    }
}
impl<'a, R, W> Future for Copy<'a, R, W>
where
    R: 'a + ?Sized + AsyncRead,
    W: 'a + ?Sized + AsyncWrite,
    <R as AsyncReadWith<'a>>::ReadFuture: Future<Output = Result<()>>,
    <W as AsyncWriteWith<'a>>::WriteFuture: Future<Output = Result<u64>>,
{
    type Output = Result<u64>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

/// Copy the entire contents of a buffered reader into a writer.
///
/// When you have a buffered reader, than is more efficient than calling [`copy`]. See [`copy`] for
/// more details about the semantics and errors of this function.
///
/// Unlike [`copy`], this [`flush`](AsyncWriteWith::flush)es the reader once it is complete.
///
/// # Examples
///
/// ```
/// # completion::future::block_on(completion::completion_async! {
/// let mut reader: &[u8] = b"Lorem ipsum dolor sit amet";
/// let mut writer = Vec::new();
///
/// completion::io::copy_buf(&mut reader, &mut writer).await?;
///
/// assert_eq!(writer, b"Lorem ipsum dolor sit amet");
/// # completion_io::Result::Ok(())
/// # }).unwrap();
/// ```
pub fn copy_buf<'a, R: AsyncBufRead + ?Sized, W: AsyncWrite + ?Sized>(
    reader: &'a mut R,
    writer: &'a mut W,
) -> CopyBuf<'a, R, W> {
    CopyBuf {
        state: CopyBufState::PreRead,
        reader: AliasableMut::from_unique(reader),
        writer: AliasableMut::from_unique(writer),
        written: 0,
    }
}

pin_project! {
    /// Future for [`copy_buf`].
    pub struct CopyBuf<'a, R: ?Sized, W: ?Sized>
    where
        R: AsyncBufRead,
        W: AsyncWrite,
    {
        #[pin]
        state: CopyBufState<'a, R, W>,

        reader: AliasableMut<'a, R>,
        writer: AliasableMut<'a, W>,

        written: u64,
    }
}

pin_project! {
    #[project = CopyBufStateProj]
    #[project_replace = CopyBufStateProjReplace]
    enum CopyBufState<'a,  R: ?Sized, W: ?Sized>
    where
        R: AsyncBufRead,
        W: AsyncWrite,
    {
        PreRead,
        Reading {
            #[pin]
            fut: <R as AsyncBufReadWith<'a>>::FillBufFuture,
        },
        Writing {
            #[pin]
            fut: <W as AsyncWriteWith<'a>>::WriteFuture,
            buf: &'a [u8],
        },
        Flushing {
            #[pin]
            fut: <W as AsyncWriteWith<'a>>::FlushFuture,
        }
    }
}

impl<'a, R, W> CompletionFuture for CopyBuf<'a, R, W>
where
    R: 'a + ?Sized + AsyncBufRead,
    W: 'a + ?Sized + AsyncWrite,
{
    type Output = Result<u64>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        loop {
            let state = this.state.as_mut().project();
            match state {
                CopyBufStateProj::PreRead => {
                    let reader = extend_lifetime_mut(&mut **this.reader);
                    this.state.set(CopyBufState::Reading {
                        fut: reader.fill_buf(),
                    });
                }
                CopyBufStateProj::Reading { fut } => {
                    let res = ready!(fut.poll(cx));
                    // Temporarily store `PreRead` so we can safely use data the future currently
                    // holds a mutable reference to.
                    this.state.set(CopyBufState::PreRead);

                    let buf = match res {
                        Ok(buf) => buf,
                        Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                        Err(e) => return Poll::Ready(Err(e)),
                    };

                    let writer = extend_lifetime_mut(&mut **this.writer);
                    this.state.set(if buf.is_empty() {
                        CopyBufState::Flushing {
                            fut: writer.flush(),
                        }
                    } else {
                        CopyBufState::Writing {
                            fut: writer.write(buf),
                            buf,
                        }
                    });
                }
                CopyBufStateProj::Writing { fut, .. } => {
                    match ready!(fut.poll(cx)) {
                        Ok(0) => {
                            return Poll::Ready(Err(Error::new(
                                ErrorKind::WriteZero,
                                "failed to write whole buffer",
                            )))
                        }
                        Ok(bytes) => {
                            *this.written += bytes as u64;
                            this.state.set(CopyBufState::PreRead);
                            this.reader.consume(bytes);
                        }
                        Err(e) if e.kind() == ErrorKind::Interrupted => {
                            // Temporarily set the state to destroy the future so we can access the
                            // writer again.
                            let buf =
                                match this.state.as_mut().project_replace(CopyBufState::PreRead) {
                                    CopyBufStateProjReplace::Writing { buf, .. } => buf,
                                    _ => unreachable!(),
                                };

                            // Retry the writing operation.
                            let writer = extend_lifetime_mut(&mut **this.writer);
                            this.state.set(CopyBufState::Writing {
                                fut: writer.write(buf),
                                buf,
                            });
                        }
                        Err(e) => return Poll::Ready(Err(e)),
                    }
                }
                CopyBufStateProj::Flushing { fut } => {
                    ready!(fut.poll(cx))?;
                    return Poll::Ready(Ok(*this.written));
                }
            }
        }
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        match self.project().state.project() {
            CopyBufStateProj::PreRead => Poll::Ready(()),
            CopyBufStateProj::Reading { fut } => fut.poll_cancel(cx),
            CopyBufStateProj::Writing { fut, .. } => fut.poll_cancel(cx),
            CopyBufStateProj::Flushing { fut, .. } => fut.poll_cancel(cx),
        }
    }
}
impl<'a, R, W> Future for CopyBuf<'a, R, W>
where
    R: 'a + ?Sized + AsyncBufRead,
    W: 'a + ?Sized + AsyncWrite,
    <R as AsyncBufReadWith<'a>>::FillBufFuture: Future<Output = Result<&'a [u8]>>,
    <W as AsyncWriteWith<'a>>::WriteFuture: Future<Output = Result<u64>>,
    <W as AsyncWriteWith<'a>>::FlushFuture: Future<Output = Result<()>>,
{
    type Output = Result<u64>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Cursor;

    use crate::future::block_on;

    use super::super::test_utils::{YieldingReader, YieldingWriter};

    const DATA: &[u8] = &[
        0x09, 0xF9, 0x11, 0x02, 0x9D, 0x74, 0xE3, 0x5B, 0xD8, 0x41, 0x56, 0xC5, 0x63, 0x56, 0x88,
        0xC0,
    ];

    #[test]
    fn no_yield() {
        for &buf in &[false, true] {
            let mut reader = Cursor::new(DATA);
            let mut writer = Vec::new();
            let res = if buf {
                block_on(copy_buf(&mut reader, &mut writer))
            } else {
                block_on(copy(&mut reader, &mut writer))
            };
            assert_eq!(res.unwrap(), DATA.len() as u64);
            assert_eq!(writer, DATA);
        }
    }

    #[test]
    fn yielding() {
        for &buf in &[false, true] {
            //            Data: ****************
            // Reader segments: [ ][ ][ ][ ][ ]|
            // Writer segments: [ ][]|[ ]|[]||||

            let mut reader = YieldingReader::new(DATA.chunks(3).map(Ok));
            let mut writer =
                YieldingWriter::new([3, 2, 1, 3, 1, 2, 1, 1, 1, 1].iter().copied().map(Ok));

            let res = if buf {
                block_on(copy_buf(&mut reader, &mut writer))
            } else {
                block_on(copy(&mut reader, &mut writer))
            };
            assert_eq!(res.unwrap(), DATA.len() as u64);
            assert_eq!(
                writer.into_items(),
                vec![
                    vec![DATA[0], DATA[1], DATA[2]],
                    vec![DATA[3], DATA[4]],
                    vec![DATA[5]],
                    vec![DATA[6], DATA[7], DATA[8]],
                    vec![DATA[9]],
                    vec![DATA[10], DATA[11]],
                    vec![DATA[12]],
                    vec![DATA[13]],
                    vec![DATA[14]],
                    vec![DATA[15]],
                ]
            );
        }
    }

    #[test]
    fn interrupted() {
        for &buf in &[false, true] {
            let mut reader = YieldingReader::new(vec![
                Err(Error::from(ErrorKind::Interrupted)),
                Ok([1, 2, 3]),
                Err(Error::from(ErrorKind::Interrupted)),
                Err(Error::from(ErrorKind::Interrupted)),
                Ok([4, 5, 6]),
            ]);
            let mut writer = YieldingWriter::new(vec![
                Err(Error::from(ErrorKind::Interrupted)),
                Err(Error::from(ErrorKind::Interrupted)),
                Ok(1),
                Err(Error::from(ErrorKind::Interrupted)),
                Ok(3),
                Err(Error::from(ErrorKind::Interrupted)),
                Ok(3),
            ]);

            let res = if buf {
                block_on(copy_buf(&mut reader, &mut writer))
            } else {
                block_on(copy(&mut reader, &mut writer))
            };
            assert_eq!(res.unwrap(), 6);
            assert_eq!(
                writer.into_items(),
                vec![vec![1], vec![2, 3], vec![4, 5, 6],],
            );
        }
    }

    #[test]
    fn reader_error() {
        for &buf in &[false, true] {
            let mut reader = YieldingReader::new(vec![
                Ok([1, 2, 3]),
                Err(Error::new(ErrorKind::Other, "Some error")),
                Ok([4, 5, 6]),
            ]);
            let mut writer =
                YieldingWriter::new(vec![Ok(3), Err(Error::new(ErrorKind::Other, "Bad error"))]);

            let res = if buf {
                block_on(copy_buf(&mut reader, &mut writer))
            } else {
                block_on(copy(&mut reader, &mut writer))
            };
            assert_eq!(res.unwrap_err().to_string(), "Some error");
            assert_eq!(writer.into_items(), vec![vec![1, 2, 3]]);
        }
    }

    #[test]
    fn writer_error() {
        for &buf in &[false, true] {
            let mut reader = YieldingReader::new(vec![
                Ok([1, 2, 3]),
                Err(Error::new(ErrorKind::Other, "Bad error")),
            ]);
            let mut writer =
                YieldingWriter::new(vec![Ok(2), Err(Error::new(ErrorKind::Other, "Some error"))]);

            let res = if buf {
                block_on(copy_buf(&mut reader, &mut writer))
            } else {
                block_on(copy(&mut reader, &mut writer))
            };
            assert_eq!(res.unwrap_err().to_string(), "Some error");
            assert_eq!(writer.into_items(), vec![vec![1, 2]]);
        }
    }
}
