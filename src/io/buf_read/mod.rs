#[cfg(doc)]
use std::io::ErrorKind;

use completion_io::AsyncBufRead;

use super::extend_lifetime_mut;
#[cfg(test)]
use super::test_utils;

mod take_until;
pub use take_until::*;

mod read_until;
pub use read_until::ReadUntil;

mod read_line;
pub use read_line::ReadLine;

mod split;
pub use split::Split;

mod lines;
pub use lines::Lines;

/// Extension trait for [`AsyncBufRead`].
pub trait AsyncBufReadExt: AsyncBufRead {
    /// Create a reader that reads until the delimiter byte or EOF is reached.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::io::{AsyncReadExt, AsyncBufReadExt};
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let mut reader = std::io::Cursor::new([1, 2, 3, 4, 5, 6, 7, 8]).take_until(5);
    ///
    /// let mut v = Vec::new();
    /// reader.read_to_end(&mut v).await?;
    ///
    /// assert_eq!(v, vec![1, 2, 3, 4, 5]);
    /// # completion_io::Result::Ok(())
    /// # }).unwrap();
    /// ```
    fn take_until(self, delim: u8) -> TakeUntil<Self>
    where
        Self: Sized,
    {
        TakeUntil::new(self, delim)
    }

    /// Read all bytes into `buf` until the delimiter byte or EOF is reached.
    ///
    /// This is semantically equivalent to a call to [`take_until`](Self::take_until) followed by a
    /// call to [`read_to_end`](super::AsyncReadExt::read_to_end). See those functions for more
    /// details.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::io::{AsyncReadExt, AsyncBufReadExt};
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let mut reader = std::io::Cursor::new([1, 2, 3, 4, 5, 6, 7, 8]);
    ///
    /// let mut v = Vec::new();
    /// reader.read_until(5, &mut v).await?;
    ///
    /// assert_eq!(v, vec![1, 2, 3, 4, 5]);
    /// # completion_io::Result::Ok(())
    /// # }).unwrap();
    /// ```
    fn read_until<'a>(&'a mut self, delim: u8, buf: &'a mut Vec<u8>) -> ReadUntil<'a, Self> {
        ReadUntil::new(self, delim, buf)
    }

    /// Read all bytes until a newline (the `0xA` byte) is reached, and append them to the provided
    /// buffer.
    ///
    /// This is semantically equivalent to a call to [`take_until`](Self::take_until) with `b'\n'`
    /// followed by a call to [`read_to_string`](super::AsyncReadExt::read_to_string). See those
    /// functions for more details and error semantics.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::io::AsyncBufReadExt;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let mut reader = std::io::Cursor::new("foo\nbar");
    ///
    /// let mut s = String::new();
    /// let num_bytes = reader.read_line(&mut s).await?;
    /// assert_eq!(num_bytes, 4);
    /// assert_eq!(s, "foo\n");
    /// s.clear();
    ///
    /// let num_bytes = reader.read_line(&mut s).await?;
    /// assert_eq!(num_bytes, 3);
    /// assert_eq!(s, "bar");
    /// s.clear();
    ///
    /// let num_bytes = reader.read_line(&mut s).await?;
    /// assert_eq!(num_bytes, 0);
    /// assert_eq!(s, "");
    /// # completion_io::Result::Ok(())
    /// # }).unwrap();
    /// ```
    fn read_line<'a>(&'a mut self, buf: &'a mut String) -> ReadLine<'a, Self> {
        ReadLine::new(self, buf)
    }

    /// Create a stream that yields the contents of this reader split on the byte `delim`.
    ///
    /// The stream returned from this function will return instances of
    /// [`io::Result`](std::io::Result)`<`[`Vec`]`<`[`u8`]`>>`. Each vector returned will _not_
    /// have the delimiter byte at the end.
    ///
    /// # Errors
    ///
    /// Each segment yielded by the stream has the same error semantics as
    /// [`read_until`](Self::read_until).
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::io::AsyncBufReadExt;
    /// use completion::stream::CompletionStreamExt;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let mut reader = std::io::Cursor::new(b"lorem-ipsum-dolor");
    ///
    /// let mut splitter = reader.split(b'-').map(|l| l.unwrap());
    /// # use futures_lite::pin;
    /// pin!(splitter);
    /// assert_eq!(splitter.next().await, Some(b"lorem".to_vec()));
    /// assert_eq!(splitter.next().await, Some(b"ipsum".to_vec()));
    /// assert_eq!(splitter.next().await, Some(b"dolor".to_vec()));
    /// assert_eq!(splitter.next().await, None);
    /// # });
    /// ```
    fn split<'r>(self, byte: u8) -> Split<'r, Self>
    where
        Self: Sized + 'r,
    {
        Split::new(self, byte)
    }

    /// Create a stream over the lines of this reader.
    ///
    /// The stream returned from this function will yield instances of
    /// [`io::Result`](std::io::Result)`<`[`String`]`>`. Each string returned will _not_ have a
    /// newline byte (the `0xA`/`\n` byte) or `CRLF` (`0xD`, `0xA`/`\r\n` bytes) at the end.
    ///
    /// # Errors
    ///
    /// Each line yielded by the stream has the same error semantics as
    /// [`read_line`](Self::read_line).
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::io::AsyncBufReadExt;
    /// use completion::stream::CompletionStreamExt;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let mut reader = std::io::Cursor::new(b"lorem\nipsum\r\ndolor");
    ///
    /// let mut lines = reader.lines().map(|l| l.unwrap());
    /// # use futures_lite::pin;
    /// pin!(lines);
    /// assert_eq!(lines.next().await, Some("lorem".to_owned()));
    /// assert_eq!(lines.next().await, Some("ipsum".to_owned()));
    /// assert_eq!(lines.next().await, Some("dolor".to_owned()));
    /// assert_eq!(lines.next().await, None);
    /// # });
    /// ```
    fn lines<'r>(self) -> Lines<'r, Self>
    where
        Self: Sized + 'r,
    {
        Lines::new(self)
    }
}
impl<T: AsyncBufRead + ?Sized> AsyncBufReadExt for T {}
