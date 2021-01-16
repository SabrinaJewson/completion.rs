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

use super::{extend_lifetime_mut, AsyncBufReadExt, ReadLine};

pin_project! {
    /// Stream for [`AsyncBufReadExt::lines`](super::AsyncBufReadExt::lines).
    ///
    /// The lifetime parameter of this type is an implementation detail, and it should be set to
    /// the lifetime of `R`. For example, if `R` is [`Empty`](completion_io::Empty) is should be
    /// `'static` and if `R` is [`Cursor<&'a [u8]>`](completion_io::Cursor) it should be `'a`.
    pub struct Lines<'r, R: AsyncBufRead> {
        #[pin]
        fut: Option<ReadLine<'r, R>>,
        reader: AliasableBox<R>,
        buf: AliasableBox<String>,
        // We want to support the above becoming unboxed in the future
        _pinned: PhantomPinned,
    }
}

impl<R: AsyncBufRead> Lines<'_, R> {
    pub(super) fn new(reader: R) -> Self {
        Self {
            fut: None,
            reader: AliasableBox::from_unique(Box::new(reader)),
            buf: AliasableBox::from_unique(Box::new(String::new())),
            _pinned: PhantomPinned,
        }
    }
}

impl<R: AsyncBufRead> CompletionStream for Lines<'_, R> {
    type Item = Result<String>;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        if this.fut.as_mut().as_pin_mut().is_none() {
            let reader = extend_lifetime_mut(this.reader);
            let buf = extend_lifetime_mut(this.buf);
            this.fut.set(Some(reader.read_line(buf)));
        }

        let res = ready!(this.fut.as_mut().as_pin_mut().unwrap().poll(cx));
        this.fut.set(None);
        let n = res?;

        let buf = &mut **this.buf;

        Poll::Ready(if n == 0 && buf.is_empty() {
            None
        } else {
            if buf.ends_with('\n') {
                buf.pop();

                if buf.ends_with('\r') {
                    buf.pop();
                }
            }

            Some(Ok(mem::replace(buf, String::new())))
        })
    }
}
impl<'r, R: AsyncBufRead> Stream for Lines<'r, R>
where
    <R as AsyncBufReadWith<'r>>::FillBufFuture: Future<Output = Result<&'r [u8]>>,
{
    type Item = Result<String>;

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

    let lines = YieldingReader::new(vec![
        Ok("\nfirst lin"),
        Err(Error::from(ErrorKind::Interrupted)),
        Ok("e\nsecond line\r\n"),
        Ok("ignored because of the error later"),
        Err(Error::new(ErrorKind::Other, "Some error")),
        Ok("third line"),
    ])
    .lines();

    futures_lite::pin!(lines);

    assert_eq!(block_on(lines.next()).unwrap().unwrap(), "");
    assert_eq!(block_on(lines.next()).unwrap().unwrap(), "first line");
    assert_eq!(block_on(lines.next()).unwrap().unwrap(), "second line");
    assert_eq!(
        block_on(lines.next()).unwrap().unwrap_err().to_string(),
        "Some error"
    );
    assert_eq!(block_on(lines.next()).unwrap().unwrap(), "third line");
    assert!(block_on(lines.next()).is_none());
}
