use std::future::Future;
use std::io::{Error, ErrorKind, Result};
use std::pin::Pin;
use std::task::{Context, Poll};

use aliasable::AliasableMut;
use completion_core::CompletionFuture;
use completion_io::{AsyncWrite, AsyncWriteWith};
use futures_core::ready;
use pin_project_lite::pin_project;

use super::extend_lifetime_mut;

pin_project! {
    /// Future for [`AsyncWriteExt::write_all`](super::AsyncWriteExt::write_all).
    pub struct WriteAll<'a, T>
    where
        T: AsyncWrite,
        T: ?Sized,
    {
        #[pin]
        fut: Option<<T as AsyncWriteWith<'a>>::WriteFuture>,
        writer: AliasableMut<'a, T>,
        buf: &'a [u8],
    }
}

impl<'a, T: AsyncWrite + ?Sized> WriteAll<'a, T> {
    pub(super) fn new(writer: &'a mut T, buf: &'a [u8]) -> Self {
        Self {
            fut: None,
            writer: AliasableMut::from_unique(writer),
            buf,
        }
    }
}

impl<'a, T: AsyncWrite + ?Sized + 'a> CompletionFuture for WriteAll<'a, T> {
    type Output = Result<()>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        loop {
            if let Some(fut) = this.fut.as_mut().as_pin_mut() {
                let res = ready!(fut.poll(cx));
                this.fut.set(None);

                match res {
                    Ok(0) => {
                        return Poll::Ready(Err(Error::new(
                            ErrorKind::WriteZero,
                            "failed to write whole buffer",
                        )));
                    }
                    Ok(bytes) => {
                        *this.buf = &this.buf[bytes..];
                    }
                    Err(e) if e.kind() == ErrorKind::Interrupted => {}
                    Err(e) => return Poll::Ready(Err(e)),
                }
            }

            if this.buf.is_empty() {
                return Poll::Ready(Ok(()));
            }

            let writer = extend_lifetime_mut(&mut **this.writer);
            this.fut.set(Some(writer.write(*this.buf)));
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
impl<'a, T: AsyncWrite + ?Sized + 'a> Future for WriteAll<'a, T>
where
    <T as AsyncWriteWith<'a>>::WriteFuture: Future<Output = Result<usize>>,
{
    type Output = Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::future::block_on;

    use super::super::{test_utils::YieldingWriter, AsyncWriteExt};

    #[test]
    fn empty() {
        let mut writer = YieldingWriter::new(vec![Ok(usize::MAX)]);
        block_on(writer.write_all(&[])).unwrap();
        assert_eq!(writer.into_items(), <Vec<Vec<u8>>>::new());
    }

    #[test]
    fn no_yield() {
        let mut writer = Vec::new();
        block_on(writer.write_all(&[1, 2, 3, 4, 5])).unwrap();
        assert_eq!(writer, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn yielding() {
        let mut writer = YieldingWriter::new(vec![Ok(2), Ok(3), Ok(1)]);
        block_on(writer.write_all(&[1, 2, 3, 4, 5, 6])).unwrap();
        assert_eq!(
            writer.into_items(),
            vec![vec![1, 2], vec![3, 4, 5], vec![6]]
        );
    }

    #[test]
    fn write_zero() {
        let mut writer = YieldingWriter::new(vec![Ok(1), Ok(2)]);
        assert_eq!(
            block_on(writer.write_all(&[1, 2, 3, 4]))
                .unwrap_err()
                .kind(),
            ErrorKind::WriteZero
        );
        assert_eq!(writer.into_items(), vec![vec![1], vec![2, 3], vec![]]);
    }

    #[test]
    fn interrupted() {
        let mut writer = YieldingWriter::new(vec![
            Ok(3),
            Err(Error::from(ErrorKind::Interrupted)),
            Err(Error::from(ErrorKind::Interrupted)),
            Ok(2),
            Ok(5),
            Err(Error::from(ErrorKind::Interrupted)),
        ]);
        block_on(writer.write_all(&[5; 10])).unwrap();
        assert_eq!(
            writer.into_items(),
            vec![vec![5, 5, 5], vec![5, 5], vec![5, 5, 5, 5, 5]]
        );
    }

    #[test]
    fn error() {
        let mut writer = YieldingWriter::new(vec![
            Ok(3),
            Err(Error::new(ErrorKind::Other, "Some error")),
            Ok(1),
        ]);
        assert_eq!(
            block_on(writer.write_all(&[1; 4])).unwrap_err().to_string(),
            "Some error"
        );
        assert_eq!(writer.into_items(), vec![vec![1, 1, 1]]);
    }
}
