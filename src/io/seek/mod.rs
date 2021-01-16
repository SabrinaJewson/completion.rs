use completion_io::AsyncSeek;

/// Extension trait for [`AsyncSeek`].
///
/// This currently contains no methods, but is present for completeness.
pub trait AsyncSeekExt: AsyncSeek {}
impl<T: AsyncSeek + ?Sized> AsyncSeekExt for T {}
