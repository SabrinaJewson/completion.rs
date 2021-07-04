use std::future::{self, Future};
use std::io::{Cursor, Empty, IoSliceMut, Repeat, Result};
use std::mem::MaybeUninit;
use std::pin::Pin;
use std::slice;
use std::task::{Context, Poll};

use completion_core::CompletionFuture;

use crate::util::derive_completion_future;

mod co_mut;

mod maybe_uninit_io_slice_mut;
pub use maybe_uninit_io_slice_mut::MaybeUninitIoSliceMut;

mod buf;
pub use buf::{ReadBuf, ReadBufRef};

mod bufs;
pub use bufs::{ReadBufs, ReadBufsRef};

mod default_read_vectored;
pub use default_read_vectored::DefaultReadVectored;

/// Read bytes from a source asynchronously.
///
/// This is an asynchronous version of [`std::io::Read`].
///
/// You should not implement this trait manually, instead implement [`AsyncReadWith`].
pub trait AsyncRead: for<'a, 'b> AsyncReadWith<'a, 'b> {}
impl<T: for<'a, 'b> AsyncReadWith<'a, 'b> + ?Sized> AsyncRead for T {}

/// Read bytes from a source asynchronously with a specific lifetime.
///
/// This trait has two lifetime parameters. The first lifetime `'a` is the lifetime of all
/// references passed into the trait's methods. The second lifetime `'b` is the lifetime used as the
/// second lifetime parameter of the [`ReadBufsRef`] passed into `read_vectored`. It must be
/// separate lifetime, as unlike `'a` it is **invariant**.
///
/// This trait also has a type parameter, but that is an unstable implementation detail. You cannot
/// implement the trait with any type there other than the default one, and you should not set it to
/// any value other than the default one in a trait bound (we can't enforce that statically, but
/// you still should not do it).
// The `LifetimeBounds` type parameter is present to enforce the lifetime bound `'b: 'a`, which is
// required in both the associated types and the functions that create them. The reason we don't
// enforce this bound by writing `b: 'a` directly is that that isn't supported by HRTBs:
// `for<'a, 'b: 'a> AsyncReadWith<'a, 'b>` is not valid syntax.
pub trait AsyncReadWith<'a, 'b, ImplicitBounds: sealed::Sealed = sealed::ImplicitBounds<'a, 'b>> {
    /// The future that reads from the source.
    ///
    /// If this future is cancelled but the operation was still able to successfully complete, the
    /// passed in [`ReadBufRef`] should be filled in with the read bytes to avoid losing data.
    type ReadFuture: CompletionFuture<Output = Result<()>>;

    /// Pull some bytes from this source into the specified buffer.
    ///
    /// If this reads 0 bytes of data, either the buffer was 0 bytes in length or the stream has
    /// reached EOF.
    ///
    /// Keep it mind that it is possible for misbehaving implementations of `AsyncReadWith` to
    /// replace the `ReadBuf` given to this function with a different one. While no sane
    /// implementation will actually do this, if it does happen UB still must not occur. This can
    /// usually be dealt with by simply panicking if, after `read` completes, the buffer's pointer
    /// has changed.
    fn read(&'a mut self, buf: ReadBufRef<'a>) -> Self::ReadFuture;

    /// The future that reads from the source into a vector of buffers. If your reader does not
    /// have efficient vectored reads, set this to [`DefaultReadVectored<'a, 'b,
    /// Self>`](DefaultReadVectored).
    ///
    /// Note that it is **not** considered a semver breaking change to change this associated type
    /// from [`DefaultReadVectored`] to another type, so if you are using a reader whose associated
    /// `ReadVectoredFuture` type is [`DefaultReadVectored`], you should not rely on it staying
    /// that way.
    ///
    /// If this future is cancelled but the operation was still able to successfully complete, the
    /// passed in [`ReadBufsRef`] should be filled in with the read bytes to avoid losing data.
    type ReadVectoredFuture: CompletionFuture<Output = Result<()>>;

    /// Like [`read`], except that it reads into several not necessarily contiguous buffers.
    ///
    /// This method must behave equivalently to a single call to [`read`] with concatenated
    /// buffers.
    ///
    /// If your reader does not have efficient vectored reads, call
    /// [`DefaultReadVectored::new(self, bufs)`](DefaultReadVectored::new). Otherwise, make sure to
    /// override [`is_read_vectored`](Self::is_read_vectored) to return `true`.
    ///
    /// [`read`]: Self::read
    fn read_vectored(&'a mut self, bufs: ReadBufsRef<'a, 'b>) -> Self::ReadVectoredFuture;

    /// Determines if the `AsyncRead`er has an efficient [`read_vectored`](Self::read_vectored)
    /// implementation.
    ///
    /// The default implementation returns `false`.
    fn is_read_vectored(&self) -> bool {
        false
    }
}

mod sealed {
    pub trait Sealed {}
    #[allow(missing_debug_implementations)]
    pub struct ImplicitBounds<'a, 'b>(&'a &'b ());
    impl Sealed for ImplicitBounds<'_, '_> {}
}

impl<'a, 'b, R> AsyncReadWith<'a, 'b> for &mut R
where
    R: AsyncRead + ?Sized,
{
    type ReadFuture = <R as AsyncReadWith<'a, 'b>>::ReadFuture;

    fn read(&'a mut self, buf: ReadBufRef<'a>) -> <R as AsyncReadWith<'a, 'b>>::ReadFuture {
        (**self).read(buf)
    }

    type ReadVectoredFuture = <R as AsyncReadWith<'a, 'b>>::ReadVectoredFuture;

    fn read_vectored(&'a mut self, bufs: ReadBufsRef<'a, 'b>) -> Self::ReadVectoredFuture {
        (**self).read_vectored(bufs)
    }

    fn is_read_vectored(&self) -> bool {
        (**self).is_read_vectored()
    }
}

impl<'a, 'b, R> AsyncReadWith<'a, 'b> for Box<R>
where
    R: AsyncRead + ?Sized,
{
    type ReadFuture = <R as AsyncReadWith<'a, 'b>>::ReadFuture;

    fn read(&'a mut self, buf: ReadBufRef<'a>) -> <R as AsyncReadWith<'a, 'b>>::ReadFuture {
        (**self).read(buf)
    }

    type ReadVectoredFuture = <R as AsyncReadWith<'a, 'b>>::ReadVectoredFuture;

    fn read_vectored(&'a mut self, bufs: ReadBufsRef<'a, 'b>) -> Self::ReadVectoredFuture {
        (**self).read_vectored(bufs)
    }

    fn is_read_vectored(&self) -> bool {
        (**self).is_read_vectored()
    }
}

impl<'a, 'b> AsyncReadWith<'a, 'b> for Empty {
    type ReadFuture = future::Ready<Result<()>>;

    fn read(&'a mut self, _buf: ReadBufRef<'a>) -> Self::ReadFuture {
        future::ready(Ok(()))
    }

    type ReadVectoredFuture = future::Ready<Result<()>>;

    fn read_vectored(&'a mut self, _bufs: ReadBufsRef<'a, 'b>) -> Self::ReadVectoredFuture {
        future::ready(Ok(()))
    }

    fn is_read_vectored(&self) -> bool {
        true
    }
}

fn repeat_byte(repeat: &mut Repeat) -> u8 {
    let mut byte = 0_u8;
    std::io::Read::read(repeat, std::slice::from_mut(&mut byte)).unwrap();
    byte
}

impl<'a, 'b> AsyncReadWith<'a, 'b> for Repeat {
    type ReadFuture = ReadRepeat<'a>;

    fn read(&'a mut self, buf: ReadBufRef<'a>) -> Self::ReadFuture {
        ReadRepeat {
            byte: repeat_byte(self),
            buf,
        }
    }

    type ReadVectoredFuture = ReadVectoredRepeat<'a, 'b>;

    fn read_vectored(&'a mut self, bufs: ReadBufsRef<'a, 'b>) -> Self::ReadVectoredFuture {
        ReadVectoredRepeat {
            byte: repeat_byte(self),
            bufs,
        }
    }

    fn is_read_vectored(&self) -> bool {
        true
    }
}

/// Future for [`read`](AsyncReadWith::read) on a [`Repeat`].
#[derive(Debug)]
pub struct ReadRepeat<'a> {
    byte: u8,
    buf: ReadBufRef<'a>,
}
impl Future for ReadRepeat<'_> {
    type Output = Result<()>;

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        let remaining = this.buf.remaining();
        unsafe { this.buf.unfilled_mut() }.fill(MaybeUninit::new(this.byte));
        unsafe { this.buf.assume_init(remaining) };
        this.buf.add_filled(remaining);
        Poll::Ready(Ok(()))
    }
}
derive_completion_future!(ReadRepeat<'_>);

/// Future for [`read_vectored`](AsyncReadWith::read_vectored) on a [`Repeat`].
#[derive(Debug)]
pub struct ReadVectoredRepeat<'a, 'b> {
    byte: u8,
    bufs: ReadBufsRef<'a, 'b>,
}
impl Future for ReadVectoredRepeat<'_, '_> {
    type Output = Result<()>;

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        let remaining = this.bufs.remaining();
        let (unfilled_buf, unfilled_bufs) = unsafe { this.bufs.unfilled_mut() };
        unfilled_buf.fill(MaybeUninit::new(this.byte));
        for buf in unfilled_bufs {
            buf.fill(MaybeUninit::new(this.byte));
        }
        unsafe { this.bufs.assume_init(remaining) };
        this.bufs.add_filled(remaining);
        Poll::Ready(Ok(()))
    }
}
derive_completion_future!(ReadVectoredRepeat<'_, '_>);

#[test]
fn test_read_repeat() {
    let mut bytes = [std::mem::MaybeUninit::uninit(); 13];
    let mut buf = ReadBuf::uninit(&mut bytes);

    futures_lite::future::block_on(std::io::repeat(185).read(buf.as_ref())).unwrap();

    assert_eq!(buf.into_filled(), &[185; 13]);
}

impl<'a, 'b, 's> AsyncReadWith<'a, 'b> for &'s [u8] {
    type ReadFuture = ReadSlice<'a, 's>;

    fn read(&'a mut self, buf: ReadBufRef<'a>) -> Self::ReadFuture {
        ReadSlice {
            // Safety: We are extending the lifetime of the reference from 'a to 's. This is safe
            // because the struct it is in only lives for as long as 'a.
            slice: unsafe { &mut *(self as *mut _) },
            buf,
        }
    }

    type ReadVectoredFuture = ReadVectoredSlice<'a, 'b, 's>;

    fn read_vectored(&'a mut self, bufs: ReadBufsRef<'a, 'b>) -> Self::ReadVectoredFuture {
        ReadVectoredSlice {
            // Safety: We are extending the lifetime of the reference from 'a to 's. This is safe
            // because the struct it is in only lives for as long as 'a.
            slice: unsafe { &mut *(self as *mut _) },
            bufs,
        }
    }

    fn is_read_vectored(&self) -> bool {
        true
    }
}

/// Future for [`read`](AsyncReadWith::read) on a byte slice (`&[u8]`).
#[derive(Debug)]
pub struct ReadSlice<'a, 's> {
    // This is conceptually an &'a mut &'s [u8]. However, that would add the implicit bound 's: 'a
    // which is incompatible with AsyncReadWith.
    slice: &'s mut &'s [u8],
    buf: ReadBufRef<'a>,
}
impl Future for ReadSlice<'_, '_> {
    type Output = Result<()>;

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let amount = std::cmp::min(self.buf.remaining(), self.slice.len());
        let (write, rest) = self.slice.split_at(amount);
        self.buf.append(write);
        *self.slice = rest;

        Poll::Ready(Ok(()))
    }
}
derive_completion_future!(ReadSlice<'_, '_>);

/// Future for [`read_vectored`](AsyncReadWith::read_vectored) on a byte slice (`&[u8]`).
#[derive(Debug)]
pub struct ReadVectoredSlice<'a, 'b, 's> {
    // This is conceptually an &'a mut &'s [u8]. However, that would add the implicit bound 's: 'a
    // which is incompatible with AsyncReadWith.
    slice: &'s mut &'s [u8],
    bufs: ReadBufsRef<'a, 'b>,
}
impl Future for ReadVectoredSlice<'_, '_, '_> {
    type Output = Result<()>;

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let amount = std::cmp::min(self.bufs.remaining(), self.slice.len());
        let (write, rest) = self.slice.split_at(amount);
        self.bufs.append(write);
        *self.slice = rest;

        Poll::Ready(Ok(()))
    }
}
derive_completion_future!(ReadVectoredSlice<'_, '_, '_>);

#[test]
fn test_read_slice() {
    futures_lite::future::block_on(async {
        let mut bytes = [std::mem::MaybeUninit::uninit(); 7];
        let mut buf = ReadBuf::uninit(&mut bytes);

        let mut slice: &[u8] = &[1, 2, 3, 4, 5];
        slice.read(buf.as_ref()).await.unwrap();

        assert_eq!(slice, &[]);
        assert_eq!(buf.filled(), &[1, 2, 3, 4, 5]);

        let mut slice: &[u8] = &[6, 7, 8, 9, 10];
        slice.read(buf.as_ref()).await.unwrap();

        assert_eq!(slice, &[8, 9, 10]);
        assert_eq!(&mut buf.filled(), &[1, 2, 3, 4, 5, 6, 7]);
    });
}

impl<'a, 'b, T: AsRef<[u8]>> AsyncReadWith<'a, 'b> for Cursor<T> {
    type ReadFuture = ReadCursor<'a, T>;

    fn read(&'a mut self, buf: ReadBufRef<'a>) -> Self::ReadFuture {
        ReadCursor { cursor: self, buf }
    }

    type ReadVectoredFuture = ReadVectoredCursor<'a, 'b, T>;

    fn read_vectored(&'a mut self, bufs: ReadBufsRef<'a, 'b>) -> Self::ReadVectoredFuture {
        ReadVectoredCursor { cursor: self, bufs }
    }

    fn is_read_vectored(&self) -> bool {
        true
    }
}

/// Future for [`read`](AsyncReadWith::read) on a [`Cursor`].
#[derive(Debug)]
pub struct ReadCursor<'a, T> {
    // This is conceptually an &'a mut Cursor<T>. However, that would add the implicit bound T: 'a
    // which is incompatible with AsyncReadWith.
    cursor: *mut Cursor<T>,
    buf: ReadBufRef<'a>,
}
// ReadBufRef is always Send+Sync, and we hold a mutable reference to Cursor.
unsafe impl<T: Send> Send for ReadCursor<'_, T> {}
unsafe impl<T: Sync> Sync for ReadCursor<'_, T> {}

impl<T: AsRef<[u8]>> Future for ReadCursor<'_, T> {
    type Output = Result<()>;

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let cursor = unsafe { &mut *self.cursor };

        let slice = std::io::BufRead::fill_buf(cursor)?;
        let amount = std::cmp::min(self.buf.remaining(), slice.len());
        self.buf.append(&slice[..amount]);
        cursor.set_position(cursor.position() + amount as u64);

        Poll::Ready(Ok(()))
    }
}
derive_completion_future!([T: AsRef<[u8]>] ReadCursor<'_, T>);

/// Future for [`read_vectored`](AsyncReadWith::read_vectored) on a [`Cursor`].
#[derive(Debug)]
pub struct ReadVectoredCursor<'a, 'b, T> {
    // This is conceptually an &'a mut Cursor<T>. However, that would add the implicit bound T: 'a
    // which is incompatible with AsyncReadWith.
    cursor: *mut Cursor<T>,
    bufs: ReadBufsRef<'a, 'b>,
}
// ReadBufsRef is always Send+Sync, and we hold a mutable reference to Cursor.
unsafe impl<T: Send> Send for ReadVectoredCursor<'_, '_, T> {}
unsafe impl<T: Sync> Sync for ReadVectoredCursor<'_, '_, T> {}

impl<T: AsRef<[u8]>> Future for ReadVectoredCursor<'_, '_, T> {
    type Output = Result<()>;

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let cursor = unsafe { &mut *self.cursor };

        let slice = std::io::BufRead::fill_buf(cursor)?;
        let amount = std::cmp::min(self.bufs.remaining(), slice.len());
        self.bufs.append(&slice[..amount]);
        cursor.set_position(cursor.position() + amount as u64);

        Poll::Ready(Ok(()))
    }
}
derive_completion_future!([T: AsRef<[u8]>] ReadVectoredCursor<'_, '_, T>);

#[test]
fn test_read_cursor() {
    futures_lite::future::block_on(async {
        let mut bytes = [std::mem::MaybeUninit::uninit(); 7];
        let mut buf = ReadBuf::uninit(&mut bytes);

        let mut cursor = Cursor::new(vec![1, 2, 3, 4, 5]);
        cursor.read(buf.as_ref()).await.unwrap();
        assert_eq!(cursor.position(), 5);
        assert_eq!(buf.filled(), &[1, 2, 3, 4, 5]);

        let mut cursor = Cursor::new(vec![6, 7, 8, 9, 10]);
        cursor.read(buf.as_ref()).await.unwrap();
        assert_eq!(cursor.position(), 2);
        assert_eq!(buf.filled(), &[1, 2, 3, 4, 5, 6, 7]);
    });
}

#[cfg(test)]
#[allow(dead_code, clippy::extra_unused_lifetimes)]
fn test_impls_traits<'a, R: AsyncRead + 'a>() {
    fn assert_impls<R: AsyncRead>() {}

    assert_impls::<Empty>();
    assert_impls::<&'a mut Empty>();
    assert_impls::<Box<Empty>>();
    assert_impls::<&'a mut Box<&'a mut Empty>>();

    assert_impls::<&'a [u8]>();
    assert_impls::<&'a mut &'a [u8]>();

    assert_impls::<Cursor<Vec<u8>>>();
    assert_impls::<Cursor<&'a [u8]>>();
    assert_impls::<&'a mut Cursor<&'a [u8]>>();

    assert_impls::<R>();
    assert_impls::<&'a mut R>();
    assert_impls::<&mut &'a mut R>();
}

unsafe fn buf_assume_init_ref(buf: &[MaybeUninit<u8>]) -> &[u8] {
    unsafe { slice::from_raw_parts(buf.as_ptr().cast(), buf.len()) }
}
unsafe fn buf_assume_init_mut(buf: &mut [MaybeUninit<u8>]) -> &mut [u8] {
    unsafe { slice::from_raw_parts_mut(buf.as_mut_ptr().cast(), buf.len()) }
}

unsafe fn bufs_assume_init_ref<'a, 'b>(
    bufs: &'a [MaybeUninitIoSliceMut<'b>],
) -> &'a [IoSliceMut<'b>] {
    unsafe { slice::from_raw_parts(bufs.as_ptr().cast(), bufs.len()) }
}
unsafe fn bufs_assume_init_mut<'a, 'b>(
    bufs: &'a mut [MaybeUninitIoSliceMut<'b>],
) -> &'a mut [IoSliceMut<'b>] {
    unsafe { slice::from_raw_parts_mut(bufs.as_mut_ptr().cast(), bufs.len()) }
}

#[cfg(test)]
fn to_slices<'a>(bufs: &'a [IoSliceMut<'_>]) -> Vec<&'a [u8]> {
    bufs.iter().map(|buf| &**buf).collect()
}
#[cfg(test)]
unsafe fn to_init_slices<'a>(bufs: &'a [MaybeUninitIoSliceMut<'_>]) -> Vec<&'a [u8]> {
    to_slices(unsafe { bufs_assume_init_ref(bufs) })
}

#[cfg(test)]
fn box_u8s<const N: usize>(array: [u8; N]) -> Box<[u8]> {
    Box::new(array)
}
#[cfg(test)]
fn box_mu_u8s<const N: usize>(array: [MaybeUninit<u8>; N]) -> Box<[MaybeUninit<u8>]> {
    Box::new(array)
}

#[cfg(test)]
define_macro! { io_slices {
    (let $pat:pat = [$($data:expr),* $(,)?]) => {
        let mut slices = [$(crate::read::box_u8s($data),)*];
        let mut vec: Vec<_> = slices.iter_mut().map(|s| crate::IoSliceMut::new(s)).collect();
        let $pat = &mut *vec;
    };
    (let $pat:pat = [$data:expr; $repeat:expr]) => {
        let mut slices = [$data; $repeat];
        let mut vec: Vec<_> = slices.iter_mut().map(|s| crate::IoSliceMut::new(s)).collect();
        let $pat = &mut *vec;
    };
    (let $pat:pat = MaybeUninit [$($data:expr),* $(,)?]) => {
        let mut slices = [$(crate::read::box_mu_u8s($data),)*];
        let mut vec: Vec<_> = slices.iter_mut().map(|s| crate::MaybeUninitIoSliceMut::new(s)).collect();
        let $pat = &mut *vec;
    };
    (let $pat:pat = init [$([$($data:expr),* $(,)?]),* $(,)?]) => {
        io_slices!(let $pat = MaybeUninit [$([$(std::mem::MaybeUninit::new($data),)*],)*]);
    };
    (let $pat:pat = uninit [$($len:expr),* $(,)?]) => {
        io_slices!(let $pat = MaybeUninit [$([std::mem::MaybeUninit::uninit(); $len],)*]);
    };
    ([$($data:expr),* $(,)?]) => {
        &mut [$(crate::IoSliceMut::new(&mut $data))*]
    };
}}
