use std::convert::TryFrom;
use std::mem::MaybeUninit;
use std::ptr;

use winapi::shared::ntdef::{CHAR, ULONG};
use winapi::shared::ws2def::WSABUF;

#[repr(transparent)]
pub(crate) struct MaybeUninitIoSliceMut(WSABUF);

impl MaybeUninitIoSliceMut {
    pub(crate) fn new(ptr: *mut MaybeUninit<u8>, len: usize) -> Self {
        let len = ULONG::try_from(len)
            .expect("attempted to create `MaybeUninitIoSliceMut` with size > 4GB");
        Self(WSABUF {
            len,
            buf: ptr.cast(),
        })
    }
    pub(crate) fn get(&self) -> *const [MaybeUninit<u8>] {
        ptr::slice_from_raw_parts(self.0.buf.cast(), self.0.len as usize)
    }
    pub(crate) fn get_mut(&mut self) -> *mut [MaybeUninit<u8>] {
        ptr::slice_from_raw_parts_mut(self.0.buf.cast(), self.0.len as usize)
    }
}
