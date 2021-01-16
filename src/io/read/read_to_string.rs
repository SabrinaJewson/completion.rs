use std::future::Future;
use std::io::{Error, ErrorKind, Result};
use std::marker::PhantomPinned;
use std::mem;
use std::pin::Pin;
use std::str;
use std::task::{Context, Poll};

use aliasable::boxed::AliasableBox;
use completion_core::CompletionFuture;
use completion_io::{AsyncRead, AsyncReadWith};
use futures_core::ready;
use pin_project_lite::pin_project;

use super::{extend_lifetime_mut, AsyncReadExt, ReadToEnd};

pin_project! {
    /// Future for [`AsyncReadExt::read_to_string`].
    #[allow(clippy::box_vec)]
    pub struct ReadToString<'a, T>
    where
        T: AsyncRead,
        T: ?Sized,
    {
        reader: Option<&'a mut T>,

        #[pin]
        inner: Option<ReadToEnd<'a, T>>,

        // The vector the above future reads to. It has to be boxed as the future also holds a
        // reference to it and Rust doesn't support shared locals.
        buf: AliasableBox<Vec<u8>>,

        // We want to support `buf` being stored inline in the future.
        #[pin]
        _pinned: PhantomPinned,

        // The initial length of the above buffer.
        initial_len: usize,

        // The string that was passing into `read_to_end`. This is kept empty throughout the
        // duration of the operation, so we only have to do a UTF-8 check at the end.
        s: &'a mut String,
    }
}

impl<'a, T: AsyncRead + ?Sized + 'a> ReadToString<'a, T> {
    pub(super) fn new(reader: &'a mut T, buf: &'a mut String) -> Self {
        let len = buf.len();
        let buf_vec = AliasableBox::from_unique(Box::new(mem::take(buf).into_bytes()));
        Self {
            reader: Some(reader),
            inner: None,
            initial_len: len,
            buf: buf_vec,
            _pinned: PhantomPinned,
            s: buf,
        }
    }
}

impl<'a, T: AsyncRead + ?Sized + 'a> CompletionFuture for ReadToString<'a, T> {
    type Output = Result<usize>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        #[allow(clippy::option_if_let_else)]
        let inner = if let Some(inner) = this.inner.as_mut().as_pin_mut() {
            inner
        } else {
            let buf = extend_lifetime_mut(&mut **this.buf);

            let fut = this
                .reader
                .take()
                .expect("polled after completion")
                .read_to_end(buf);
            this.inner.set(Some(fut));
            this.inner.as_mut().as_pin_mut().unwrap()
        };

        let res = ready!(inner.poll(cx));
        this.inner.set(None);

        // The future is gone now, so we can safely create a mutable reference without aliasing.
        let buf = &mut **this.buf;
        let initial_len = *this.initial_len;

        let res = res.and_then(|bytes| {
            str::from_utf8(&buf[initial_len..])
                .map(|_| bytes)
                .map_err(|e| Error::new(ErrorKind::InvalidData, e))
        });

        if res.is_err() {
            buf.set_len(initial_len);
        }

        **this.s = String::from_utf8_unchecked(mem::take(buf));

        Poll::Ready(res)
    }
}
impl<'a, T: AsyncRead + ?Sized + 'a> Future for ReadToString<'a, T>
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

    use crate::future::block_on;

    use super::super::test_utils::YieldingReader;

    #[test]
    fn success() {
        let mut reader = YieldingReader::new(vec![Ok(" "), Ok("World"), Ok("!")]);

        let mut s = "Hello".to_owned();
        assert_eq!(block_on(reader.read_to_string(&mut s)).unwrap(), 7);
        assert_eq!(s, "Hello World!");
    }

    #[test]
    fn error() {
        let mut reader = YieldingReader::new(vec![
            Ok(" "),
            Err(Error::from(ErrorKind::Interrupted)),
            Ok("World"),
            Err(Error::new(ErrorKind::Other, "Some error")),
            Ok("!"),
        ]);

        let mut s = "Hello".to_owned();
        assert_eq!(
            block_on(reader.read_to_string(&mut s))
                .unwrap_err()
                .to_string(),
            "Some error"
        );
        assert_eq!(s, "Hello");
    }

    #[test]
    fn invalid_utf8() {
        let mut reader = YieldingReader::new(vec![Ok(" World".as_bytes()), Ok(&[0xC0])]);

        let mut s = "Hello".to_owned();
        assert_eq!(
            block_on(reader.read_to_string(&mut s)).unwrap_err().kind(),
            ErrorKind::InvalidData,
        );
        assert_eq!(s, "Hello");
    }
}
