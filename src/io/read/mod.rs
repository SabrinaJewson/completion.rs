#[cfg(doc)]
use std::io::ErrorKind;

use completion_io::{AsyncRead, ReadBufRef};

use super::extend_lifetime_mut;
#[cfg(test)]
use super::test_utils;

mod read_to_end;
pub use read_to_end::ReadToEnd;

mod read_to_string;
pub use read_to_string::ReadToString;

mod read_exact;
pub use read_exact::ReadExact;

mod chain;
pub use chain::*;

mod take;
pub use take::*;

/// Extension trait for [`AsyncRead`].
pub trait AsyncReadExt: AsyncRead {
    /// Read all bytes until EOF in this source, placing them into `buf`.
    ///
    /// If successful, this function will return the total number of bytes read.
    ///
    /// # Errors
    ///
    /// If this function encounters an error of the kind [`ErrorKind::Interrupted`] then the error
    /// is ignored and the operation will continue.
    ///
    /// If any other read error is encountered then this function immediately returns. Any bytes
    /// which have already been read will be appended to `buf`.
    ///
    /// # Cancellation
    ///
    /// If the returned future is cancelled, all previously read bytes will be appended to `buf`
    /// and no data will be lost.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::io::AsyncReadExt;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let mut reader = std::io::Cursor::new([1, 2, 3, 4, 5]);
    ///
    /// let mut v = Vec::new();
    /// reader.read_to_end(&mut v).await?;
    ///
    /// assert_eq!(v, vec![1, 2, 3, 4, 5]);
    /// # completion_io::Result::Ok(())
    /// # }).unwrap();
    /// ```
    #[inline]
    fn read_to_end<'a>(&'a mut self, buf: &'a mut Vec<u8>) -> ReadToEnd<'a, Self> {
        ReadToEnd::new(self, buf)
    }

    /// Read all bytes until EOF in this source, appending them to `buf`.
    ///
    /// If successful, this function returns the number of bytes which were read and appended to
    /// `buf`.
    ///
    /// # Errors
    ///
    /// If the data in the reader is not valid UTF-8 then an error is returned and `buf` is
    /// unchanged.
    ///
    /// If this function encounters an error of the kind [`ErrorKind::Interrupted`] then the error
    /// is ignored and the operation will continue.
    ///
    /// If any other read error is encountered then this function immediately returns, and the
    /// string will be left unchanged. Crucially, this is _different_ from the behaviour of
    /// [`read_to_end`](Self::read_to_end), which will leave any bytes that have already been read
    /// in the buffer.
    ///
    /// # Cancellation
    ///
    /// If the returned future is cancelled, the string will be left unchanged. This is different
    /// from the behaviour of [`read_to_end`](Self::read_to_end), which will not lose any data.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::io::AsyncReadExt;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let mut reader = std::io::Cursor::new("Hello World");
    ///
    /// let mut s = String::new();
    /// reader.read_to_string(&mut s).await?;
    ///
    /// assert_eq!(s, "Hello World");
    /// # completion_io::Result::Ok(())
    /// # }).unwrap();
    /// ```
    #[inline]
    fn read_to_string<'a>(&'a mut self, buf: &'a mut String) -> ReadToString<'a, Self> {
        ReadToString::new(self, buf)
    }

    /// Read the exact number of bytes required to fill the buffer.
    ///
    /// # Errors
    ///
    /// If this function encounters an error of the kind [`ErrorKind::Interrupted`] then the error
    /// is ignored and the operation will continue.
    ///
    /// If this function encounters an "end of file" before completely filling the buffer, it
    /// returns an error of the kind [`ErrorKind::UnexpectedEof`]. The buffer will contain as many
    /// bytes as were written before the error occurred in that case.
    ///
    /// If any other read error is encountered then this function immediately returns. The contents
    /// of `buf` are unspecified in this case.
    ///
    /// If this function returns an error, it is unspecified how many bytes it has read, but it will
    /// never read more than would be necessary to completely fill the buffer.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::mem::MaybeUninit;
    /// use completion::io::{ReadBuf, AsyncReadExt};
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let mut reader = std::io::Cursor::new("Hello World");
    ///
    /// let mut storage = [MaybeUninit::uninit(); 11];
    /// let mut buf = ReadBuf::uninit(&mut storage);
    /// reader.read_exact(buf.as_ref()).await?;
    ///
    /// assert_eq!(buf.into_filled(), b"Hello World");
    /// # completion_io::Result::Ok(())
    /// # }).unwrap();
    /// ```
    fn read_exact<'a>(&'a mut self, buf: ReadBufRef<'a>) -> ReadExact<'a, Self> {
        ReadExact::new(self, buf)
    }

    /// Chain this reader with another.
    ///
    /// The returned [`AsyncRead`] instance will first read all bytes from this object until EOF is
    /// encountered. Afterwards the output is equivalent to the output of `next`.
    ///
    /// # The static bound
    ///
    /// The second reader is currently required to live for `'static`. This is due to limitations
    /// with Rust, and we can hopefully remove it in the future.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::io::AsyncReadExt;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let first = std::io::Cursor::new("Hello ");
    /// let second = std::io::Cursor::new("world!");
    /// let mut reader = first.chain(second);
    ///
    /// let mut s = String::new();
    /// reader.read_to_string(&mut s).await?;
    /// assert_eq!(s, "Hello world!");
    /// # completion_io::Result::Ok(())
    /// # }).unwrap();
    /// ```
    fn chain<R: AsyncRead + 'static>(self, next: R) -> Chain<Self, R>
    where
        Self: Sized,
    {
        Chain::new(self, next)
    }

    /// Read at most `limit` bytes from the reader.
    ///
    /// This function returns a new instance of [`AsyncRead`] which will read at most `limit`
    /// bytes after which it will reach EOF. Any read errors will not count towards the number of
    /// bytes read and future calls to [`read`](super::AsyncReadWith::read) may succeed.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::io::AsyncReadExt;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let mut reader = std::io::Cursor::new("foo-bar").take(5);
    ///
    /// let mut s = String::new();
    /// reader.read_to_string(&mut s).await?;
    /// assert_eq!(s, "foo-b");
    /// # completion_io::Result::Ok(())
    /// # }).unwrap();
    /// ```
    fn take(self, limit: u64) -> Take<Self>
    where
        Self: Sized,
    {
        Take::new(self, limit)
    }
}
impl<T: AsyncRead + ?Sized> AsyncReadExt for T {}
