use std::fmt::{self, Debug, Formatter};
use std::future::{self, Future};
use std::io::{Cursor, Empty, Repeat, Result};
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::pin::Pin;
use std::ptr::NonNull;
use std::slice;
use std::task::{Context, Poll};

use completion_core::CompletionFuture;

/// Read bytes from a source asynchronously.
///
/// This is an asynchronous version of [`std::io::Read`].
///
/// You should not implement this trait manually, instead implement [`AsyncReadWith`].
pub trait AsyncRead: for<'a> AsyncReadWith<'a> {}
impl<T: for<'a> AsyncReadWith<'a> + ?Sized> AsyncRead for T {}

/// Read bytes from a source asynchronously with a specific lifetime.
pub trait AsyncReadWith<'a> {
    /// The future that reads from the source.
    type ReadFuture: CompletionFuture<Output = Result<()>>;

    /// Pull some bytes from this source into the specified buffer.
    ///
    /// If this reads 0 bytes of data, either the buffer was 0 bytes in length or the stream has
    /// reached EOF.
    fn read(&'a mut self, buf: ReadBufMut<'a>) -> Self::ReadFuture;

    // TODO: support vectored reads
}

impl<'a, R: AsyncReadWith<'a> + ?Sized> AsyncReadWith<'a> for &mut R {
    type ReadFuture = R::ReadFuture;

    fn read(&'a mut self, buf: ReadBufMut<'a>) -> Self::ReadFuture {
        (**self).read(buf)
    }
}

impl<'a, R: AsyncReadWith<'a> + ?Sized> AsyncReadWith<'a> for Box<R> {
    type ReadFuture = R::ReadFuture;

    fn read(&'a mut self, buf: ReadBufMut<'a>) -> Self::ReadFuture {
        (**self).read(buf)
    }
}

impl<'a> AsyncReadWith<'a> for Empty {
    type ReadFuture = future::Ready<Result<()>>;

    fn read(&'a mut self, _buf: ReadBufMut<'a>) -> Self::ReadFuture {
        future::ready(Ok(()))
    }
}

impl<'a> AsyncReadWith<'a> for Repeat {
    type ReadFuture = ReadRepeat<'a>;

    fn read(&'a mut self, buf: ReadBufMut<'a>) -> Self::ReadFuture {
        let mut byte = 0_u8;
        std::io::Read::read(self, std::slice::from_mut(&mut byte)).unwrap();
        ReadRepeat { byte, buf }
    }
}

/// Future for [`read`](AsyncReadWith::read) on a [`Repeat`].
#[derive(Debug)]
pub struct ReadRepeat<'a> {
    byte: u8,
    buf: ReadBufMut<'a>,
}
impl CompletionFuture for ReadRepeat<'_> {
    type Output = Result<()>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Future::poll(self, cx)
    }
}
impl Future for ReadRepeat<'_> {
    type Output = Result<()>;

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let remaining = self.buf.remaining();
        unsafe {
            self.buf
                .unfilled_mut()
                .as_mut_ptr()
                .write_bytes(self.byte, remaining);
            self.buf.assume_init(remaining);
        };
        self.buf.add_filled(remaining);
        Poll::Ready(Ok(()))
    }
}

#[test]
fn test_read_repeat() {
    let mut bytes = [MaybeUninit::uninit(); 13];
    let mut buf = ReadBuf::uninit(&mut bytes);

    futures_lite::future::block_on(std::io::repeat(185).read(buf.as_mut())).unwrap();

    assert_eq!(buf.into_filled(), &[185; 13]);
}

impl<'a, 's> AsyncReadWith<'a> for &'s [u8] {
    type ReadFuture = ReadSlice<'a, 's>;

    fn read(&'a mut self, buf: ReadBufMut<'a>) -> Self::ReadFuture {
        ReadSlice {
            // Safety: We are extending the lifetime of the reference from 'a to 's. This is safe
            // because the struct it is in only lives for as long as 'a.
            slice: unsafe { &mut *(self as *mut _) },
            buf,
        }
    }
}

/// Future for [`read`](AsyncReadWith::read) on a byte slice (`&[u8]`).
#[derive(Debug)]
pub struct ReadSlice<'a, 's> {
    // This is conceptually an &'a mut &'s [u8]. However, that would add the implicit bound 's: 'a
    // which is incompatible with AsyncReadWith.
    slice: &'s mut &'s [u8],
    buf: ReadBufMut<'a>,
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
impl CompletionFuture for ReadSlice<'_, '_> {
    type Output = Result<()>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Future::poll(self, cx)
    }
}

#[test]
fn test_read_slice() {
    futures_lite::future::block_on(async {
        let mut bytes = [MaybeUninit::uninit(); 7];
        let mut buf = ReadBuf::uninit(&mut bytes);

        let mut slice: &[u8] = &[1, 2, 3, 4, 5];
        slice.read(buf.as_mut()).await.unwrap();

        assert_eq!(slice, &[]);
        assert_eq!(buf.as_mut().filled(), &[1, 2, 3, 4, 5]);

        let mut slice: &[u8] = &[6, 7, 8, 9, 10];
        slice.read(buf.as_mut()).await.unwrap();

        assert_eq!(slice, &[8, 9, 10]);
        assert_eq!(buf.as_mut().filled(), &[1, 2, 3, 4, 5, 6, 7]);
    });
}

impl<'a, T: AsRef<[u8]>> AsyncReadWith<'a> for Cursor<T> {
    type ReadFuture = ReadCursor<'a, T>;

    fn read(&'a mut self, buf: ReadBufMut<'a>) -> Self::ReadFuture {
        ReadCursor { cursor: self, buf }
    }
}

/// Future for [`read`](AsyncReadWith::read) on a [`Cursor`].
#[derive(Debug)]
pub struct ReadCursor<'a, T> {
    // This is conceptually an &'a mut Cursor<T>. However, that would add the implicit bound T: 'a
    // which is incompatible with AsyncReadWith.
    cursor: *mut Cursor<T>,
    buf: ReadBufMut<'a>,
}
// ReadBufMut is always Send+Sync, and we hold a mutable reference to Cursor.
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
impl<T: AsRef<[u8]>> CompletionFuture for ReadCursor<'_, T> {
    type Output = Result<()>;

    unsafe fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Future::poll(self, cx)
    }
}

#[test]
fn test_read_cursor() {
    futures_lite::future::block_on(async {
        let mut bytes = [MaybeUninit::uninit(); 7];
        let mut buf = ReadBuf::uninit(&mut bytes);

        let mut cursor = Cursor::new(vec![1, 2, 3, 4, 5]);
        cursor.read(buf.as_mut()).await.unwrap();
        assert_eq!(cursor.position(), 5);
        assert_eq!(buf.as_mut().filled(), &[1, 2, 3, 4, 5]);

        let mut cursor = Cursor::new(vec![6, 7, 8, 9, 10]);
        cursor.read(buf.as_mut()).await.unwrap();
        assert_eq!(cursor.position(), 2);
        assert_eq!(buf.as_mut().filled(), &[1, 2, 3, 4, 5, 6, 7]);
    });
}

#[cfg(test)]
#[allow(dead_code, clippy::extra_unused_lifetimes)]
fn test_impls_traits<'a>() {
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
}

/// Macro to define the commend methods in both `ReadBuf` and `ReadBufMut`.
macro_rules! common_read_buf_methods {
    ($get:expr, $get_mut:expr $(,)?) => {
        /// Get the total capacity of the buffer.
        #[inline]
        #[must_use]
        pub fn capacity(&self) -> usize {
            $get(self).data.len()
        }

        /// Get a shared reference to the filled portion of the buffer.
        #[inline]
        #[must_use]
        pub fn filled(&self) -> &[u8] {
            let buf = $get(self);
            unsafe { &*(buf.data.get_unchecked(..buf.filled) as *const _ as *const _) }
        }

        /// Get a mutable reference to the filled portion of the buffer.
        #[inline]
        #[must_use]
        pub fn filled_mut(&mut self) -> &mut [u8] {
            let buf = unsafe { $get_mut(self) };
            unsafe { &mut *(buf.data.get_unchecked_mut(..buf.filled) as *mut _ as *mut _) }
        }

        /// Get a shared reference to the initialized portion of the buffer.
        ///
        /// This includes the filled portion.
        #[inline]
        #[must_use]
        pub fn initialized(&self) -> &[u8] {
            let buf = $get(self);
            unsafe { &*(buf.data.get_unchecked(..buf.initialized) as *const _ as *const _) }
        }

        /// Get a mutable reference to the initialized portion of the buffer.
        ///
        /// This includes the filled portion.
        #[inline]
        pub fn initialized_mut(&mut self) -> &mut [u8] {
            let buf = unsafe { $get_mut(self) };
            unsafe { &mut *(buf.data.get_unchecked_mut(..buf.initialized) as *mut _ as *mut _) }
        }

        /// Get a mutable reference to the unfilled part of the buffer without ensuring that it has
        /// been fully initialized.
        ///
        /// # Safety
        ///
        /// The caller must not deinitialize portions of the buffer that have already been
        /// initialized.
        #[inline]
        #[must_use]
        pub unsafe fn unfilled_mut(&mut self) -> &mut [MaybeUninit<u8>] {
            let buf = $get_mut(self);
            buf.data.get_unchecked_mut(buf.filled..)
        }

        /// Get a shared reference to the entire backing buffer.
        #[inline]
        #[must_use]
        pub fn all(&self) -> &[MaybeUninit<u8>] {
            $get(self).data
        }

        /// Get a mutable reference to the entire backing buffer.
        ///
        /// # Safety
        ///
        /// The caller must not deinitialize portions of the buffer that have already been
        /// initialized.
        #[inline]
        #[must_use]
        pub unsafe fn all_mut(&mut self) -> &mut [MaybeUninit<u8>] {
            $get_mut(self).data
        }

        /// Get a mutable reference to the unfilled part of the buffer, ensuring it is fully
        ///
        /// initialized.
        ///
        /// Since `ReadBuf` tracks the region of the buffer that has been initialized, this is
        /// effectively "free" after the first use.
        #[inline]
        pub fn initialize_unfilled(&mut self) -> &mut [u8] {
            self.initialize_unfilled_to(self.remaining())
        }

        /// Get a mutable reference to the first `n` bytes of the unfilled part of the buffer, ensuring
        /// it is fully initialized.
        ///
        /// # Panics
        ///
        /// Panics if `self.remaining()` is less than `n`.
        #[inline]
        pub fn initialize_unfilled_to(&mut self, n: usize) -> &mut [u8] {
            assert!(
                self.remaining() >= n,
                "attempted to obtain more bytes than the buffer's capacity"
            );

            let buf = unsafe { $get_mut(self) };
            let end = buf.filled + n;

            if buf.initialized < end {
                unsafe {
                    buf.data
                        .get_unchecked_mut(buf.initialized)
                        .as_mut_ptr()
                        .write_bytes(0, end - buf.initialized);
                }
                buf.initialized = end;
            }

            unsafe { &mut *(buf.data.get_unchecked_mut(buf.filled..end) as *mut _ as *mut _) }
        }

        /// Get the number of bytes at the end of the slice that have not yet been filled.
        #[inline]
        #[must_use]
        pub fn remaining(&self) -> usize {
            self.capacity() - $get(self).filled
        }

        /// Clear the buffer, resetting the filled region to empty.
        ///
        /// The number of initialized bytes is not changed, and the contents of the buffer is not
        /// modified.
        #[inline]
        pub fn clear(&mut self) {
            unsafe { $get_mut(self) }.filled = 0;
        }

        /// Increase the size of the filled region of the buffer by `n` bytes.
        ///
        /// The number of initialized bytes is not changed.
        ///
        /// # Panics
        ///
        /// Panics if the filled region of the buffer would become larger than the initialized region.
        #[inline]
        pub fn add_filled(&mut self, n: usize) {
            let filled = $get(&*self).filled.checked_add(n).expect(
                "attempted to increase the filled region of the buffer beyond the integer limit",
            );
            self.set_filled(filled);
        }

        /// Set the size of the filled region of the buffer to `n`.
        ///
        /// The number of initialized bytes is not changed.
        ///
        /// Note that this can be used to *shrink* the filled region of the buffer in addition to
        /// growing it (for example, by a `Read` implementation that compresses data in-place).
        ///
        /// # Panics
        ///
        /// Panics if the filled region of the buffer would become larger than the initialized region.
        #[inline]
        pub fn set_filled(&mut self, n: usize) {
            let buf = unsafe { $get_mut(self) };
            assert!(
                n <= buf.initialized,
                "attempted to increase the filled region of the buffer beyond initialized region"
            );

            buf.filled = n;
        }

        /// Asserts that the first `n` unfilled bytes of the buffer are initialized.
        ///
        /// `ReadBuf` assumes that bytes are never deinitialized, so this method does nothing when
        /// called with fewer bytes than are already known to be initialized.
        ///
        /// # Safety
        ///
        /// The caller must ensure that the first `n` unfilled bytes of the buffer have already been
        /// initialized.
        #[inline]
        pub unsafe fn assume_init(&mut self, n: usize) {
            let buf = $get_mut(self);
            let new = buf.filled + n;
            if new > buf.initialized {
                buf.initialized = n;
            }
        }

        /// Appends data to the buffer, advancing the written position and possibly also the initialized
        /// position.
        ///
        /// # Panics
        ///
        /// Panics if `self.remaining()` is less than `other.len()`.
        #[inline]
        pub fn append(&mut self, other: &[u8]) {
            assert!(
                self.remaining() >= other.len(),
                "attempted to append more bytes to the buffer than it has capacity for",
            );

            let buf = unsafe { $get_mut(self) };

            let end = buf.filled + other.len();

            unsafe {
                buf.data
                    .get_unchecked_mut(buf.filled..end)
                    .as_mut_ptr()
                    .cast::<u8>()
                    .copy_from_nonoverlapping(other.as_ptr(), other.len())
            }

            if buf.initialized < end {
                buf.initialized = end;
            }
            buf.filled = end;
        }
    };
}

/// A wrapper around a byte buffer that is incrementally filled and initialized.
pub struct ReadBuf<'a> {
    data: &'a mut [MaybeUninit<u8>],
    /// The index up to which the buffer has been filled with meaningful data.
    filled: usize,
    /// The index up to which the buffer's data is initialized with useless data.
    initialized: usize,
}

impl<'a> ReadBuf<'a> {
    /// Create a new `ReadBuf` from a fully initialized buffer.
    #[inline]
    pub fn new(buf: &'a mut [u8]) -> Self {
        let initialized = buf.len();
        Self {
            data: unsafe { &mut *(buf as *mut _ as *mut [MaybeUninit<u8>]) },
            filled: 0,
            initialized,
        }
    }

    /// Create a new `ReadBuf` from a fully uninitialized buffer.
    ///
    /// Use [`assume_init`](ReadBufMut::assume_init) if part of the buffer is known to be already
    /// initialized.
    #[inline]
    pub fn uninit(data: &'a mut [MaybeUninit<u8>]) -> ReadBuf<'a> {
        Self {
            data,
            filled: 0,
            initialized: 0,
        }
    }

    /// Get a mutable reference to this buffer as a [`ReadBufMut`].
    #[inline]
    pub fn as_mut(&mut self) -> ReadBufMut<'_> {
        ReadBufMut {
            buf: NonNull::from(self),
            _covariant: PhantomData,
        }
    }

    /// Consume the buffer, returning the entire partially initialized backing slice.
    #[inline]
    #[must_use]
    pub fn into_all(self) -> &'a mut [MaybeUninit<u8>] {
        self.data
    }

    /// Consume the buffer, returning its three parts, the filled portion, the unfilled portion and
    /// the uninitialized portion.
    #[inline]
    #[must_use]
    pub fn into_parts(self) -> (&'a mut [u8], &'a mut [u8], &'a mut [MaybeUninit<u8>]) {
        let len = self.data.len();
        let ptr = self.data.as_mut_ptr();

        unsafe {
            (
                slice::from_raw_parts_mut(ptr as *mut u8, self.filled),
                slice::from_raw_parts_mut(
                    ptr.add(self.filled) as *mut u8,
                    self.initialized - self.filled,
                ),
                slice::from_raw_parts_mut(ptr.add(self.initialized), len - self.initialized),
            )
        }
    }

    /// Consume the buffer, returning its filled portion.
    #[inline]
    #[must_use]
    pub fn into_filled(self) -> &'a mut [u8] {
        unsafe { &mut *(self.data.get_unchecked_mut(..self.filled) as *mut _ as *mut _) }
    }
}

/// These methods are also present on [`ReadBufMut`].
#[allow(unused_unsafe)]
impl ReadBuf<'_> {
    common_read_buf_methods!(std::convert::identity, std::convert::identity);
}

impl Debug for ReadBuf<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        struct InitializedByte(u8);
        impl Debug for InitializedByte {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                write!(f, "({})", self.0)
            }
        }
        struct Uninit;
        impl Debug for Uninit {
            fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
                f.write_str("-")
            }
        }

        let mut list = f.debug_list();

        for (i, byte) in self.data.iter().enumerate() {
            if i < self.filled {
                list.entry(&unsafe { byte.assume_init() });
            } else if i < self.initialized {
                list.entry(&InitializedByte(unsafe { byte.assume_init() }));
            } else {
                list.entry(&Uninit);
            }
        }

        list.finish()
    }
}

/// A type that grants mutable access to a [`ReadBuf`].
///
/// You can create this by calling [`ReadBuf::as_mut`].
pub struct ReadBufMut<'a> {
    /// This is a `NonNull` to allow `'a` to be covariant. As a safety invariant, this must _never_
    /// be moved out of and the inner buffer's pointer to the bytes must never be changed (as the
    /// `'a` lifetime could be shorter than the actual lifetime of the `ReadBuf`/bytes).
    buf: NonNull<ReadBuf<'a>>,
    /// Even though we hold a mutable reference to `ReadBuf`, it is safe to be covariant as we
    /// never reassign the buffer, or change the buffer's pointer to the bytes.
    _covariant: PhantomData<&'a ()>,
}

// We effectively hold an `&'a mut ReadBuf<'a>` which is Send and Sync.
unsafe impl Send for ReadBufMut<'_> {}
unsafe impl Sync for ReadBufMut<'_> {}

impl<'a> ReadBufMut<'a> {
    /// Get a shared reference to the internal buffer.
    #[inline]
    #[must_use]
    pub fn buf(&self) -> &ReadBuf<'a> {
        unsafe {
            // Safety: You cannot move out of a shared reference.
            self.buf.as_ref()
        }
    }

    /// Get a mutable reference to the internal buffer.
    ///
    /// # Safety
    ///
    /// This must not be moved out of, and the buffer's pointer to the bytes must not be changed.
    #[inline]
    pub unsafe fn buf_mut(&mut self) -> &mut ReadBuf<'a> {
        self.buf.as_mut()
    }

    /// Convert this type to a mutable reference to the internal buffer.
    ///
    /// # Safety
    ///
    /// This must not be moved out of, and the buffer's pointer to the bytes must not be changed.
    #[inline]
    #[must_use]
    pub unsafe fn into_mut(self) -> &'a mut ReadBuf<'a> {
        &mut *self.buf.as_ptr()
    }

    /// Borrow the buffer, rather than consuming it.
    #[inline]
    #[must_use]
    pub fn as_mut(&mut self) -> ReadBufMut<'_> {
        ReadBufMut {
            buf: self.buf,
            _covariant: PhantomData,
        }
    }
}

/// These methods are also present on [`ReadBuf`].
impl ReadBufMut<'_> {
    common_read_buf_methods!(Self::buf, Self::buf_mut,);
}

impl Debug for ReadBufMut<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.buf().fmt(f)
    }
}
