use std::fmt::{self, Debug, Formatter};
use std::mem::MaybeUninit;
use std::slice;

use super::co_mut::CoMut;
use super::{buf_assume_init_mut, buf_assume_init_ref};

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
            data: unsafe { slice::from_raw_parts_mut(buf.as_mut_ptr().cast(), buf.len()) },
            filled: 0,
            initialized,
        }
    }

    /// Create a new `ReadBuf` from a fully uninitialized buffer.
    ///
    /// Use [`assume_init`](Self::assume_init) if part of the buffer is known to be already
    /// initialized.
    #[inline]
    pub fn uninit(data: &'a mut [MaybeUninit<u8>]) -> ReadBuf<'a> {
        Self {
            data,
            filled: 0,
            initialized: 0,
        }
    }

    /// Get a mutable reference to this `ReadBuf` as a [`ReadBufRef`].
    #[must_use]
    #[inline]
    pub fn as_ref(&mut self) -> ReadBufRef<'_> {
        ReadBufRef {
            buf: CoMut::from(self),
        }
    }

    /// Consume the buffer, returning its filled portion.
    #[must_use]
    #[inline]
    pub fn into_filled(self) -> &'a mut [u8] {
        unsafe { buf_assume_init_mut(self.data.get_unchecked_mut(..self.filled)) }
    }

    /// Consume the buffer, returning the entire partially initialized backing slice.
    #[must_use]
    #[inline]
    pub fn into_inner(self) -> &'a mut [MaybeUninit<u8>] {
        self.data
    }
}

/// Macro to define common methods between `ReadBuf` and `ReadBufRef`.
macro_rules! common_read_buf_methods {
    ($get:expr, $get_mut:expr) => {
        /// Get the total capacity of the buffer.
        #[must_use]
        #[inline]
        pub fn capacity(&self) -> usize {
            $get(self).data.len()
        }

        /// Get a shared reference to the filled portion of the buffer.
        #[must_use]
        #[inline]
        pub fn filled(&self) -> &[u8] {
            let this = $get(self);
            unsafe { buf_assume_init_ref(this.data.get_unchecked(..this.filled)) }
        }

        /// Get a mutable reference to the filled portion of the buffer.
        #[must_use]
        #[inline]
        pub fn filled_mut(&mut self) -> &mut [u8] {
            let this = unsafe { $get_mut(self) };
            unsafe { buf_assume_init_mut(this.data.get_unchecked_mut(..this.filled)) }
        }

        /// Get a shared reference to the initialized portion of the buffer.
        ///
        /// This includes the filled portion.
        #[must_use]
        #[inline]
        pub fn initialized(&self) -> &[u8] {
            let this = $get(self);
            unsafe { buf_assume_init_ref(this.data.get_unchecked(..this.initialized)) }
        }

        /// Get a mutable reference to the initialized portion of the buffer.
        ///
        /// This includes the filled portion.
        #[must_use]
        #[inline]
        pub fn initialized_mut(&mut self) -> &mut [u8] {
            let this = unsafe { $get_mut(self) };
            unsafe { buf_assume_init_mut(this.data.get_unchecked_mut(..this.initialized)) }
        }

        /// Get a shared reference to the entire partially initialized backing slice.
        #[must_use]
        #[inline]
        pub fn inner(&self) -> &[MaybeUninit<u8>] {
            $get(self).data
        }

        /// Get a mutable reference to the entire partially initialized backing slice.
        ///
        /// # Safety
        ///
        /// The caller must not deinitialize portions of the buffer that have already been initialized.
        #[must_use]
        #[inline]
        pub unsafe fn inner_mut(&mut self) -> &mut [MaybeUninit<u8>] {
            unsafe { $get_mut(self) }.data
        }

        /// Get a mutable reference to the unfilled part of the buffer without ensuring that it has
        /// been fully initialized.
        ///
        /// # Safety
        ///
        /// The caller must not deinitialize portions of the buffer that have already been initialized.
        #[must_use]
        #[inline]
        pub unsafe fn unfilled_mut(&mut self) -> &mut [MaybeUninit<u8>] {
            let this = unsafe { $get_mut(self) };
            unsafe { this.data.get_unchecked_mut(this.filled..) }
        }

        /// Get a mutable reference to the unfilled part of the buffer, ensuring it is fully
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

            let this = unsafe { $get_mut(self) };

            let end = this.filled + n;

            if this.initialized < end {
                unsafe { this.data.get_unchecked_mut(this.initialized..end) }
                    .fill(MaybeUninit::new(0));
                this.initialized = end;
            }

            unsafe { buf_assume_init_mut(this.data.get_unchecked_mut(this.filled..end)) }
        }

        /// Get the number of bytes at the end of the slice that have not yet been filled.
        #[must_use]
        #[inline]
        pub fn remaining(&self) -> usize {
            let this = $get(self);
            this.capacity() - this.filled
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
        #[track_caller]
        #[inline]
        pub fn add_filled(&mut self, n: usize) {
            let this = unsafe { $get_mut(self) };
            let filled = this.filled.checked_add(n).expect(
                "attempted to increase the filled region of the buffer beyond the integer limit",
            );
            this.set_filled(filled);
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
        #[track_caller]
        #[inline]
        pub fn set_filled(&mut self, n: usize) {
            let this = unsafe { $get_mut(self) };
            assert!(
                n <= this.initialized,
                "attempted to increase the filled region of the buffer beyond initialized region"
            );
            this.filled = n;
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
            let this = unsafe { $get_mut(self) };
            let new = this.filled + n;
            if new > this.initialized {
                this.initialized = new;
            }
        }

        /// Appends data to the buffer, advancing the written position and possibly also the initialized
        /// position.
        ///
        /// # Panics
        ///
        /// Panics if `self.remaining()` is less than `other.len()`.
        #[track_caller]
        #[inline]
        pub fn append(&mut self, other: &[u8]) {
            assert!(
                self.remaining() >= other.len(),
                "attempted to append more bytes to the buffer than it has capacity for",
            );

            for (dest, &byte) in unsafe { self.unfilled_mut() }.iter_mut().zip(other) {
                *dest = MaybeUninit::new(byte);
            }

            unsafe { self.assume_init(other.len()) };

            self.add_filled(other.len());
        }
    };
}

/// These methods are also present on [`ReadBufRef`].
#[allow(unused_unsafe)]
impl ReadBuf<'_> {
    common_read_buf_methods!(std::convert::identity, std::convert::identity);
}

/// Helper for implementing `Debug`. Also used by `ReadBufs`.
pub(super) struct PartialInitSlice<'a> {
    pub(super) data: &'a [MaybeUninit<u8>],
    pub(super) filled: usize,
    pub(super) initialized: usize,
}
impl Debug for PartialInitSlice<'_> {
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

impl Debug for ReadBuf<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        PartialInitSlice {
            data: &self.data,
            filled: self.filled,
            initialized: self.initialized,
        }
        .fmt(f)
    }
}

/// A type that grants mutable access to a [`ReadBuf`].
///
/// You can create this by calling [`ReadBuf::as_ref`].
pub struct ReadBufRef<'a> {
    buf: CoMut<'a, ReadBuf<'a>>,
}

impl<'a> ReadBufRef<'a> {
    /// Get a shared reference to the internal [`ReadBuf`].
    #[must_use]
    #[inline]
    pub fn buf(&self) -> &ReadBuf<'a> {
        &self.buf
    }

    /// Get a mutable reference to the internal [`ReadBuf`].
    ///
    /// # Safety
    ///
    /// The [`ReadBuf`] must not be replaced through this mutable reference.
    #[must_use]
    #[inline]
    pub unsafe fn buf_mut(&mut self) -> &mut ReadBuf<'a> {
        unsafe { self.buf.get_mut() }
    }

    /// Consume this `ReadBufRef`, retrieving the underlying mutable reference to a [`ReadBuf`].
    ///
    /// # Safety
    ///
    /// The [`ReadBuf`] must not be replaced through this mutable reference.
    #[must_use]
    #[inline]
    pub unsafe fn into_mut(self) -> &'a mut ReadBuf<'a> {
        unsafe { self.buf.into_inner() }
    }

    /// Reborrow the `ReadBufRef` to a shorter lifetime.
    #[must_use]
    #[inline]
    pub fn as_ref(&mut self) -> ReadBufRef<'_> {
        unsafe { self.buf_mut() }.as_ref()
    }
}

/// These methods are also present on [`ReadBuf`].
impl ReadBufRef<'_> {
    common_read_buf_methods!(Self::buf, Self::buf_mut);
}

impl Debug for ReadBufRef<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.buf().fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use std::mem::{self, MaybeUninit};
    use std::panic;

    use crate::read::{buf_assume_init_mut, buf_assume_init_ref};

    use super::ReadBuf;

    fn uninit_array<T, const N: usize>(array: [T; N]) -> [MaybeUninit<T>; N] {
        let res = unsafe { mem::transmute_copy(&array) };
        mem::forget(array);
        res
    }

    #[test]
    fn new() {
        let mut data = [1, 2, 3, 4];
        let buf = ReadBuf::new(&mut data);
        assert_eq!(unsafe { buf_assume_init_mut(buf.data) }, &mut [1, 2, 3, 4]);
        assert_eq!(buf.filled, 0);
        assert_eq!(buf.initialized, 4);
    }

    #[test]
    fn uninit() {
        let mut data = [MaybeUninit::uninit(), MaybeUninit::new(2)];
        let buf = ReadBuf::uninit(&mut data);
        assert_eq!(buf.data.len(), 2);
        assert_eq!(unsafe { buf.data[1].assume_init() }, 2);
        assert_eq!(buf.filled, 0);
        assert_eq!(buf.initialized, 0);
    }

    #[test]
    fn capacity() {
        assert_eq!(ReadBuf::new(&mut []).capacity(), 0);
        assert_eq!(ReadBuf::new(&mut [0; 5]).capacity(), 5);
        assert_eq!(ReadBuf::uninit(&mut []).capacity(), 0);
        assert_eq!(
            ReadBuf::uninit(&mut [MaybeUninit::uninit(); 7]).capacity(),
            7
        );
    }

    #[test]
    fn get_filled() {
        let mut data = [1, 2, 3, 4, 5];
        let mut buf = ReadBuf::new(&mut data);
        assert_eq!(buf.filled(), &[]);
        assert_eq!(buf.filled_mut(), &mut []);
        buf.filled = 2;
        assert_eq!(buf.filled(), &[1, 2]);
        assert_eq!(buf.filled_mut(), &mut [1, 2]);
        assert_eq!(buf.into_filled(), &mut [1, 2]);
    }

    #[test]
    fn get_initialized() {
        let mut data = uninit_array([1, 2, 3, 4, 5]);
        let mut buf = ReadBuf::uninit(&mut data);
        assert_eq!(buf.initialized(), &[]);
        assert_eq!(buf.initialized_mut(), &mut []);
        buf.initialized += 2;
        assert_eq!(buf.initialized(), &[1, 2]);
        assert_eq!(buf.initialized_mut(), &mut [1, 2]);
    }

    #[test]
    fn get_inner() {
        let data = [1, 2, 3, 4, 5, 6];
        let mut backing = data;
        let mut buf = ReadBuf::new(&mut backing);
        buf.filled = 1;
        buf.initialized = 2;
        assert_eq!(unsafe { buf_assume_init_ref(buf.inner()) }, &data);
        assert_eq!(unsafe { buf_assume_init_mut(buf.inner_mut()) }, &data);
        assert_eq!(unsafe { buf_assume_init_ref(buf.into_inner()) }, &data);
    }

    #[test]
    fn unfilled() {
        let mut data = [1, 2, 3, 4];
        let mut buf = ReadBuf::new(&mut data);
        assert_eq!(
            unsafe { buf_assume_init_ref(buf.unfilled_mut()) },
            [1, 2, 3, 4]
        );
        buf.filled = 2;
        assert_eq!(unsafe { buf_assume_init_ref(buf.unfilled_mut()) }, [3, 4]);
    }

    #[test]
    fn initialize_unfilled() {
        let mut data = uninit_array([1, 2, 3, 4, 5]);
        let mut buf = ReadBuf {
            data: &mut data,
            filled: 1,
            initialized: 3,
        };
        assert_eq!(buf.initialize_unfilled_to(3), &mut [2, 3, 0]);
        assert_eq!(buf.filled, 1);
        assert_eq!(buf.initialized, 4);
        assert_eq!(unsafe { buf.data[4].assume_init() }, 5);
        assert_eq!(buf.initialize_unfilled(), &mut [2, 3, 0, 0]);
        assert_eq!(unsafe { buf.data[0].assume_init() }, 1);
        assert_eq!(buf.filled, 1);
        assert_eq!(buf.initialized, 5);
    }

    #[test]
    fn remaining() {
        let mut data = [0; 5];
        let mut buf = ReadBuf::new(&mut data);
        assert_eq!(buf.remaining(), 5);
        buf.filled += 5;
        assert_eq!(buf.remaining(), 0);
    }

    #[test]
    fn set_filled() {
        let mut data = [1, 2, 3, 4, 5];
        let mut buf = ReadBuf::new(&mut data);

        buf.set_filled(3);
        assert_eq!(buf.filled(), &[1, 2, 3]);

        buf.add_filled(2);
        assert_eq!(buf.filled(), &[1, 2, 3, 4, 5]);

        buf.set_filled(1);
        assert_eq!(buf.filled(), &[1]);

        buf.add_filled(0);
        assert_eq!(buf.filled(), &[1]);

        buf.clear();
        assert_eq!(buf.filled(), &[]);

        panic::catch_unwind(panic::AssertUnwindSafe(|| buf.add_filled(10))).unwrap_err();
        assert_eq!(buf.filled(), &[]);
    }

    #[test]
    fn assume_init() {
        let mut data = uninit_array([1, 2, 3, 4]);
        let mut buf = ReadBuf {
            data: &mut data,
            filled: 1,
            initialized: 2,
        };

        unsafe { buf.assume_init(2) };
        assert_eq!(buf.initialized(), &[1, 2, 3]);

        unsafe { buf.assume_init(0) };
        assert_eq!(buf.initialized(), &[1, 2, 3]);

        unsafe { buf.assume_init(3) };
        assert_eq!(buf.initialized(), &[1, 2, 3, 4]);
    }

    #[test]
    fn append() {
        let mut data = [MaybeUninit::uninit(); 6];
        let mut buf = ReadBuf::uninit(&mut data);

        buf.append(&[1, 2, 3, 4]);
        assert_eq!(buf.filled(), &[1, 2, 3, 4]);
        assert_eq!(buf.initialized, buf.filled);

        buf.append(&[]);
        assert_eq!(buf.filled(), &[1, 2, 3, 4]);
        assert_eq!(buf.initialized, buf.filled);

        panic::catch_unwind(panic::AssertUnwindSafe(|| buf.append(&[9, 9, 9, 9]))).unwrap_err();
        assert_eq!(buf.filled(), &[1, 2, 3, 4]);
        assert_eq!(buf.initialized, buf.filled);

        buf.append(&[5, 6]);
        assert_eq!(buf.filled(), &[1, 2, 3, 4, 5, 6]);
        assert_eq!(buf.initialized, buf.filled);

        buf.append(&[]);
        assert_eq!(buf.filled(), &[1, 2, 3, 4, 5, 6]);
        assert_eq!(buf.initialized, buf.filled);
    }

    #[test]
    fn debug() {
        let mut data = uninit_array([1, 2, 3, 4, 5, 6]);
        let buf = ReadBuf {
            data: &mut data,
            filled: 2,
            initialized: 4,
        };
        assert_eq!(format!("{:?}", buf), "[1, 2, (3), (4), -, -]");
    }
}
