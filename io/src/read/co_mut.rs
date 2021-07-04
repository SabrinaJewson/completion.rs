use std::marker::PhantomData;
use std::ops::Deref;
use std::ptr::NonNull;

/// A covariant mutable reference.
pub struct CoMut<'a, T> {
    ptr: NonNull<T>,
    _ref: PhantomData<&'a T>,
}

impl<'a, T> CoMut<'a, T> {
    /// Get a mutable reference to the inner `T`.
    ///
    /// # Safety
    ///
    /// Any of `T`'s covariant generic parameters may have a shorter lifetime than their real one,
    /// so `T` must not be modified in a way that would cause use-after-frees.
    pub unsafe fn get_mut(&mut self) -> &mut T {
        unsafe { self.ptr.as_mut() }
    }

    /// Convert this type into the inner `&mut T`.
    ///
    /// # Safety
    ///
    /// Any of `T`'s covariant generic parameters may have a shorter lifetime than their real one,
    /// so `T` must not be modified in a way that would cause use-after-frees.
    pub unsafe fn into_inner(mut self) -> &'a mut T {
        unsafe { self.ptr.as_mut() }
    }
}

impl<'a, T> From<&'a mut T> for CoMut<'a, T> {
    fn from(reference: &'a mut T) -> Self {
        Self {
            ptr: NonNull::from(reference),
            _ref: PhantomData,
        }
    }
}

impl<T> Deref for CoMut<'_, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        unsafe { self.ptr.as_ref() }
    }
}

unsafe impl<T> Send for CoMut<'_, T> {}
unsafe impl<T> Sync for CoMut<'_, T> {}
