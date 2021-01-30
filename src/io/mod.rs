//! Utilities for programming with asynchronous I/O.
//!
//! Requires the `std` feature.

pub use completion_io::*;

mod read;
pub use read::*;

mod buf_read;
pub use buf_read::*;

mod write;
pub use write::*;

mod seek;
pub use seek::*;

mod copy;
pub use copy::*;

mod buffered;
pub use buffered::*;

unsafe fn extend_lifetime_mut<'a, T: ?Sized>(r: &mut T) -> &'a mut T {
    &mut *(r as *mut _)
}
unsafe fn extend_lifetime<'a, T: ?Sized>(r: &T) -> &'a T {
    &*(r as *const _)
}

#[cfg(test)]
mod test_utils {
    use std::collections::VecDeque;
    use std::future::{self, Future};
    use std::io::{IoSlice, Result};
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use completion_core::CompletionFuture;
    use completion_io::{AsyncBufReadWith, AsyncReadWith, AsyncWriteWith, ReadBufMut};

    pub(crate) use crate::test_utils::*;

    #[derive(Debug)]
    pub(super) struct YieldingReader {
        items: VecDeque<Result<Vec<u8>>>,
    }
    impl YieldingReader {
        pub(super) fn new<I, S>(items: I) -> Self
        where
            I: IntoIterator<Item = Result<S>>,
            S: AsRef<[u8]>,
        {
            Self {
                items: items
                    .into_iter()
                    .map(|i| i.map(|s| s.as_ref().to_owned()))
                    .collect(),
            }
        }
    }
    impl<'a> AsyncReadWith<'a> for YieldingReader {
        type ReadFuture = Yield<ReadFuture<'a>>;

        fn read(&'a mut self, buf: ReadBufMut<'a>) -> Self::ReadFuture {
            Yield::once(ReadFuture { reader: self, buf })
        }
    }
    pub(super) struct ReadFuture<'a> {
        reader: &'a mut YieldingReader,
        buf: ReadBufMut<'a>,
    }
    impl CompletionFuture for ReadFuture<'_> {
        type Output = Result<()>;
        unsafe fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Ready(match self.reader.items.pop_front() {
                Some(Ok(bytes)) => {
                    let buf_remaining = self.buf.remaining();
                    if buf_remaining < bytes.len() {
                        self.buf.append(&bytes[..buf_remaining]);
                        self.reader
                            .items
                            .push_front(Ok(bytes[buf_remaining..].to_owned()));
                    } else {
                        self.buf.append(&bytes);
                    }
                    Ok(())
                }
                Some(Err(e)) => Err(e),
                None => Ok(()),
            })
        }
        unsafe fn poll_cancel(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
            Poll::Ready(())
        }
    }
    impl Future for ReadFuture<'_> {
        type Output = Result<()>;
        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            unsafe { CompletionFuture::poll(self, cx) }
        }
    }

    impl<'a> AsyncBufReadWith<'a> for YieldingReader {
        type FillBufFuture = Yield<FillBufFuture<'a>>;

        fn fill_buf(&'a mut self) -> Self::FillBufFuture {
            Yield::once(FillBufFuture { reader: Some(self) })
        }
        fn consume(&mut self, amt: usize) {
            if amt == 0 {
                return;
            }
            let slice = self.items.front_mut().unwrap().as_mut().unwrap();
            if amt == slice.len() {
                self.items.pop_front();
            } else {
                *slice = slice[amt..].to_owned();
            }
        }
    }
    pub(super) struct FillBufFuture<'a> {
        reader: Option<&'a mut YieldingReader>,
    }
    impl<'a> CompletionFuture for FillBufFuture<'a> {
        type Output = Result<&'a [u8]>;
        unsafe fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            let reader = self.reader.take().expect("polled after completion");
            Poll::Ready(match reader.items.pop_front() {
                Some(Ok(bytes)) => {
                    reader.items.push_front(Ok(bytes));
                    Ok(&**reader.items.front().unwrap().as_ref().unwrap())
                }
                Some(Err(e)) => Err(e),
                None => Ok(&[]),
            })
        }
        unsafe fn poll_cancel(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
            Poll::Ready(())
        }
    }

    #[derive(Debug)]
    pub(super) struct YieldingWriter {
        results: VecDeque<Result<usize>>,
        items: Vec<Vec<u8>>,
    }
    impl YieldingWriter {
        pub(super) fn new<I: IntoIterator<Item = Result<usize>>>(results: I) -> Self {
            Self {
                results: results.into_iter().collect(),
                items: Vec::new(),
            }
        }
        pub(super) fn into_items(self) -> Vec<Vec<u8>> {
            self.items
        }
    }
    impl<'a> AsyncWriteWith<'a> for YieldingWriter {
        type WriteFuture = Yield<WriteFuture<'a>>;
        type WriteVectoredFuture = completion_io::DefaultWriteVectored<'a, Self>;
        type FlushFuture = future::Ready<Result<()>>;

        fn write(&'a mut self, buf: &'a [u8]) -> Self::WriteFuture {
            assert!(!buf.is_empty(), "attempted to write an empty buffer");
            let result = self.results.pop_front().unwrap_or(Ok(0));
            Yield::once(WriteFuture {
                writer: self,
                buf,
                result: Some(result.map_err(|e| e)),
            })
        }
        fn write_vectored(&'a mut self, bufs: &'a [IoSlice<'a>]) -> Self::WriteVectoredFuture {
            completion_io::DefaultWriteVectored::new(self, bufs)
        }
        fn flush(&'a mut self) -> Self::FlushFuture {
            future::ready(Ok(()))
        }
    }
    pub(super) struct WriteFuture<'a> {
        writer: &'a mut YieldingWriter,
        buf: &'a [u8],
        result: Option<Result<usize>>,
    }
    impl CompletionFuture for WriteFuture<'_> {
        type Output = Result<usize>;

        unsafe fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            let this = &mut *self;
            let bytes = this.result.take().expect("polled after completion")?;
            let amt = std::cmp::min(bytes, this.buf.len());
            this.writer.items.push(this.buf[..amt].to_vec());
            Poll::Ready(Ok(amt))
        }
        unsafe fn poll_cancel(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
            Poll::Ready(())
        }
    }
}
