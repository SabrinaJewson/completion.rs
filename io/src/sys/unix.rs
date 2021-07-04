use std::mem::MaybeUninit;
use std::ptr;

#[repr(transparent)]
pub(crate) struct MaybeUninitIoSliceMut(libc::iovec);

impl MaybeUninitIoSliceMut {
    pub(crate) fn new(ptr: *mut MaybeUninit<u8>, len: usize) -> Self {
        Self(libc::iovec {
            iov_base: ptr.cast(),
            iov_len: len,
        })
    }
    pub(crate) fn get(&self) -> *const [MaybeUninit<u8>] {
        ptr::slice_from_raw_parts(self.0.iov_base.cast(), self.0.iov_len)
    }
    pub(crate) fn get_mut(&mut self) -> *mut [MaybeUninit<u8>] {
        ptr::slice_from_raw_parts_mut(self.0.iov_base.cast(), self.0.iov_len)
    }
}
