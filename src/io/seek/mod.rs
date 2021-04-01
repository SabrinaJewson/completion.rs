use completion_io::AsyncSeek;

mod stream_position;
pub use stream_position::StreamPosition;

/// Extension trait for [`AsyncSeek`].
pub trait AsyncSeekExt: AsyncSeek {
    /// Retrieve the current seek position from the start of the stream.
    ///
    /// This is equivalent to `self.seek(SeekFrom::Current(0))`.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::io::{Cursor, AsyncReadExt, AsyncSeekExt};
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let mut reader = Cursor::new([1, 2, 3]);
    /// assert_eq!(reader.stream_position().await?, 0);
    ///
    /// reader.read_to_end(&mut Vec::new()).await?;
    /// assert_eq!(reader.stream_position().await?, 3);
    /// # completion_io::Result::Ok(())
    /// # }).unwrap();
    /// ```
    fn stream_position(&mut self) -> StreamPosition<'_, Self> {
        StreamPosition::new(self)
    }
}
impl<T: AsyncSeek + ?Sized> AsyncSeekExt for T {}
