use std::future::Future;
use std::io::{ErrorKind, Result};
use std::marker::PhantomPinned;
use std::mem::MaybeUninit;
use std::pin::Pin;
use std::slice;
use std::task::{Context, Poll};

use aliasable::AliasableMut;
use completion_core::CompletionFuture;
use completion_io::{AsyncRead, AsyncReadWith, ReadBuf};
use futures_core::ready;
use pin_project_lite::pin_project;

use super::extend_lifetime_mut;

pin_project! {
    /// Future for [`AsyncReadExt::read_to_end`](super::AsyncReadExt::read_to_end).
    pub struct ReadToEnd<'a, T>
    where
        T: AsyncRead,
        T: ?Sized,
    {
        // The current reading future.
        #[pin]
        fut: Option<<T as AsyncReadWith<'a>>::ReadFuture>,

        reader: AliasableMut<'a, T>,

        // The buffer the future reads to. It has to be boxed as the future also holds a reference
        // to it and Rust doesn't support shared locals. It holds a reference to the data in `buf`.
        read_buf: Box<Option<ReadBuf<'a>>>,

        // Although this type could in theory be `Unpin`, we want to be able to unbox `read_buf` in
        // the future without breaking changes.
        #[pin]
        _pinned: PhantomPinned,

        // The buffer that was passed into `read_to_end`.
        buf: &'a mut Vec<u8>,

        // The index in the buffer up to which it is initialized. This often will go beyond the
        // length of the buffer.
        initialized_to: usize,

        // The number of filled bytes at the start of the operation.
        initial_filled: usize,
    }
}

impl<'a, T: AsyncRead + ?Sized + 'a> ReadToEnd<'a, T> {
    pub(super) fn new(reader: &'a mut T, buf: &'a mut Vec<u8>) -> Self {
        let buf_len = buf.len();
        Self {
            fut: None,
            reader: AliasableMut::from_unique(reader),
            read_buf: Box::new(None),
            _pinned: PhantomPinned,
            buf,
            initialized_to: buf_len,
            initial_filled: buf_len,
        }
    }
}

impl<'a, T: AsyncRead + ?Sized + 'a> CompletionFuture for ReadToEnd<'a, T> {
    type Output = Result<usize>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        loop {
            if let Some(fut) = this.fut.as_mut().as_pin_mut() {
                let res = ready!(fut.poll(cx));
                this.fut.set(None);

                // There is no future, so we can create a mutable reference to `read_buf` without
                // aliasing.
                let read_buf = this.read_buf.take().unwrap();

                match res {
                    Ok(()) => {
                        let filled = read_buf.filled().len();
                        let initialized = read_buf.initialized().len();

                        drop(read_buf);

                        // No bytes were written to the buffer; we have reached EOF.
                        if filled == 0 {
                            return Poll::Ready(Ok(this.buf.len() - *this.initial_filled));
                        }

                        this.buf.set_len(this.buf.len() + filled);
                        *this.initialized_to = this.buf.len() + initialized;
                    }
                    Err(e) if e.kind() == ErrorKind::Interrupted => {}
                    Err(e) => return Poll::Ready(Err(e)),
                }
            }

            this.buf.reserve(32);

            // Set up the read buffer.
            **this.read_buf = Some(ReadBuf::uninit(slice::from_raw_parts_mut(
                this.buf.as_mut_ptr().add(this.buf.len()) as *mut MaybeUninit<u8>,
                this.buf.capacity() - this.buf.len(),
            )));
            let read_buf = (**this.read_buf).as_mut().unwrap();
            read_buf.assume_init(*this.initialized_to - this.buf.len());

            // Set the reading future.
            let reader = extend_lifetime_mut(&mut **this.reader);
            let read_buf = extend_lifetime_mut(read_buf);
            this.fut.as_mut().set(Some(reader.read(read_buf.as_mut())));
        }
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if let Some(fut) = self.project().fut.as_pin_mut() {
            fut.poll_cancel(cx)
        } else {
            Poll::Ready(())
        }
    }
}
impl<'a, T: AsyncRead + ?Sized + 'a> Future for ReadToEnd<'a, T>
where
    <T as AsyncReadWith<'a>>::ReadFuture: Future<Output = Result<()>>,
{
    type Output = Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{Cursor, Error};

    use crate::future::block_on;

    use super::super::{
        test_utils::{poll_once, YieldingReader},
        AsyncReadExt,
    };

    #[test]
    fn no_yield() {
        let mut v = Vec::new();

        let mut cursor = Cursor::new(&[1, 2, 3, 4, 5]);
        assert_eq!(block_on(cursor.read_to_end(&mut v)).unwrap(), 5);
        assert_eq!(v, &[1, 2, 3, 4, 5]);

        let mut cursor = Cursor::new(&[8; 500]);
        assert_eq!(block_on(cursor.read_to_end(&mut v)).unwrap(), 500);
        assert_eq!(v.len(), 505);
        assert!(v.starts_with(&[1, 2, 3, 4, 5]));
        for &n in &v[5..] {
            assert_eq!(n, 8);
        }
    }

    #[test]
    fn yielding() {
        const BYTES: usize = 13;

        let mut v = Vec::new();

        let mut reader = YieldingReader::new((0..BYTES).map(|_| Ok([18_u8])));
        assert_eq!(block_on(reader.read_to_end(&mut v)).unwrap(), BYTES);
        assert_eq!(v, [18; BYTES]);
    }

    #[test]
    fn partial() {
        let mut v = Vec::new();

        let mut reader = YieldingReader::new((0..10).map(|_| [10, 11]).map(Ok));
        let fut = reader.read_to_end(&mut v);
        futures_lite::pin!(fut);
        assert!(poll_once(fut.as_mut()).is_none());
        assert!(poll_once(fut.as_mut()).is_none());
        assert_eq!(v, [10, 11]);
    }

    #[test]
    fn error() {
        let mut v = vec![1, 2, 3];

        let mut reader = YieldingReader::new(vec![
            Ok([4, 5]),
            Ok([6, 7]),
            Err(Error::new(ErrorKind::Other, "Some error")),
            Ok([8, 9]),
        ]);
        assert_eq!(
            block_on(reader.read_to_end(&mut v))
                .unwrap_err()
                .to_string(),
            "Some error"
        );
        assert_eq!(v, [1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn ignore_interrupted() {
        let mut v = vec![1, 2, 3];

        let mut reader = YieldingReader::new(vec![
            Err(Error::from(ErrorKind::Interrupted)),
            Ok(&[4, 5][..]),
            Err(Error::from(ErrorKind::Interrupted)),
            Err(Error::from(ErrorKind::Interrupted)),
            Ok(&[6]),
            Err(Error::from(ErrorKind::Interrupted)),
            Ok(&[7, 8]),
        ]);
        assert_eq!(block_on(reader.read_to_end(&mut v)).unwrap(), 5);
        assert_eq!(v, [1, 2, 3, 4, 5, 6, 7, 8]);
    }
}
