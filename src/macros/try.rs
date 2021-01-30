/// A stable version of the try trait.
#[doc(hidden)]
pub trait __Try {
    /// The type of this value on success.
    type Ok;
    /// The type of this value on failure.
    type Error;

    /// Applies the `?` operator.
    ///
    /// # Errors
    ///
    /// Fails if the try value is an error.
    fn into_result(self) -> Result<Self::Ok, Self::Error>;

    /// Wrap an error value.
    fn from_error(v: Self::Error) -> Self;

    /// Wrap an OK value.
    fn from_ok(v: Self::Ok) -> Self;
}

impl<T> __Try for Option<T> {
    type Ok = T;
    type Error = NoneError;

    fn into_result(self) -> Result<Self::Ok, Self::Error> {
        self.ok_or(NoneError)
    }
    fn from_error(_: Self::Error) -> Self {
        None
    }
    fn from_ok(v: Self::Ok) -> Self {
        Some(v)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NoneError;

impl<T, E> __Try for Result<T, E> {
    type Ok = T;
    type Error = E;

    fn into_result(self) -> Result<<Self as __Try>::Ok, Self::Error> {
        self
    }
    fn from_error(v: Self::Error) -> Self {
        Err(v)
    }
    fn from_ok(v: <Self as __Try>::Ok) -> Self {
        Ok(v)
    }
}
