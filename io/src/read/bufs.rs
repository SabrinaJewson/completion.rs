use std::cmp;
use std::fmt::{self, Debug, Formatter};
use std::io::IoSliceMut;
use std::mem::MaybeUninit;
use std::slice;

use super::co_mut::CoMut;
use super::MaybeUninitIoSliceMut;
use super::{buf_assume_init_mut, buf_assume_init_ref, bufs_assume_init_mut, bufs_assume_init_ref};

/// A wrapper around a set of byte buffers that are incrementally filled and initialized.
pub struct ReadBufs<'a, 'b> {
    data: &'a mut [MaybeUninitIoSliceMut<'b>],
    /// The number of buffers that have been fully filled. This is always accurate.
    filled_bufs: usize,
    /// How filled the partially filled buffer is. This is less than the length of the buffer unless
    /// the user has replaced the partially filled buffer with a smaller one.
    filled_to: usize,
    /// The number of buffers that have been fully initialized. This is always accurate and >=
    /// `filled_bufs`.
    initialized_bufs: usize,
    /// How initialized the partially initialized buffer is. This is less than the length of the
    /// buffer unless the user has replaced the partially initialized buffer with a smaller one.
    initialized_to: usize,
}

impl<'a, 'b> ReadBufs<'a, 'b> {
    /// Create a new `ReadBufs` from a set of fully initialized buffers.
    #[inline]
    pub fn new(bufs: &'a mut [IoSliceMut<'b>]) -> Self {
        let filled_bufs = bufs
            .iter()
            .position(|buf| !buf.is_empty())
            .unwrap_or(bufs.len());
        let initialized_bufs = bufs.len();
        Self {
            data: unsafe { slice::from_raw_parts_mut(<*mut _>::cast(bufs), bufs.len()) },
            filled_bufs,
            filled_to: 0,
            initialized_bufs,
            initialized_to: 0,
        }
    }

    /// Create a new `ReadBufs` from a set of fully uninitialized buffers.
    ///
    /// Use [`assume_init`](Self::assume_init) if part of the buffers are known to be already
    /// initialized.
    #[inline]
    pub fn uninit(bufs: &'a mut [MaybeUninitIoSliceMut<'b>]) -> Self {
        let first_nonempty_buf = bufs
            .iter()
            .position(|buf| !buf.is_empty())
            .unwrap_or(bufs.len());
        Self {
            data: bufs,
            filled_bufs: first_nonempty_buf,
            filled_to: 0,
            initialized_bufs: first_nonempty_buf,
            initialized_to: 0,
        }
    }

    /// Get a mutable reference to this `ReadBufs` as a [`ReadBufsRef`].
    #[must_use]
    #[inline]
    pub fn as_ref(&mut self) -> ReadBufsRef<'_, 'b> {
        ReadBufsRef {
            bufs: CoMut::from(self),
        }
    }

    unsafe fn get_to(&self, buf: usize, to: usize) -> (&[IoSliceMut<'b>], &[u8]) {
        let bufs = unsafe { bufs_assume_init_ref(self.data.get_unchecked(..buf)) };
        let slice = self
            .data
            .get(buf)
            .map(|buf| {
                debug_assert!(to < buf.len());
                unsafe { buf_assume_init_ref(&buf[..to]) }
            })
            .unwrap_or_default();
        (bufs, slice)
    }

    unsafe fn get_to_mut(&mut self, buf: usize, to: usize) -> (&mut [IoSliceMut<'b>], &mut [u8]) {
        let (before_bufs, after_bufs) = self.data.split_at_mut(buf);
        let bufs = unsafe { bufs_assume_init_mut(before_bufs) };
        let slice = after_bufs
            .first_mut()
            .map(|buf| {
                debug_assert!(to < buf.len());
                unsafe { buf_assume_init_mut(&mut buf[..to]) }
            })
            .unwrap_or_default();
        (bufs, slice)
    }

    unsafe fn into_to(self, buf: usize, to: usize) -> (&'a mut [IoSliceMut<'b>], &'a mut [u8]) {
        let (before_bufs, after_bufs) = self.data.split_at_mut(buf);
        let bufs = unsafe { bufs_assume_init_mut(before_bufs) };
        let slice = after_bufs
            .first_mut()
            .map(|buf| {
                debug_assert!(to < buf.len());
                unsafe { buf_assume_init_mut(&mut buf[..to]) }
            })
            .unwrap_or_default();
        (bufs, slice)
    }

    unsafe fn get_from_mut(
        &mut self,
        buf: usize,
        from: usize,
    ) -> (&mut [MaybeUninit<u8>], &mut [MaybeUninitIoSliceMut<'b>]) {
        unsafe { self.data.get_unchecked_mut(buf..) }
            .split_first_mut()
            .map(|(before_buf, after_bufs)| {
                debug_assert!(from < before_buf.len());
                (&mut before_buf[from..], after_bufs)
            })
            .unwrap_or_default()
    }

    /// Consume the buffer, returning its filled portion.
    ///
    /// This returns a tuple of all the fully filled buffers as well as a slice of any data that
    /// wasn't able to fully fill a whole buffer. This slice may be empty, but it will never be the
    /// same size as the buffer it points to.
    #[must_use]
    #[inline]
    pub fn into_filled(self) -> (&'a mut [IoSliceMut<'b>], &'a mut [u8]) {
        let (filled_bufs, filled_to) = (self.filled_bufs, self.filled_to);
        unsafe { self.into_to(filled_bufs, filled_to) }
    }

    /// Consume the `ReadBufs`, returning all the partially initialized backing buffers.
    #[must_use]
    #[inline]
    pub fn into_inner(self) -> &'a mut [MaybeUninitIoSliceMut<'b>] {
        self.data
    }

    fn add_to_index_pair(&self, mut buf: usize, to: usize, mut n: usize) -> Option<(usize, usize)> {
        if n == 0 {
            return Some((buf, to));
        }

        let buf_len = self.inner().get(buf)?.len();
        if n < buf_len - to {
            return Some((buf, n + to));
        }
        n -= buf_len - to;
        buf += 1;

        loop {
            let buf_len = match self.inner().get(buf) {
                Some(buf) => buf.len(),
                None if n == 0 => return Some((buf, 0)),
                None => return None,
            };
            if n < buf_len {
                return Some((buf, n));
            }
            n -= buf_len;
            buf += 1;
        }
    }
}

macro_rules! common_read_bufs_methods {
    ($get:expr, $get_mut:expr) => {
        // TODO: Analogue to `ReadBuf::capacity`: should it return two usizes or a sum?

        /// Get a shared reference to the filled portion of the buffers.
        ///
        /// This returns a tuple of all the fully filled buffers as well as a slice of any data that
        /// wasn't able to fully fill a whole buffer. This slice may be empty, but it will never be
        /// the same size as the buffer it points to unless that buffer is empty.
        #[must_use]
        #[inline]
        pub fn filled(&self) -> (&[IoSliceMut<'b>], &[u8]) {
            let this = $get(self);
            unsafe { this.get_to(this.filled_bufs, this.filled_to) }
        }

        /// Get a mutable reference to the filled portion of the buffers.
        ///
        /// This returns a tuple of all the fully filled buffers as well as a slice of any data that
        /// wasn't able to fully fill a whole buffer. This slice may be empty, but it will never be
        /// the same size as the buffer it points to unless that buffer is empty.
        #[must_use]
        #[inline]
        pub fn filled_mut(&mut self) -> (&mut [IoSliceMut<'b>], &mut [u8]) {
            let this = unsafe { $get_mut(self) };
            unsafe { this.get_to_mut(this.filled_bufs, this.filled_to) }
        }

        /// Get a shared reference to the initialized portion of the buffer.
        ///
        /// This includes the filled portion.
        ///
        /// This returns a tuple of all the fully initialized buffers as well as a slice of any
        /// additional bytes that have been initialized, but not enough have to fill the next
        /// buffer.  This slice may be empty, but it will never be the same size as the buffer it
        /// points to unless that buffer is empty.
        #[must_use]
        #[inline]
        pub fn initialized(&self) -> (&[IoSliceMut<'b>], &[u8]) {
            let this = $get(self);
            unsafe { this.get_to(this.initialized_bufs, this.initialized_to) }
        }

        /// Get a mutable reference to the initialized portion of the buffer.
        ///
        /// This includes the filled portion.
        ///
        /// This returns a tuple of all the fully initialized buffers as well as a slice of any
        /// additional bytes that have been initialized, but not enough have to fill the next
        /// buffer.  This slice may be empty, but it will never be the same size as the buffer it
        /// points to unless that buffer is empty.
        #[must_use]
        #[inline]
        pub fn initialized_mut(&mut self) -> (&mut [IoSliceMut<'b>], &mut [u8]) {
            let this = unsafe { $get_mut(self) };
            unsafe { this.get_to_mut(this.initialized_bufs, this.initialized_to) }
        }

        /// Get a shared reference to all the partially initialized backing buffers.
        #[must_use]
        #[inline]
        pub fn inner(&self) -> &[MaybeUninitIoSliceMut<'b>] {
            $get(self).data
        }

        /// Get a mutable reference to all the partially initialized backing buffers.
        ///
        /// # Safety
        ///
        /// The caller must not deinitialize portions of the buffers that have already been
        /// initialized.
        #[must_use]
        #[inline]
        pub unsafe fn inner_mut(&mut self) -> &mut [MaybeUninitIoSliceMut<'b>] {
            unsafe { $get_mut(self) }.data
        }

        /// Get a mutable reference to the unfilled part of the buffers, without ensuring they have
        /// been fully initialized.
        ///
        /// This returns a tuple of the bytes in the latest buffer that have not been filled, as
        /// well as the remaining buffers that are completely empty. The first slice is only empty
        /// if there are no unfilled bytes or the user has swapped out one of the buffers contained
        /// in this `ReadBufs` (in which case it is OK to panic). It can be the length of an entire
        /// buffer.
        ///
        /// # Safety
        ///
        /// The caller must not deinitialize portions of the buffer that have already been
        /// initialized.
        #[must_use]
        #[inline]
        pub unsafe fn unfilled_mut(
            &mut self,
        ) -> (&mut [MaybeUninit<u8>], &mut [MaybeUninitIoSliceMut<'b>]) {
            let this = unsafe { $get_mut(self) };
            unsafe { this.get_from_mut(this.filled_bufs, this.filled_to) }
        }

        /// Get a mutable reference to the unfilled part of the buffers, ensuring they are fully
        /// initialized.
        ///
        /// Since `ReadBufs` tracks the region of the buffers that have been initialized, this is
        /// effectively "free" after the first use.
        #[inline]
        pub fn initialize_unfilled(&mut self) -> (&mut [u8], &mut [IoSliceMut<'b>]) {
            let this = unsafe { $get_mut(self) };

            let (uninit_buf, uninit_bufs) =
                unsafe { this.get_from_mut(this.initialized_bufs, this.initialized_to) };

            uninit_buf.fill(MaybeUninit::new(0));
            for buf in uninit_bufs {
                buf.fill(MaybeUninit::new(0));
            }

            this.initialized_bufs = this.data.len();
            this.initialized_to = 0;

            unsafe {
                let (init_buf, init_bufs) = this.unfilled_mut();
                (
                    buf_assume_init_mut(init_buf),
                    bufs_assume_init_mut(init_bufs),
                )
            }
        }

        // TODO: Analogue to `ReadBuf::initialize_unfilled_to`: should it take one or two usizes?

        /// Get the number of bytes at the end of the buffers that have not yet been filled.
        #[must_use]
        #[inline]
        pub fn remaining(&self) -> usize {
            let this = $get(self);

            unsafe { this.data.get_unchecked(this.filled_bufs..) }
                .split_first()
                .map(|(before_buf, after_bufs)| {
                    <_>::into_iter([before_buf.len() - this.filled_to])
                        .chain(after_bufs.iter().map(|buf| buf.len()))
                        .sum::<usize>()
                })
                .unwrap_or_default()
        }

        /// Clear the buffers, resetting the filled region to empty.
        ///
        /// The number of initialized bytes is not changed, and the contents of the buffers are not
        /// modified.
        #[inline]
        pub fn clear(&mut self) {
            let this = unsafe { $get_mut(self) };
            this.filled_bufs = 0;
            this.filled_to = 0;
        }

        /// Increase the size of the filled region of the buffer by `n` bytes.
        ///
        /// The number of initialized bytes is not changed.
        ///
        /// # Panics
        ///
        /// Panics if the filled region of the buffer would become larger than the initialized
        /// region.
        #[track_caller]
        #[inline]
        pub fn add_filled(&mut self, n: usize) {
            let this = unsafe { $get_mut(self) };
            let (filled_bufs, filled_to) = this
                .add_to_index_pair(this.filled_bufs, this.filled_to, n)
                .expect(
                    "attempted to increase the filled region of the buffers beyond the last buffer",
                );

            this.set_filled(filled_bufs, filled_to);
        }

        /// Set the size of the filled region of the buffers to `to` bytes in the `buf`th buffer.
        ///
        /// To mark the buffers as completely filled, set `buf` to the total number of buffers (one
        /// index past the end) and `to` to zero.
        ///
        /// The number of initialized bytes is not changed.
        ///
        /// Note that this can be used to *shrink* the filled region of the buffer in addition to
        /// growing it (for example, by a `Read` implementation that compresses data in-place).
        ///
        /// # Panics
        ///
        /// - Panics if the filled region of the buffers would become larger than the initialized
        /// region.
        /// - Panics if `to` is `>=` the length of the `buf`th buffer.
        #[track_caller]
        #[inline]
        pub fn set_filled(&mut self, buf: usize, to: usize) {
            let this = unsafe { $get_mut(self) };
            assert!(
                (buf, to) <= (this.initialized_bufs, this.initialized_to),
                "attempted to increase the filled region of the buffers beyond initialized region"
            );
            assert!(
                to < this.data.get(buf).map_or(1, |buf| buf.len()),
                "attempted to mark a buffer as filled beyond its length"
            );
            this.filled_bufs = buf;
            this.filled_to = to;
        }

        /// Asserts that the first `n` unfilled bytes of the buffers are initialized.
        ///
        /// `ReadBufs` assumes that bytes are never deinitialized, so this method does nothing when
        /// called with fewer bytes than are already known to be initialized.
        ///
        /// # Safety
        ///
        /// The caller must ensure that the first `n` unfilled bytes of the buffers have already
        /// been initialized.
        #[inline]
        // Panics only result as a violation of the "Safety" part of the doc.
        #[allow(clippy::missing_panics_doc)]
        pub unsafe fn assume_init(&mut self, n: usize) {
            let this = unsafe { $get_mut(self) };
            let (initialized_bufs, initialized_to) = this
                .add_to_index_pair(this.filled_bufs, this.filled_to, n)
                .unwrap();

            if (initialized_bufs, initialized_to) > (this.initialized_bufs, this.initialized_to) {
                this.initialized_bufs = initialized_bufs;
                this.initialized_to = initialized_to;
            }
        }

        /// Appends data to the buffers, advancing the written position and possibly also the
        /// initialized position.
        ///
        /// # Panics
        ///
        /// Panics if the buffers do not have capacity to hold the entire slice.
        #[track_caller]
        #[inline]
        pub fn append(&mut self, other: &[u8]) {
            assert!(
                self.remaining() >= other.len(),
                "attempted to append more bytes to the buffers than they have capacity for",
            );

            let (slice, bufs) = unsafe { self.unfilled_mut() };
            slice
                .iter_mut()
                .chain(bufs.iter_mut().flat_map(|buf| &mut **buf))
                .zip(other)
                .for_each(|(dest, &byte)| *dest = MaybeUninit::new(byte));

            unsafe {
                self.assume_init(other.len());
            }

            self.add_filled(other.len());
        }
    };
}

/// These methods are also present on [`ReadBufsRef`].
#[allow(unused_unsafe)]
impl<'b> ReadBufs<'_, 'b> {
    common_read_bufs_methods!(std::convert::identity, std::convert::identity);
}

impl Debug for ReadBufs<'_, '_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        use super::buf::PartialInitSlice;

        f.debug_list()
            .entries(
                self.data
                    .iter()
                    .enumerate()
                    .map(|(i, slice)| PartialInitSlice {
                        data: &slice,
                        filled: match i.cmp(&self.filled_bufs) {
                            cmp::Ordering::Less => slice.len(),
                            cmp::Ordering::Equal => self.filled_to,
                            cmp::Ordering::Greater => 0,
                        },
                        initialized: match i.cmp(&self.initialized_bufs) {
                            cmp::Ordering::Less => slice.len(),
                            cmp::Ordering::Equal => self.initialized_to,
                            cmp::Ordering::Greater => 0,
                        },
                    }),
            )
            .finish()
    }
}

/// A type that grants mutable access to a [`ReadBufs`].
///
/// You can create this by calling [`ReadBufs::as_ref`].
pub struct ReadBufsRef<'a, 'b> {
    bufs: CoMut<'a, ReadBufs<'a, 'b>>,
}

impl<'a, 'b> ReadBufsRef<'a, 'b> {
    /// Get a shared reference to the internal [`ReadBufs`].
    #[must_use]
    #[inline]
    pub fn bufs(&self) -> &ReadBufs<'a, 'b> {
        &self.bufs
    }

    /// Get a mutable reference to the internal [`ReadBufs`].
    ///
    /// # Safety
    ///
    /// The [`ReadBufs`] must not be replaced through this mutable reference.
    #[must_use]
    #[inline]
    pub unsafe fn bufs_mut(&mut self) -> &mut ReadBufs<'a, 'b> {
        unsafe { self.bufs.get_mut() }
    }

    /// Consume this `ReadBufsRef`, retrieving the underlying mutable reference to a [`ReadBufs`].
    ///
    /// # Safety
    ///
    /// The [`ReadBufs`] must not be replaced through this mutable reference.
    #[must_use]
    #[inline]
    pub unsafe fn into_mut(self) -> &'a mut ReadBufs<'a, 'b> {
        unsafe { self.bufs.into_inner() }
    }

    /// Reborrow the `ReadBufsRef` to a shorter lifetime.
    #[must_use]
    #[inline]
    pub fn as_ref(&mut self) -> ReadBufsRef<'_, 'b> {
        unsafe { self.bufs_mut() }.as_ref()
    }
}

/// These methods are also present on [`ReadBufs`].
impl<'b> ReadBufsRef<'_, 'b> {
    common_read_bufs_methods!(Self::bufs, Self::bufs_mut);
}

impl Debug for ReadBufsRef<'_, '_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.bufs().fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use std::io::IoSliceMut;
    use std::mem::MaybeUninit;
    use std::panic;

    use crate::read::{buf_assume_init_ref, io_slices};
    use crate::read::{to_init_slices, to_slices};

    use super::ReadBufs;

    #[test]
    fn new() {
        io_slices!(let data = [[1, 2, 3], [4, 5], [], [6]]);
        let bufs = ReadBufs::new(data);
        assert_eq!(bufs.data.len(), 4);
        assert_eq!(
            unsafe { to_init_slices(bufs.data) },
            [&[1_u8, 2, 3] as &[u8], &[4, 5], &[], &[6]]
        );
        assert_eq!(bufs.filled_bufs, 0);
        assert_eq!(bufs.filled_to, 0);
        assert_eq!(bufs.initialized_bufs, 4);
        assert_eq!(bufs.initialized_to, 0);
    }

    #[test]
    fn new_empty_start() {
        io_slices!(let data = [[], [], [], [1], []]);
        let bufs = ReadBufs::new(data);
        assert_eq!(bufs.filled_bufs, 3);
        assert_eq!(bufs.filled_to, 0);
        assert_eq!(bufs.initialized_bufs, 5);
        assert_eq!(bufs.initialized_to, 0);
    }

    #[test]
    fn new_all_empty() {
        io_slices!(let data = [[], [], [], [], []]);
        let bufs = ReadBufs::new(data);
        assert_eq!(bufs.filled_bufs, 5);
        assert_eq!(bufs.filled_to, 0);
        assert_eq!(bufs.initialized_bufs, 5);
        assert_eq!(bufs.initialized_to, 0);
    }

    #[test]
    fn uninit() {
        io_slices!(let data = uninit [2, 3, 0, 5]);
        data[3][2] = MaybeUninit::new(5);
        let bufs = ReadBufs::uninit(data);
        assert_eq!(bufs.data.len(), 4);
        assert_eq!(unsafe { bufs.data[3][2].assume_init() }, 5);
        assert_eq!(bufs.filled_bufs, 0);
        assert_eq!(bufs.filled_to, 0);
        assert_eq!(bufs.initialized_bufs, 0);
        assert_eq!(bufs.initialized_to, 0);
    }

    #[test]
    fn uninit_empty_start() {
        io_slices!(let data = uninit [0, 0, 0, 1, 0]);
        let bufs = ReadBufs::uninit(data);
        assert_eq!(bufs.filled_bufs, 3);
        assert_eq!(bufs.filled_to, 0);
        assert_eq!(bufs.initialized_bufs, 3);
        assert_eq!(bufs.initialized_to, 0);
    }

    #[test]
    fn uninit_all_empty() {
        io_slices!(let data = uninit [0, 0, 0, 0, 0]);
        let bufs = ReadBufs::uninit(data);
        assert_eq!(bufs.filled_bufs, 5);
        assert_eq!(bufs.filled_to, 0);
        assert_eq!(bufs.initialized_bufs, 5);
        assert_eq!(bufs.initialized_to, 0);
    }

    #[test]
    fn get_cursors() {
        #[track_caller]
        fn check_cursors<const N: usize, const M: usize>(
            data: &mut [IoSliceMut<'_>],
            cursor_buf: usize,
            expected_bufs: [&[u8]; N],
            cursor_to: usize,
            expected_buf: [u8; M],
        ) {
            let mut read_bufs = ReadBufs::new(data);

            let check_tuple = |(bufs, buf): (&[IoSliceMut<'_>], &[u8])| {
                assert_eq!(to_slices(bufs), expected_bufs);
                assert_eq!(buf, expected_buf);
            };
            let check_tuple_mut = |(bufs, buf): (&mut _, &mut _)| {
                check_tuple((bufs, &*buf));
            };

            read_bufs.initialized_bufs = cursor_buf;
            read_bufs.initialized_to = cursor_to;
            check_tuple(read_bufs.initialized());
            check_tuple_mut(read_bufs.initialized_mut());

            read_bufs.filled_bufs = cursor_buf;
            read_bufs.filled_to = cursor_to;
            check_tuple(read_bufs.filled());
            check_tuple_mut(read_bufs.filled_mut());
            check_tuple_mut(read_bufs.into_filled());
        }

        io_slices!(let data = [[1, 2, 3], [4, 5], [], [6, 7]]);

        check_cursors(data, 0, [], 0, []);
        check_cursors(data, 0, [], 2, [1, 2]);
        check_cursors(data, 3, [&[1, 2, 3], &[4, 5], &[]], 0, []);
        check_cursors(data, 3, [&[1, 2, 3], &[4, 5], &[]], 1, [6]);
        check_cursors(data, 4, [&[1, 2, 3], &[4, 5], &[], &[6, 7]], 0, []);
    }

    #[test]
    fn get_inner() {
        let data_template: &[&[u8]] = &[&[1, 2, 3], &[4, 5]];
        io_slices!(let data = init [[1, 2, 3], [4, 5]]);
        let mut read_bufs = ReadBufs {
            data,
            filled_bufs: 1,
            filled_to: 2,
            initialized_bufs: 2,
            initialized_to: 0,
        };
        assert_eq!(unsafe { to_init_slices(read_bufs.inner()) }, data_template);
        assert_eq!(
            unsafe { to_init_slices(read_bufs.inner_mut()) },
            data_template
        );
        assert_eq!(
            unsafe { to_init_slices(read_bufs.into_inner()) },
            data_template,
        );
    }

    #[test]
    fn unfilled() {
        #[track_caller]
        fn check_unfilled<const N: usize, const M: usize>(
            read_bufs: &mut ReadBufs<'_, '_>,
            filled_bufs: usize,
            filled_to: usize,
            expected_buf: [u8; N],
            expected_bufs: [&[u8]; M],
        ) {
            read_bufs.filled_bufs = filled_bufs;
            read_bufs.filled_to = filled_to;
            let (buf, bufs) = unsafe { read_bufs.unfilled_mut() };
            assert_eq!(unsafe { buf_assume_init_ref(buf) }, expected_buf);
            assert_eq!(unsafe { to_init_slices(bufs) }, expected_bufs);
        }

        io_slices!(let data = [[1, 2], [], [3]]);
        let mut read_bufs = ReadBufs::new(data);

        check_unfilled(&mut read_bufs, 0, 0, [1, 2], [&[], &[3]]);
        check_unfilled(&mut read_bufs, 0, 1, [2], [&[], &[3]]);
        check_unfilled(&mut read_bufs, 2, 0, [3], []);
        check_unfilled(&mut read_bufs, 3, 0, [], []);
    }

    #[test]
    fn initialize_unfilled() {
        io_slices!(let data = init [[1], [2, 3, 4, 5], [6, 7, 8], [], [9]]);
        let mut read_bufs = ReadBufs {
            data,
            filled_bufs: 1,
            filled_to: 2,
            initialized_bufs: 2,
            initialized_to: 1,
        };

        let (buf, bufs) = read_bufs.initialize_unfilled();
        assert_eq!(buf, [4, 5]);
        assert_eq!(to_slices(bufs), [&[6_u8, 0, 0] as &[u8], &[], &[0]]);
        assert_eq!(read_bufs.filled_bufs, 1);
        assert_eq!(read_bufs.filled_to, 2);
        assert_eq!(read_bufs.initialized_bufs, 5);
        assert_eq!(read_bufs.initialized_to, 0);

        read_bufs.data[2][1] = MaybeUninit::new(10);
        read_bufs.data[2][2] = MaybeUninit::new(11);
        read_bufs.data[4][0] = MaybeUninit::new(12);
        read_bufs.filled_to += 1;

        let (buf, bufs) = read_bufs.initialize_unfilled();
        assert_eq!(buf, [5]);
        assert_eq!(to_slices(bufs), [&[6_u8, 10, 11] as &[u8], &[], &[12]]);
        assert_eq!(read_bufs.filled_bufs, 1);
        assert_eq!(read_bufs.filled_to, 3);
        assert_eq!(read_bufs.initialized_bufs, 5);
        assert_eq!(read_bufs.initialized_to, 0);
    }

    #[test]
    fn remaining() {
        io_slices!(let data = [[1, 2, 3], [4, 5, 6, 7], [], [8, 9]]);

        let mut bufs = ReadBufs::new(data);
        assert_eq!(bufs.remaining(), 9);

        bufs.filled_to = 2;
        assert_eq!(bufs.remaining(), 7);

        bufs.filled_bufs = 1;
        bufs.filled_to = 3;
        assert_eq!(bufs.remaining(), 3);

        bufs.filled_bufs = 3;
        bufs.filled_to = 0;
        assert_eq!(bufs.remaining(), 2);

        bufs.filled_bufs = 4;
        bufs.filled_to = 0;
        assert_eq!(bufs.remaining(), 0);
    }

    #[test]
    fn set_filled() {
        io_slices!(let data = [[1, 2, 3], [4, 5, 6, 7], [], [8, 9, 255]]);
        let mut bufs = ReadBufs::new(data);
        bufs.initialized_bufs -= 1;
        bufs.initialized_to = 2;

        bufs.set_filled(1, 2);
        let (filled_bufs, filled_buf) = bufs.filled();
        assert_eq!(to_slices(filled_bufs), [[1, 2, 3]]);
        assert_eq!(filled_buf, [4, 5]);

        bufs.add_filled(4);
        assert_eq!(bufs.filled_bufs, 3);
        assert_eq!(bufs.filled_to, 2);
        let (filled_bufs, filled_buf) = bufs.filled();
        assert_eq!(
            to_slices(filled_bufs),
            [&[1_u8, 2, 3] as &[_], &[4, 5, 6, 7], &[]]
        );
        assert_eq!(filled_buf, [8, 9]);

        bufs.set_filled(1, 0);
        let (filled_bufs, filled_buf) = bufs.filled();
        assert_eq!(to_slices(filled_bufs), [[1_u8, 2, 3]]);
        assert_eq!(filled_buf, []);

        bufs.add_filled(0);
        let (filled_bufs, filled_buf) = bufs.filled();
        assert_eq!(to_slices(filled_bufs), [[1_u8, 2, 3]]);
        assert_eq!(filled_buf, []);

        bufs.clear();
        let (filled_bufs, filled_buf) = bufs.filled();
        assert!(filled_bufs.is_empty() && filled_buf.is_empty());

        panic::catch_unwind(panic::AssertUnwindSafe(|| bufs.add_filled(10))).unwrap_err();
        assert_eq!(bufs.filled_bufs, 0);
        assert_eq!(bufs.filled_to, 0);

        let mut fail_set_filled = |buf, to| {
            panic::catch_unwind(panic::AssertUnwindSafe(|| bufs.set_filled(buf, to))).unwrap_err();
            assert_eq!(bufs.filled_bufs, 0);
            assert_eq!(bufs.filled_to, 0);
        };

        fail_set_filled(0, 3);
        fail_set_filled(2, 0);
        fail_set_filled(2, 1);
        fail_set_filled(3, 3);
        fail_set_filled(4, 0);
        fail_set_filled(4, 1);
        fail_set_filled(5, 0);
    }

    #[test]
    fn assume_init() {
        io_slices!(let data = init [[1, 2, 3], [], [4, 5, 6, 7]]);
        let mut bufs = ReadBufs::uninit(data);
        bufs.filled_to += 1;
        bufs.initialized_to += 2;

        unsafe { bufs.assume_init(2) };
        assert_eq!(bufs.initialized_bufs, 2);
        assert_eq!(bufs.initialized_to, 0);

        unsafe { bufs.assume_init(0) };
        assert_eq!(bufs.initialized_bufs, 2);
        assert_eq!(bufs.initialized_to, 0);

        unsafe { bufs.assume_init(3) };
        assert_eq!(bufs.initialized_bufs, 2);
        assert_eq!(bufs.initialized_to, 1);

        unsafe { bufs.assume_init(6) };
        assert_eq!(bufs.initialized_bufs, 3);
        assert_eq!(bufs.initialized_to, 0);
    }

    #[test]
    fn append() {
        io_slices!(let data = uninit [0, 2, 3, 1]);
        let mut bufs = ReadBufs::uninit(data);

        bufs.append(&[1, 2, 3, 4]);
        let (filled_bufs, filled_buf) = bufs.filled();
        assert_eq!(to_slices(filled_bufs), &[&[] as &[u8], &[1, 2]]);
        assert_eq!(filled_buf, &[3, 4]);
        assert_eq!(bufs.initialized_bufs, bufs.filled_bufs);
        assert_eq!(bufs.initialized_to, bufs.filled_to);

        bufs.append(&[]);
        assert_eq!(bufs.filled_bufs, 2);
        assert_eq!(bufs.filled_to, 2);
        assert_eq!(bufs.initialized_bufs, 2);
        assert_eq!(bufs.initialized_to, 2);

        panic::catch_unwind(panic::AssertUnwindSafe(|| bufs.append(&[9, 9, 9, 9]))).unwrap_err();
        assert_eq!(bufs.filled_bufs, 2);
        assert_eq!(bufs.filled_to, 2);
        assert_eq!(bufs.initialized_bufs, 2);
        assert_eq!(bufs.initialized_to, 2);

        bufs.append(&[5, 6]);
        let (filled_bufs, filled_buf) = bufs.filled();
        assert_eq!(
            to_slices(filled_bufs),
            &[&[] as &[u8], &[1, 2], &[3, 4, 5], &[6]]
        );
        assert_eq!(filled_buf, &[]);
        assert_eq!(bufs.initialized_bufs, bufs.filled_bufs);
        assert_eq!(bufs.initialized_to, bufs.filled_to);

        bufs.append(&[]);
        assert_eq!(bufs.filled_bufs, 4);
        assert_eq!(bufs.filled_to, 0);
        assert_eq!(bufs.initialized_bufs, 4);
        assert_eq!(bufs.initialized_to, 0);
    }

    #[test]
    fn debug() {
        io_slices!(let data = init [[], [1, 2, 3], [4], [5, 6, 7], [8, 9]]);
        let bufs = ReadBufs {
            data,
            filled_bufs: 1,
            filled_to: 2,
            initialized_bufs: 3,
            initialized_to: 1,
        };
        assert_eq!(
            format!("{:?}", bufs),
            "[[], [1, 2, (3)], [(4)], [(5), -, -], [-, -]]"
        );
    }
}
