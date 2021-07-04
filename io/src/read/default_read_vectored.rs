use std::cell::UnsafeCell;
use std::future::Future;
use std::io::Result;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::pin::Pin;
use std::task::{Context, Poll};

use completion_core::CompletionFuture;
use futures_core::ready;
use pin_project_lite::pin_project;
use pinned_aliasable::Aliasable;

use super::{AsyncReadWith, ReadBuf, ReadBufsRef};

pin_project! {
    /// A default implementation of [`ReadVectoredFuture`](AsyncReadWith::ReadVectoredFuture) for
    /// types that don't have efficient vectored reads.
    ///
    /// This will forward to [`read`](AsyncReadWith::read) with the first nonempty buffer provided,
    /// or an empty one if none exists.
    #[derive(Debug)]
    #[project = Proj]
    pub struct DefaultReadVectored<'a, 'b, T: AsyncReadWith<'a, 'b>> {
        #[pin]
        state: State<'a, 'b, T>,
        #[pin]
        bufs: Aliasable<UnsafeCell<ReadBufsRef<'a, 'b>>>,
        _debug_bounds: PhantomData<T::ReadFuture>,
    }
}

unsafe impl<'a, 'b, T: AsyncReadWith<'a, 'b>> Send for DefaultReadVectored<'a, 'b, T>
where
    T: Send,
    <T as AsyncReadWith<'a, 'b>>::ReadFuture: Send,
{
}
unsafe impl<'a, 'b, T: AsyncReadWith<'a, 'b>> Sync for DefaultReadVectored<'a, 'b, T>
where
    T: Sync,
    <T as AsyncReadWith<'a, 'b>>::ReadFuture: Sync,
{
}

pin_project! {
    #[derive(Debug)]
    #[project = StateProj]
    #[project_replace = StateProjReplace]
    enum State<'a, 'b, T: AsyncReadWith<'a, 'b>> {
        Initial {
            reader: &'a mut T,
        },
        Reading {
            // The intenal reading future. Only `None` during initialization and destruction.
            #[pin]
            future: Option<T::ReadFuture>,
            // Guard that contains the `ReadBuf` the above future reads into and makes sure that
            // the `ReadBufs`' cursor is updated once the future is done.
            #[pin]
            guard: Aliasable<Guard<'a, 'b>>,
        },
        Done,
    }
}

impl<'a, 'b, T: AsyncReadWith<'a, 'b>> DefaultReadVectored<'a, 'b, T> {
    /// Create a new `DefaultReadVectored` future.
    pub fn new(reader: &'a mut T, bufs: ReadBufsRef<'a, 'b>) -> Self {
        Self {
            state: State::Initial { reader },
            bufs: Aliasable::new(UnsafeCell::new(bufs)),
            _debug_bounds: PhantomData,
        }
    }
}

impl<'a, 'b, T: AsyncReadWith<'a, 'b>> CompletionFuture for DefaultReadVectored<'a, 'b, T> {
    type Output = Result<()>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let Proj {
            mut state, bufs, ..
        } = self.project();

        if let StateProj::Done = state.as_mut().project() {
            panic!("Polled `DefaultReadVectored` after completion");
        }
        if let StateProj::Initial { .. } = state.as_mut().project() {
            let bufs_cell = unsafe { bufs.as_ref().get_extended() };
            let bufs = unsafe { &mut *bufs_cell.get() };

            // Calculate how many bytes are initialized at the start of the partially filled buffer.
            // `None` if the buffer is fully initialized.
            let (initialized_bufs, initialized_buf) = bufs.initialized();
            let (filled_bufs, filled_buf) = bufs.filled();
            let initialized_to = (initialized_bufs.len() == filled_bufs.len())
                .then(|| initialized_buf.len() - filled_buf.len());

            // Create a `ReadBuf` to pass to the future.
            let mut read_buf = ReadBuf::uninit(unsafe { bufs.unfilled_mut() }.0);
            unsafe { read_buf.assume_init(initialized_to.unwrap_or_else(|| read_buf.capacity())) };

            // Set the state to reading, and take the reader from the old state.
            let reader = match state.as_mut().project_replace(State::Reading {
                future: None,
                guard: Aliasable::new(Guard {
                    buf: ManuallyDrop::new(UnsafeCell::new(read_buf)),
                    bufs: bufs_cell,
                }),
            }) {
                StateProjReplace::Initial { reader } => reader,
                _ => unreachable!(),
            };

            // Delegate to the inner reading future.
            let (mut future, guard) = match state.as_mut().project() {
                StateProj::Reading { future, guard } => (future, guard),
                _ => unreachable!(),
            };
            let read_buf = unsafe { &mut *guard.as_ref().get_extended().buf.get() };
            future.set(Some(reader.read(read_buf.as_ref())));
        }
        if let StateProj::Reading { future, .. } = state.as_mut().project() {
            ready!(unsafe { future.as_pin_mut().unwrap().poll(cx) })?;
        }
        state.set(State::Done);
        Poll::Ready(Ok(()))
    }
    unsafe fn poll_cancel(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut state = self.project().state;

        match state.as_mut().project() {
            StateProj::Initial { .. } => {}
            StateProj::Reading { future, .. } => {
                ready!(unsafe { future.as_pin_mut().unwrap().poll_cancel(cx) });
            }
            StateProj::Done => panic!("Polled `DefaultReadVectored` after completion"),
        }
        state.set(State::Done);
        Poll::Ready(())
    }
}

impl<'a, 'b, T: AsyncReadWith<'a, 'b>> Future for DefaultReadVectored<'a, 'b, T>
where
    <T as AsyncReadWith<'a, 'b>>::ReadFuture: Future<Output = Result<()>>,
{
    type Output = Result<()>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { CompletionFuture::poll(self, cx) }
    }
}

/// Guard that updates the number of filled bytes in the `ReadBufs` using the `ReadBuf` when it is
/// dropped.
struct Guard<'a, 'b> {
    buf: ManuallyDrop<UnsafeCell<ReadBuf<'a>>>,
    bufs: &'a UnsafeCell<ReadBufsRef<'a, 'b>>,
}

impl<'a, 'b> Drop for Guard<'a, 'b> {
    fn drop(&mut self) {
        let buf = self.buf.get_mut();
        let filled = buf.filled().len();
        let initialized = buf.initialized().len();
        unsafe { ManuallyDrop::drop(&mut self.buf) };

        // SAFETY: The only other reference to this should be `buf`, which has been dropped.
        let bufs = unsafe { &mut *self.bufs.get() };
        unsafe { bufs.assume_init(initialized) };
        bufs.add_filled(filled);
    }
}

#[cfg(test)]
mod tests {
    use std::io::Result;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use completion_core::CompletionFuture;

    use crate::read::{io_slices, AsyncReadWith, ReadBufRef, ReadBufs, ReadBufsRef};
    use crate::read::{to_init_slices, to_slices};

    use super::DefaultReadVectored;

    struct Reader {
        expected_capacity: usize,
        expected_initialized: Vec<u8>,
        to_append: Vec<u8>,
    }

    impl<'a, 'b> AsyncReadWith<'a, 'b> for Reader {
        type ReadFuture = Read<'a>;
        fn read(&'a mut self, buf: ReadBufRef<'a>) -> Self::ReadFuture {
            assert_eq!(buf.capacity(), self.expected_capacity);
            assert_eq!(buf.initialized(), self.expected_initialized);
            assert_eq!(buf.filled().len(), 0);
            Read {
                reader: self,
                buf,
                state: 0,
            }
        }

        type ReadVectoredFuture = DefaultReadVectored<'a, 'b, Self>;
        fn read_vectored(&'a mut self, bufs: ReadBufsRef<'a, 'b>) -> Self::ReadVectoredFuture {
            DefaultReadVectored::new(self, bufs)
        }
    }

    struct Read<'a> {
        reader: &'a mut Reader,
        buf: ReadBufRef<'a>,
        state: u32,
    }
    impl CompletionFuture for Read<'_> {
        type Output = Result<()>;

        unsafe fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let this = &mut *self;
            match this.state {
                0 => {}
                1 => {
                    this.buf.append(&this.reader.to_append);
                }
                2 => return Poll::Ready(Ok(())),
                _ => unreachable!(),
            }
            this.state += 1;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
        unsafe fn poll_cancel(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            let this = &mut *self;
            match this.state {
                0 => panic!(),
                1 => {
                    this.buf.append(&this.reader.to_append);
                }
                2 => return Poll::Ready(()),
                _ => unreachable!(),
            }
            this.state += 1;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }

    #[test]
    fn reading() {
        io_slices!(let data = init [[], [], [1, 2, 3, 4, 5, 6], [0]]);
        let mut bufs = ReadBufs::uninit(data);
        unsafe { bufs.assume_init(4) };
        bufs.add_filled(1);

        let mut reader = Reader {
            expected_capacity: 5,
            expected_initialized: vec![2, 3, 4],
            to_append: vec![255, 254, 253],
        };

        let mut future = Box::pin(reader.read_vectored(bufs.as_ref()));
        assert!(matches!(
            future.as_ref().project_ref().state.get_ref(),
            super::State::Initial { .. }
        ));

        assert!(unsafe { future.as_mut().poll(&mut noop_cx()) }.is_pending());
        assert!(matches!(
            future.as_ref().project_ref().state.get_ref(),
            super::State::Reading { .. }
        ));

        assert!(unsafe { future.as_mut().poll(&mut noop_cx()) }.is_pending());
        assert!(matches!(
            unsafe { future.as_mut().poll(&mut noop_cx()) },
            Poll::Ready(Ok(()))
        ));
        assert!(matches!(
            future.as_ref().project_ref().state.get_ref(),
            super::State::Done
        ));

        drop(future);

        assert_eq!(bufs.filled().1, &[1, 255, 254, 253]);
        assert_eq!(bufs.initialized().1, &[1, 255, 254, 253]);
        assert_eq!(
            unsafe { to_init_slices(bufs.inner()) },
            [&[] as &[u8], &[], &[1, 255, 254, 253, 5, 6], &[0]]
        );
    }

    #[test]
    fn premature_drop() {
        io_slices!(let data = init [[91, 92, 93, 94, 95, 96]]);
        let mut bufs = ReadBufs::uninit(data);

        let mut reader = Reader {
            expected_capacity: 6,
            expected_initialized: Vec::new(),
            to_append: vec![1, 2, 3, 4, 5, 6],
        };

        let mut future = Box::pin(reader.read_vectored(bufs.as_ref()));
        assert!(unsafe { future.as_mut().poll(&mut noop_cx()) }.is_pending());
        assert!(unsafe { future.as_mut().poll(&mut noop_cx()) }.is_pending());
        drop(future);

        assert_eq!(to_slices(bufs.filled().0), [[1, 2, 3, 4, 5, 6]]);
        assert_eq!(bufs.filled().1, &[]);
        assert_eq!(to_slices(bufs.initialized().0), [[1, 2, 3, 4, 5, 6]]);
        assert_eq!(bufs.initialized().1, &[]);
    }

    #[test]
    fn cancellation() {
        io_slices!(let data = [[1, 2], [93, 94, 95, 96], [7, 8]]);
        let mut bufs = ReadBufs::new(data);
        bufs.add_filled(2);

        let mut reader = Reader {
            expected_capacity: 4,
            expected_initialized: vec![93, 94, 95, 96],
            to_append: vec![3, 4, 5, 6],
        };

        let mut future = Box::pin(reader.read_vectored(bufs.as_ref()));
        assert!(unsafe { future.as_mut().poll(&mut noop_cx()) }.is_pending());
        assert!(unsafe { future.as_mut().poll_cancel(&mut noop_cx()) }.is_pending());
        assert!(unsafe { future.as_mut().poll_cancel(&mut noop_cx()) }.is_ready());
        assert!(matches!(
            future.as_ref().project_ref().state.get_ref(),
            super::State::Done,
        ));
        drop(future);

        assert_eq!(
            to_slices(bufs.filled().0),
            [&[1_u8, 2] as &[_], &[3, 4, 5, 6]]
        );
        assert_eq!(bufs.filled().1, &[]);
        assert_eq!(
            to_slices(bufs.initialized().0),
            [&[1_u8, 2] as &[_], &[3, 4, 5, 6], &[7, 8]]
        );
        assert_eq!(bufs.initialized().1, &[]);
    }

    #[test]
    fn cancel_not_started() {
        io_slices!(let data = uninit [7]);
        let mut bufs = ReadBufs::uninit(data);

        let mut reader = Reader {
            expected_capacity: 10000,
            expected_initialized: vec![1, 2, 3],
            to_append: Vec::new(),
        };

        let mut future = Box::pin(reader.read_vectored(bufs.as_ref()));
        assert!(unsafe { future.as_mut().poll_cancel(&mut noop_cx()) }.is_ready());
        drop(future);

        assert_eq!(bufs.filled().0.len(), 0);
        assert_eq!(bufs.filled().1, &[]);
        assert_eq!(bufs.initialized().0.len(), 0);
        assert_eq!(bufs.initialized().1, &[]);
    }

    fn noop_cx() -> Context<'static> {
        use std::ptr;
        use std::task::{RawWaker, RawWakerVTable, Waker};

        const WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(|_| RAW_WAKER, drop, drop, drop);
        const RAW_WAKER: RawWaker = RawWaker::new(ptr::null(), &WAKER_VTABLE);

        struct SyncRawWaker(RawWaker);
        unsafe impl Sync for SyncRawWaker {}
        static SYNC_RAW_WAKER: SyncRawWaker = SyncRawWaker(RAW_WAKER);

        // SAFETY: RawWaker and Waker and guaranteed to have the same layout.
        let waker = unsafe { &*(&SYNC_RAW_WAKER.0 as *const RawWaker).cast::<Waker>() };
        Context::from_waker(waker)
    }
}
