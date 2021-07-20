use std::future::Future;
use std::io::Result;
use std::pin::Pin;
use std::task::{Context, Poll};

use aliasable::AliasableMut;
use completion_core::CompletionFuture;
use completion_io::{
    AsyncBufRead, AsyncBufReadWith, AsyncRead, AsyncReadWith, ReadBuf, ReadBufRef, ReadBufs,
    ReadBufsRef,
};
use futures_core::ready;
use pin_project_lite::pin_project;

use super::extend_lifetime_mut;

/// Reader for [`AsyncReadExt::chain`](super::AsyncReadExt::chain).
#[derive(Debug)]
pub struct Chain<T, U> {
    first: T,
    second: U,
    done_first: bool,
}

impl<T, U> Chain<T, U> {
    pub(super) fn new(first: T, second: U) -> Self {
        Self {
            first,
            second,
            done_first: false,
        }
    }

    /// Consume the chain, returning the wrapped readers.
    pub fn into_inner(self) -> (T, U) {
        (self.first, self.second)
    }

    /// Get shared references to the underlying readers in the chain.
    pub fn get_ref(&self) -> (&T, &U) {
        (&self.first, &self.second)
    }

    /// Get mutable references to the underlying readers in the chain.
    ///
    /// Care should be taken to avoid modifying the internal I/O state of the underlying readers as
    /// doing so may corrupt the internal state of this chain.
    pub fn get_mut(&mut self) -> (&mut T, &mut U) {
        (&mut self.first, &mut self.second)
    }
}

impl<'a, 'b, T: AsyncRead, U: AsyncRead + 'static> AsyncReadWith<'a, 'b> for Chain<T, U> {
    type ReadFuture = ReadChain<'a, T, U>;

    fn read(&'a mut self, buf: ReadBufRef<'a>) -> Self::ReadFuture {
        let state = if self.done_first {
            ReadChainState::Second {
                fut: self.second.read(buf),
            }
        } else {
            let mut buf = AliasableMut::from_unique(unsafe { buf.into_mut() });
            ReadChainState::First {
                fut: self
                    .first
                    .read(unsafe { extend_lifetime_mut(&mut *buf) }.as_ref()),
                second: &mut self.second,
                initial_filled: buf.filled().len(),
                buf,
                done_first: &mut self.done_first,
            }
        };
        ReadChain { state }
    }

    type ReadVectoredFuture = ReadVectoredChain<'a, 'b, T, U>;

    fn read_vectored(&'a mut self, bufs: ReadBufsRef<'a, 'b>) -> Self::ReadVectoredFuture {
        let state = if self.done_first {
            ReadVChainState::Second {
                fut: self.second.read_vectored(bufs),
            }
        } else {
            let mut bufs = AliasableMut::from_unique(unsafe { bufs.into_mut() });
            ReadVChainState::First {
                fut: self
                    .first
                    .read_vectored(unsafe { extend_lifetime_mut(&mut *bufs) }.as_ref()),
                second: &mut self.second,
                initial_filled: (bufs.filled().0.len(), bufs.filled().1.len()),
                bufs,
                done_first: &mut self.done_first,
            }
        };
        ReadVectoredChain { state }
    }

    fn is_read_vectored(&self) -> bool {
        if self.done_first {
            self.second.is_read_vectored()
        } else {
            self.first.is_read_vectored() && self.second.is_read_vectored()
        }
    }
}

pin_project! {
    /// Future for [`read`](AsyncReadWith::read) on a [`Chain`].
    pub struct ReadChain<'a, T: AsyncRead, U: AsyncRead>
    where
        U: 'static,
    {
        #[pin]
        state: ReadChainState<'a, T, U>,
    }
}
pin_project! {
    #[project = ReadChainStateProj]
    #[project_replace = ReadChainStateProjReplace]
    enum ReadChainState<'a, T: AsyncRead, U: AsyncRead>
    where
        U: 'static,
    {
        First {
            #[pin]
            fut: <T as AsyncReadWith<'a, 'static>>::ReadFuture,
            second: &'a mut U,
            initial_filled: usize,
            buf: AliasableMut<'a, ReadBuf<'a>>,
            done_first: &'a mut bool,
        },
        Second {
            #[pin]
            fut: <U as AsyncReadWith<'a, 'static>>::ReadFuture,
        },
        Temporary,
    }
}

impl<T: AsyncRead, U: AsyncRead> CompletionFuture for ReadChain<'_, T, U> {
    type Output = Result<()>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        if let ReadChainStateProj::First { fut, .. } = this.state.as_mut().project() {
            ready!(fut.poll(cx))?;

            let (second, initial_filled, buf, done_first) = match this
                .state
                .as_mut()
                .project_replace(ReadChainState::Temporary)
            {
                ReadChainStateProjReplace::First {
                    second,
                    initial_filled,
                    buf,
                    done_first,
                    ..
                } => (second, initial_filled, buf, done_first),
                _ => unreachable!(),
            };
            let buf = AliasableMut::into_unique(buf).as_ref();

            if buf.filled().len() > initial_filled || buf.capacity() - initial_filled == 0 {
                return Poll::Ready(Ok(()));
            }

            *done_first = true;
            this.state.set(ReadChainState::Second {
                fut: second.read(buf),
            });
        }
        match this.state.project() {
            ReadChainStateProj::Second { fut } => fut.poll(cx),
            ReadChainStateProj::Temporary => panic!("polled after completion"),
            ReadChainStateProj::First { .. } => unreachable!(),
        }
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        match self.project().state.project() {
            ReadChainStateProj::First { fut, .. } => fut.poll_cancel(cx),
            ReadChainStateProj::Second { fut } => fut.poll_cancel(cx),
            ReadChainStateProj::Temporary => Poll::Ready(()),
        }
    }
}

impl<'a, T: AsyncRead, U: AsyncRead> Future for ReadChain<'_, T, U>
where
    <T as AsyncReadWith<'a, 'static>>::ReadFuture: Future<Output = Result<()>>,
    <U as AsyncReadWith<'a, 'static>>::ReadFuture: Future<Output = Result<()>>,
{
    type Output = Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

impl<'a, T: AsyncBufRead, U: AsyncBufRead + 'static> AsyncBufReadWith<'a> for Chain<T, U> {
    type FillBufFuture = FillBufChain<'a, T, U>;

    fn fill_buf(&'a mut self) -> Self::FillBufFuture {
        FillBufChain {
            state: if self.done_first {
                FillBufChainState::Second {
                    fut: self.second.fill_buf(),
                }
            } else {
                FillBufChainState::First {
                    fut: self.first.fill_buf(),
                    done_first: &mut self.done_first,
                    second: &mut self.second,
                }
            },
        }
    }
    fn consume(&mut self, amt: usize) {
        if self.done_first {
            self.second.consume(amt);
        } else {
            self.first.consume(amt);
        }
    }
}

pin_project! {
    /// Future for [`read_vectored`](AsyncReadWith::read_vectored) on a [`Chain`].
    pub struct ReadVectoredChain<'a, 'b, T: AsyncRead, U: AsyncRead>
    where
        U: 'static,
    {
        #[pin]
        state: ReadVChainState<'a, 'b, T, U>,
    }
}
pin_project! {
    #[project = ReadVChainStateProj]
    #[project_replace = ReadVChainStateProjReplace]
    enum ReadVChainState<'a, 'b, T: AsyncRead, U: AsyncRead>
    where
        U: 'static,
    {
        First {
            #[pin]
            fut: <T as AsyncReadWith<'a, 'b>>::ReadVectoredFuture,
            second: &'a mut U,
            initial_filled: (usize, usize),
            bufs: AliasableMut<'a, ReadBufs<'a, 'b>>,
            done_first: &'a mut bool,
        },
        Second {
            #[pin]
            fut: <U as AsyncReadWith<'a, 'b>>::ReadVectoredFuture,
        },
        Temporary
    }
}

impl<T: AsyncRead, U: AsyncRead> CompletionFuture for ReadVectoredChain<'_, '_, T, U> {
    type Output = Result<()>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        if let ReadVChainStateProj::First { fut, .. } = this.state.as_mut().project() {
            ready!(fut.poll(cx))?;

            let (second, initial_filled, bufs, done_first) = match this
                .state
                .as_mut()
                .project_replace(ReadVChainState::Temporary)
            {
                ReadVChainStateProjReplace::First {
                    second,
                    initial_filled,
                    bufs,
                    done_first,
                    ..
                } => (second, initial_filled, bufs, done_first),
                _ => unreachable!(),
            };
            let bufs = AliasableMut::into_unique(bufs).as_ref();

            let (filled_bufs, filled_buf) = bufs.filled();
            if (filled_bufs.len(), filled_buf.len()) > initial_filled
                || initial_filled.0 == bufs.inner().len()
            {
                return Poll::Ready(Ok(()));
            }

            *done_first = true;
            this.state.set(ReadVChainState::Second {
                fut: second.read_vectored(bufs),
            });
        }
        match this.state.project() {
            ReadVChainStateProj::Second { fut } => fut.poll(cx),
            ReadVChainStateProj::Temporary => panic!("Polled `ReadVectoredChain` after completion"),
            ReadVChainStateProj::First { .. } => unreachable!(),
        }
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        match self.project().state.project() {
            ReadVChainStateProj::First { fut, .. } => fut.poll_cancel(cx),
            ReadVChainStateProj::Second { fut } => fut.poll_cancel(cx),
            ReadVChainStateProj::Temporary => Poll::Ready(()),
        }
    }
}

impl<'a, 'b, T: AsyncRead, U: AsyncRead> Future for ReadVectoredChain<'a, 'b, T, U>
where
    <T as AsyncReadWith<'a, 'b>>::ReadVectoredFuture: Future<Output = Result<()>>,
    <U as AsyncReadWith<'a, 'b>>::ReadVectoredFuture: Future<Output = Result<()>>,
{
    type Output = Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

pin_project! {
    /// Future for [`fill_buf`](AsyncBufReadWith::fill_buf) on a [`Chain`].
    pub struct FillBufChain<'a, T: AsyncBufRead, U: AsyncBufRead>
    where
        U: 'static,
    {
        #[pin]
        state: FillBufChainState<'a, T, U>,
    }
}
pin_project! {
    #[project = FillBufChainStateProj]
    #[project_replace = FillBufChainStateProjReplace]
    enum FillBufChainState<'a, T: AsyncBufRead, U: AsyncBufRead>
    where
        U: 'static,
    {
        First {
            #[pin]
            fut: <T as AsyncBufReadWith<'a>>::FillBufFuture,
            done_first: &'a mut bool,
            second: &'a mut U,
        },
        Second {
            #[pin]
            fut: <U as AsyncBufReadWith<'a>>::FillBufFuture,
        },
        Temporary,
    }
}

impl<'a, T: AsyncBufRead, U: AsyncBufRead> CompletionFuture for FillBufChain<'a, T, U> {
    type Output = Result<&'a [u8]>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        if let FillBufChainStateProj::First { fut, .. } = this.state.as_mut().project() {
            let buf = ready!(fut.poll(cx))?;

            if !buf.is_empty() {
                return Poll::Ready(Ok(buf));
            }

            let (done_first, second) = match this
                .state
                .as_mut()
                .project_replace(FillBufChainState::Temporary)
            {
                FillBufChainStateProjReplace::First {
                    done_first, second, ..
                } => (done_first, second),
                _ => unreachable!(),
            };

            *done_first = true;
            this.state.set(FillBufChainState::Second {
                fut: second.fill_buf(),
            });
        }
        match this.state.project() {
            FillBufChainStateProj::Second { fut } => fut.poll(cx),
            _ => unreachable!(),
        }
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        match self.project().state.project() {
            FillBufChainStateProj::First { fut, .. } => fut.poll_cancel(cx),
            FillBufChainStateProj::Second { fut } => fut.poll_cancel(cx),
            FillBufChainStateProj::Temporary => Poll::Ready(()),
        }
    }
}
impl<'a, T: AsyncBufRead, U: AsyncBufRead> Future for FillBufChain<'a, T, U>
where
    <T as AsyncBufReadWith<'a>>::FillBufFuture: Future<Output = Result<&'a [u8]>>,
    <U as AsyncBufReadWith<'a>>::FillBufFuture: Future<Output = Result<&'a [u8]>>,
{
    type Output = Result<&'a [u8]>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{Error, ErrorKind};
    use std::mem::MaybeUninit;

    use crate::future::block_on;

    use super::super::{test_utils::YieldingReader, AsyncReadExt};

    #[test]
    fn read() {
        let first = YieldingReader::new(vec![Ok(&[1, 2, 3][..]), Ok(&[4])]);
        let second = YieldingReader::new(vec![
            Err(Error::new(ErrorKind::Other, "Some error")),
            Ok(&[5, 6, 7][..]),
        ]);

        let mut storage = [MaybeUninit::uninit(); 20];
        let mut buf = ReadBuf::uninit(&mut storage);

        let mut chain = first.chain(second);

        block_on(chain.read(buf.as_ref())).unwrap();
        assert_eq!(buf.as_ref().filled(), [1, 2, 3]);

        block_on(chain.read(buf.as_ref())).unwrap();
        assert_eq!(buf.as_ref().filled(), [1, 2, 3, 4]);

        assert_eq!(
            block_on(chain.read(buf.as_ref())).unwrap_err().to_string(),
            "Some error"
        );
        assert_eq!(buf.as_ref().filled(), [1, 2, 3, 4]);

        block_on(chain.read(buf.as_ref())).unwrap();
        assert_eq!(buf.as_ref().filled(), [1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn buf_read() {
        let first = YieldingReader::new(vec![Ok(&[1, 2, 3][..]), Ok(&[4])]);
        let second = YieldingReader::new(vec![
            Err(Error::new(ErrorKind::Other, "Some error")),
            Ok(&[5, 6, 7][..]),
        ]);

        let mut chain = first.chain(second);

        assert_eq!(block_on(chain.fill_buf()).unwrap(), [1, 2, 3]);
        assert_eq!(block_on(chain.fill_buf()).unwrap(), [1, 2, 3]);

        chain.consume(2);
        assert_eq!(block_on(chain.fill_buf()).unwrap(), [3]);
        assert_eq!(block_on(chain.fill_buf()).unwrap(), [3]);

        chain.consume(1);
        assert_eq!(block_on(chain.fill_buf()).unwrap(), [4]);
        assert_eq!(block_on(chain.fill_buf()).unwrap(), [4]);

        chain.consume(1);
        assert_eq!(
            block_on(chain.fill_buf()).unwrap_err().to_string(),
            "Some error"
        );
        assert_eq!(block_on(chain.fill_buf()).unwrap(), [5, 6, 7]);
        assert_eq!(block_on(chain.fill_buf()).unwrap(), [5, 6, 7]);

        chain.consume(3);
        assert_eq!(block_on(chain.fill_buf()).unwrap(), []);
        assert_eq!(block_on(chain.fill_buf()).unwrap(), []);
    }
}
