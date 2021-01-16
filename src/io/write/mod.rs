#[cfg(doc)]
use std::io::ErrorKind;

use completion_io::AsyncWrite;

use super::extend_lifetime_mut;
#[cfg(test)]
use super::test_utils;

mod write_all;
pub use write_all::WriteAll;

/// Extension trait for [`AsyncWrite`].
pub trait AsyncWriteExt: AsyncWrite {
    /// Attempt to write an entire buffer into this writer.
    ///
    /// This method will continuously call [`write`] until there is no more data to be written or an
    /// error of non-[`ErrorKind::Interrupted`] is returned. This method will not return until the
    /// entire buffer has been successfully written or such an error occurs. The first error that is
    /// not of [`ErrorKind::Interrupted`] kind generated from this method will be returned.
    ///
    /// If the buffer contains no data, this will never call [`write`].
    ///
    /// # Errors
    ///
    /// This function will return the first error of non-[`ErrorKind::Interrupted`] kind that
    /// [`write`] returns. If the writer is given a non-empty buffer but it returns without writing
    /// any data, it will error with [`ErrorKind::WriteZero`].
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::io::AsyncWriteExt;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let mut writer = Vec::new();
    /// writer.write_all(b"Hello World!").await?;
    /// assert_eq!(writer, b"Hello World!");
    /// # completion_io::Result::Ok(())
    /// # }).unwrap();
    /// ```
    ///
    /// [`write`]: completion_io::AsyncWriteWith::write
    fn write_all<'a>(&'a mut self, buf: &'a [u8]) -> WriteAll<'a, Self> {
        WriteAll::new(self, buf)
    }
}
impl<T: AsyncWrite + ?Sized> AsyncWriteExt for T {}
