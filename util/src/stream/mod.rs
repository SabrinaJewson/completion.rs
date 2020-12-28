//! Utilities for the [`CompletionStream`] trait.

use core::iter::FusedIterator;
use core::pin::Pin;
use core::task::{Context, Poll};

#[doc(no_inline)]
pub use completion_core::CompletionStream;
use futures_core::Stream;

use super::MustComplete;

mod adapters;
pub use adapters::*;

mod futures;
pub use futures::*;

mod unfold;
pub use unfold::*;

/// Extension trait for [`CompletionStream`].
pub trait CompletionStreamExt: CompletionStream {
    /// A convenience for calling [`CompletionStream::poll_next`] on [`Unpin`] futures.
    ///
    /// # Safety
    ///
    /// Identical to [`CompletionStream::poll_next`].
    unsafe fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<Self::Item>>
    where
        Self: Unpin,
    {
        Pin::new(self).poll_next(cx)
    }

    /// Get the next item in the stream.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion_util::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion_util::future::block_on(completion_util::completion_async! {
    /// let mut stream = stream::iter(0..3).must_complete();
    /// assert_eq!(stream.next().await, Some(0));
    /// assert_eq!(stream.next().await, Some(1));
    /// assert_eq!(stream.next().await, Some(2));
    /// assert_eq!(stream.next().await, None);
    /// # });
    /// ```
    fn next(&mut self) -> Next<'_, Self> {
        Next { stream: self }
    }

    /// Count the number of items in the stream.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion_util::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion_util::future::block_on(completion_util::completion_async! {
    /// let stream_1 = stream::iter(3..7).must_complete();
    /// let stream_2 = stream::iter(&[8, 2, 4]).must_complete();
    ///
    /// assert_eq!(stream_1.count().await, 4);
    /// assert_eq!(stream_2.count().await, 3);
    /// # });
    /// ```
    fn count(self) -> Count<Self>
    where
        Self: Sized,
    {
        Count {
            count: 0,
            stream: self,
        }
    }

    /// Get the last element in the stream.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion_util::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion_util::future::block_on(completion_util::completion_async! {
    /// assert_eq!(stream::iter(3..7).must_complete().last().await, Some(6));
    /// assert_eq!(stream::empty::<String>().must_complete().last().await, None);
    /// # });
    /// ```
    fn last(self) -> Last<Self>
    where
        Self: Sized,
    {
        Last {
            stream: self,
            last: None,
        }
    }

    /// Get the nth element in the stream.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion_util::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion_util::future::block_on(completion_util::completion_async! {
    /// assert_eq!(stream::iter(3..7).must_complete().nth(2).await, Some(5));
    /// assert_eq!(stream::iter(3..7).must_complete().nth(10).await, None);
    /// # });
    /// ```
    fn nth(&mut self, n: usize) -> Nth<'_, Self> {
        Nth { stream: self, n }
    }

    // TODO: step_by

    /// Chain this stream with another.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion_util::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion_util::future::block_on(completion_util::completion_async! {
    /// let mut stream = stream::iter(4..6).must_complete()
    ///     .chain(stream::iter(6..10).must_complete());
    ///
    /// assert_eq!(stream.next().await, Some(4));
    /// assert_eq!(stream.next().await, Some(5));
    /// assert_eq!(stream.next().await, Some(6));
    /// assert_eq!(stream.next().await, Some(7));
    /// assert_eq!(stream.next().await, Some(8));
    /// assert_eq!(stream.next().await, Some(9));
    /// assert_eq!(stream.next().await, None);
    /// # });
    /// ```
    fn chain<U: CompletionStream<Item = Self::Item>>(self, other: U) -> Chain<Self, U>
    where
        Self: Sized,
    {
        Chain {
            a: Some(self),
            b: other,
        }
    }

    // TODO: zip

    /// Map this stream's items with a closure.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion_util::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion_util::future::block_on(completion_util::completion_async! {
    /// let mut stream = stream::iter(0..5).must_complete().map(|x| x * 2 + 4);
    ///
    /// assert_eq!(stream.next().await, Some(4));
    /// assert_eq!(stream.next().await, Some(6));
    /// assert_eq!(stream.next().await, Some(8));
    /// assert_eq!(stream.next().await, Some(10));
    /// assert_eq!(stream.next().await, Some(12));
    /// assert_eq!(stream.next().await, None);
    /// # });
    /// ```
    fn map<T, F: FnMut(Self::Item) -> T>(self, f: F) -> Map<Self, F>
    where
        Self: Sized,
    {
        Map { stream: self, f }
    }

    // TODO: then

    /// Call a closure on each item the stream.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion_util::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion_util::future::block_on(completion_util::completion_async! {
    /// stream::iter(0..8).must_complete().for_each(|num| println!("{}", num)).await;
    /// # });
    /// ```
    fn for_each<F: FnMut(Self::Item)>(self, f: F) -> ForEach<Self, F>
    where
        Self: Sized,
    {
        ForEach { stream: self, f }
    }

    // TODO: filter
    // TODO: filter_map
    // TODO: enumerate
    // TODO: peekable
    // TODO: skip_while
    // TODO: take_while
    // TODO: skip
    // TODO: take
    // TODO: scan
    // TODO: flat_map
    // TODO: flatten

    /// Fuse the stream so that it is guaranteed to continue to yield `None` when exhausted.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion_util::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion_util::future::block_on(completion_util::completion_async! {
    /// let mut stream = stream::once(5).must_complete().fuse();
    /// assert_eq!(stream.next().await, Some(5));
    /// assert_eq!(stream.next().await, None);
    /// assert_eq!(stream.next().await, None);
    /// assert_eq!(stream.next().await, None);
    /// # });
    /// ```
    fn fuse(self) -> Fuse<Self>
    where
        Self: Sized,
    {
        Fuse { stream: Some(self) }
    }

    // TODO: inspect
    // TODO: by_ref
    // TODO: collect
    // TODO: try_collect
    // TODO: partition
    // TODO: try_fold
    // TODO: try_for_each
    // TODO: fold
    // TODO: all
    // TODO: any
    // TODO: find
    // TODO: find_map
    // TODO: position
    // TODO: max
    // TODO: min
    // TODO: max_by_key
    // TODO: max_by
    // TODO: min_by_key
    // TODO: min_by
    // TODO: unzip
    // TODO: copied
    // TODO: cloned
    // TODO: cycle
    // TODO: cmp
    // TODO: partial_cmp
    // TODO: eq
    // TODO: ne
    // TODO: lt
    // TODO: le
    // TODO: gt
    // TODO: ge

    /// Box the stream, erasing its type.
    ///
    /// Requires the `alloc` feature.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion_util::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # let some_condition = true;
    /// // These streams are different types, but boxing them makes them the same type.
    /// let stream = if some_condition {
    ///     stream::iter(2..18).must_complete().boxed()
    /// } else {
    ///     stream::iter(vec![5, 3, 7, 8, 2]).must_complete().boxed()
    /// };
    /// ```
    #[cfg(feature = "alloc")]
    fn boxed<'a>(self) -> BoxCompletionStream<'a, Self::Item>
    where
        Self: Sized + Send + 'a,
    {
        Box::pin(self)
    }

    /// Box the stream locally, erasing its type.
    ///
    /// Requires the `alloc` feature.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion_util::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # let some_condition = true;
    /// // These streams are different types, but boxing them makes them the same type.
    /// let stream = if some_condition {
    ///     stream::iter(2..18).must_complete().boxed_local()
    /// } else {
    ///     stream::iter(vec![5, 3, 7, 8, 2]).must_complete().boxed_local()
    /// };
    /// ```
    #[cfg(feature = "alloc")]
    fn boxed_local<'a>(self) -> LocalBoxCompletionStream<'a, Self::Item>
    where
        Self: Sized + 'a,
    {
        Box::pin(self)
    }
}
impl<T: CompletionStream + ?Sized> CompletionStreamExt for T {}

/// A type-erased completion future.
///
/// Requires the `alloc` feature.
#[cfg(feature = "alloc")]
pub type BoxCompletionStream<'a, T> = Pin<Box<dyn CompletionStream<Item = T> + Send + 'a>>;

/// A type-erased completion future that cannot be send across threads.
///
/// Requires the `alloc` feature.
#[cfg(feature = "alloc")]
pub type LocalBoxCompletionStream<'a, T> = Pin<Box<dyn CompletionStream<Item = T> + 'a>>;

/// Extension trait for converting [`Stream`]s to [`CompletionStream`]s.
pub trait StreamExt: Stream + Sized {
    /// Make sure that the future will complete. Equivalent to [`MustComplete::new`].
    ///
    /// # Examples
    ///
    /// ```
    /// use completion_util::StreamExt;
    /// use futures_lite::stream;
    ///
    /// let completion_stream = stream::iter(&[1, 1, 2, 3, 5]).must_complete();
    /// ```
    fn must_complete(self) -> MustComplete<Self> {
        MustComplete::new(self)
    }
}
impl<T: Stream> StreamExt for T {}

/// Convert a stream to a blocking iterator.
///
/// # Examples
///
// TODO: use stream macro here
/// ```
/// use completion_util::{stream, StreamExt};
///
/// let stream = futures_lite::stream::once('!').must_complete();
/// futures_lite::pin!(stream);
/// assert_eq!(stream::block_on(stream).collect::<Vec<_>>(), vec!['!']);
/// ```
pub fn block_on<S: CompletionStream + Unpin>(stream: S) -> BlockOn<S> {
    BlockOn { stream }
}

/// Iterator for [`block_on`].
#[derive(Debug)]
pub struct BlockOn<S> {
    stream: S,
}

impl<S: CompletionStream + Unpin> Iterator for BlockOn<S> {
    type Item = S::Item;

    fn next(&mut self) -> Option<Self::Item> {
        crate::future::block_on(self.stream.next())
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.stream.size_hint()
    }
}
impl<S: CompletionStream + Unpin> FusedIterator for BlockOn<Fuse<S>> {}
