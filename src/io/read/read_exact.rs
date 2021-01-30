use std::future::Future;
use std::io::{Error, ErrorKind, Result};
use std::pin::Pin;
use std::task::{Context, Poll};

use aliasable::AliasableMut;
use completion_core::CompletionFuture;
use completion_io::{AsyncRead, AsyncReadWith, ReadBuf, ReadBufMut};
use futures_core::ready;
use pin_project_lite::pin_project;

use super::extend_lifetime_mut;

pin_project! {
    /// Future for [`AsyncReadExt::read_exact`](super::AsyncReadExt::read_exact).
    pub struct ReadExact<'a, T>
    where
        T: AsyncRead,
        T: ?Sized,
    {
        #[pin]
        fut: Option<<T as AsyncReadWith<'a>>::ReadFuture>,
        reader: AliasableMut<'a, T>,
        buf: AliasableMut<'a, ReadBuf<'a>>,
        // The number of bytes filled at the start of the previous operation.
        previous_filled: usize,
    }
}

impl<'a, T: AsyncRead + ?Sized + 'a> ReadExact<'a, T> {
    pub(super) fn new(reader: &'a mut T, buf: ReadBufMut<'a>) -> Self {
        Self {
            fut: None,
            reader: AliasableMut::from_unique(reader),
            buf: AliasableMut::from(unsafe { buf.into_mut() }),
            previous_filled: 0,
        }
    }
}

impl<'a, T: AsyncRead + ?Sized + 'a> CompletionFuture for ReadExact<'a, T> {
    type Output = Result<()>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        loop {
            if let Some(fut) = this.fut.as_mut().as_pin_mut() {
                let res = ready!(fut.poll(cx));
                this.fut.set(None);

                match res {
                    Ok(()) => {
                        // There is no future, so we can create a mutable reference to `read_buf`
                        // without aliasing.
                        let read_buf = this.buf.as_mut();

                        if read_buf.filled().len() == *this.previous_filled {
                            return Poll::Ready(Err(Error::new(
                                ErrorKind::UnexpectedEof,
                                "failed to fill buffer",
                            )));
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::Interrupted => {}
                    Err(e) => return Poll::Ready(Err(e)),
                }
            }

            let read_buf = &mut **this.buf;

            if read_buf.remaining() == 0 {
                return Poll::Ready(Ok(()));
            }

            *this.previous_filled = read_buf.filled().len();

            let reader = extend_lifetime_mut(&mut **this.reader);
            let read_buf = extend_lifetime_mut(read_buf);
            this.fut.set(Some(reader.read(read_buf.as_mut())));
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
impl<'a, T: AsyncRead + ?Sized + 'a> Future for ReadExact<'a, T>
where
    <T as AsyncReadWith<'a>>::ReadFuture: Future<Output = Result<()>>,
{
    type Output = Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Cursor;
    use std::mem::MaybeUninit;

    use crate::future::block_on;

    use super::super::{test_utils::YieldingReader, AsyncReadExt};

    #[test]
    fn no_yield() {
        let mut cursor = Cursor::new([8; 50]);

        let mut storage = [MaybeUninit::uninit(); 10];
        let mut buffer = ReadBuf::uninit(&mut storage);
        block_on(cursor.read_exact(buffer.as_mut())).unwrap();

        assert_eq!(buffer.into_filled(), [8; 10]);
        assert_eq!(cursor.position(), 10);
    }

    #[test]
    fn yielding() {
        let mut reader = YieldingReader::new((0..100).map(|_| Ok([5, 6, 7])));

        let mut storage = [MaybeUninit::uninit(); 10];
        let mut buffer = ReadBuf::uninit(&mut storage);
        block_on(reader.read_exact(buffer.as_mut())).unwrap();

        assert_eq!(buffer.into_filled(), [5, 6, 7, 5, 6, 7, 5, 6, 7, 5]);
    }

    #[test]
    fn eof() {
        let mut reader = YieldingReader::new(vec![
            Err(Error::from(ErrorKind::Interrupted)),
            Ok(&[0, 1, 2][..]),
            Err(Error::from(ErrorKind::Interrupted)),
            Ok(&[3, 4]),
            Err(Error::from(ErrorKind::Interrupted)),
            Err(Error::from(ErrorKind::Interrupted)),
        ]);

        let mut storage = [MaybeUninit::uninit(); 10];
        let mut buffer = ReadBuf::uninit(&mut storage);
        assert_eq!(
            block_on(reader.read_exact(buffer.as_mut()))
                .unwrap_err()
                .kind(),
            ErrorKind::UnexpectedEof
        );

        assert_eq!(buffer.into_filled(), [0, 1, 2, 3, 4]);
    }
}
