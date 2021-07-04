use std::mem::MaybeUninit;
use std::ptr;

#[repr(transparent)]
pub(crate) struct MaybeUninitIoSliceMut(wasi::Iovec);

impl MaybeUninitIoSliceMut {
    pub(crate) fn new(ptr: *mut MaybeUninit<u8>, len: usize) -> Self {
        Self(wasi::Iovec {
            buf: ptr.cast(),
            buf_len: len,
        })
    }
    pub(crate) fn get(&self) -> *const [MaybeUninit<u8>] {
        ptr::slice_from_raw_parts(self.0.buf.cast(), self.0.buf_len)
    }
    pub(crate) fn get_mut(&mut self) -> *mut [MaybeUninit<u8>] {
        ptr::slice_from_raw_parts_mut(self.0.buf.cast(), self.0.buf_len)
    }
}
