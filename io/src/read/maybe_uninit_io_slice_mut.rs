use std::fmt::{self, Debug, Formatter};
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::ops::{Deref, DerefMut};

use crate::sys;

/// A partially uninitialized [`IoSliceMut`], used inside [`ReadBufs`].
///
/// It is semantically a wrapper around an `&mut [MaybeUninit<u8>]`, but is guaranteed to be ABI
/// compatible with the `iovec` type on Unix platforms, the `WSABUF` type on Windows and the
/// `Iovec` type on Wasi.
///
/// [`IoSliceMut`]: std::io::IoSliceMut
/// [`ReadBufs`]: super::ReadBufs
#[repr(transparent)]
pub struct MaybeUninitIoSliceMut<'a> {
    slice: sys::MaybeUninitIoSliceMut,
    _phantom: PhantomData<&'a mut [MaybeUninit<u8>]>,
}

unsafe impl Send for MaybeUninitIoSliceMut<'_> {}
unsafe impl Sync for MaybeUninitIoSliceMut<'_> {}

impl Debug for MaybeUninitIoSliceMut<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_tuple("MaybeUninitIoSliceMut")
            .field(&self.len())
            .finish()
    }
}

impl<'a> MaybeUninitIoSliceMut<'a> {
    /// Create a new `MaybeUninitIoSliceMut` wrapping a byte slice.
    ///
    /// # Panics
    ///
    /// Panics on Windows if the slice is larger than 4GB.
    #[inline]
    pub fn new(buf: &'a mut [MaybeUninit<u8>]) -> Self {
        Self {
            slice: sys::MaybeUninitIoSliceMut::new(buf.as_mut_ptr(), buf.len()),
            _phantom: PhantomData,
        }
    }
}

impl Deref for MaybeUninitIoSliceMut<'_> {
    type Target = [MaybeUninit<u8>];
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.slice.get() }
    }
}
impl DerefMut for MaybeUninitIoSliceMut<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.slice.get_mut() }
    }
}
