use std::mem::MaybeUninit;
use std::ptr;

#[repr(transparent)]
pub(crate) struct MaybeUninitIoSliceMut(*mut [MaybeUninit<u8>]);

impl MaybeUninitIoSliceMut {
    pub(crate) fn new(ptr: *mut MaybeUninit<u8>, len: usize) -> Self {
        Self(ptr::slice_from_raw_parts_mut(ptr, len))
    }
    pub(crate) fn get(&self) -> *const [MaybeUninit<u8>] {
        self.0
    }
    pub(crate) fn get_mut(&mut self) -> *mut [MaybeUninit<u8>] {
        self.0
    }
}
