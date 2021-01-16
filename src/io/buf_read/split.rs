use std::future::Future;
use std::io::Result;
use std::marker::PhantomPinned;
use std::mem;
use std::pin::Pin;
use std::task::{Context, Poll};

use aliasable::boxed::AliasableBox;
use completion_core::{CompletionFuture, CompletionStream};
use completion_io::{AsyncBufRead, AsyncBufReadWith};
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

use super::{extend_lifetime_mut, AsyncBufReadExt, ReadUntil};

pin_project! {
    /// Stream for [`AsyncBufReadExt::split`](super::AsyncBufReadExt::split).
    ///
    /// The lifetime parameter of this type is an implementation detail, and it should be set to
    /// the lifetime of `R`. For example, if `R` is [`Empty`](completion_io::Empty) is should be
    /// `'static` and if `R` is [`Cursor<&'a [u8]>`](completion_io::Cursor) it should be `'a`.
    pub struct Split<'r, R: AsyncBufRead> {
        delim: u8,
        #[pin]
        fut: Option<ReadUntil<'r, R>>,
        reader: AliasableBox<R>,
        buf: AliasableBox<Vec<u8>>,
        // We want to support the above becoming unboxed in the future
        _pinned: PhantomPinned,
    }
}

impl<R: AsyncBufRead> Split<'_, R> {
    pub(super) fn new(reader: R, delim: u8) -> Self {
        Self {
            delim,
            fut: None,
            reader: AliasableBox::from_unique(Box::new(reader)),
            buf: AliasableBox::from_unique(Box::new(Vec::new())),
            _pinned: PhantomPinned,
        }
    }
}

impl<R: AsyncBufRead> CompletionStream for Split<'_, R> {
    type Item = Result<Vec<u8>>;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        if this.fut.as_mut().as_pin_mut().is_none() {
            let reader = extend_lifetime_mut(this.reader);
            let buf = extend_lifetime_mut(this.buf);
            this.fut.set(Some(reader.read_until(*this.delim, buf)));
        }

        let res = ready!(this.fut.as_mut().as_pin_mut().unwrap().poll(cx));
        this.fut.set(None);
        let n = res?;

        let buf = &mut **this.buf;

        Poll::Ready(if n == 0 && buf.is_empty() {
            None
        } else {
            if buf.last() == Some(this.delim) {
                buf.pop();
            }

            Some(Ok(mem::replace(buf, Vec::new())))
        })
    }
}
impl<'r, R: AsyncBufRead> Stream for Split<'r, R>
where
    <R as AsyncBufReadWith<'r>>::FillBufFuture: Future<Output = Result<&'r [u8]>>,
{
    type Item = Result<Vec<u8>>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
}

#[test]
fn test() {
    use std::io::{Error, ErrorKind};

    use crate::future::block_on;
    use crate::stream::CompletionStreamExt;

    use super::{test_utils::YieldingReader, AsyncBufReadExt};

    let parts = YieldingReader::new(vec![
        Ok(&[1, 2, 3, 4, 5][..]),
        Err(Error::from(ErrorKind::Interrupted)),
        Ok(&[6, 1, 2, 3, 4, 5, 1]),
        Ok(&[2, 3, 4]),
        Err(Error::new(ErrorKind::Other, "Some error")),
        Ok(&[5, 6]),
    ])
    .split(1);

    futures_lite::pin!(parts);

    assert_eq!(block_on(parts.next()).unwrap().unwrap(), []);
    assert_eq!(block_on(parts.next()).unwrap().unwrap(), [2, 3, 4, 5, 6]);
    assert_eq!(block_on(parts.next()).unwrap().unwrap(), [2, 3, 4, 5]);
    assert_eq!(
        block_on(parts.next()).unwrap().unwrap_err().to_string(),
        "Some error"
    );
    assert_eq!(block_on(parts.next()).unwrap().unwrap(), [2, 3, 4, 5, 6]);
    assert!(block_on(parts.next()).is_none());
}
