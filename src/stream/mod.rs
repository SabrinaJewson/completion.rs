//! Utilities for the [`CompletionStream`] trait.

#[cfg(feature = "alloc")]
use alloc::boxed::Box;
use core::cmp;
#[cfg(feature = "std")]
use core::iter::FusedIterator;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::CompletionFuture;
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

mod from_completion_stream;
pub use from_completion_stream::FromCompletionStream;

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
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let mut stream = stream::iter(0..3).must_complete();
    /// assert_eq!(stream.next().await, Some(0));
    /// assert_eq!(stream.next().await, Some(1));
    /// assert_eq!(stream.next().await, Some(2));
    /// assert_eq!(stream.next().await, None);
    /// # });
    /// ```
    fn next(&mut self) -> Next<'_, Self>
    where
        Self: Unpin,
    {
        Next::new(self)
    }

    /// Count the number of items in the stream.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
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
        Count::new(self)
    }

    /// Get the last element in the stream.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// assert_eq!(stream::iter(3..7).must_complete().last().await, Some(6));
    /// assert_eq!(stream::empty::<String>().must_complete().last().await, None);
    /// # });
    /// ```
    fn last(self) -> Last<Self>
    where
        Self: Sized,
    {
        Last::new(self)
    }

    /// Get the nth element in the stream.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// assert_eq!(stream::iter(3..7).must_complete().nth(2).await, Some(5));
    /// assert_eq!(stream::iter(3..7).must_complete().nth(10).await, None);
    /// # });
    /// ```
    fn nth(&mut self, n: usize) -> Nth<'_, Self>
    where
        Self: Unpin,
    {
        Nth::new(self, n)
    }

    /// Create a stream starting at the same point, but stepping by the given amount each
    /// iteration.
    ///
    /// # Panics
    ///
    /// This will panic if `step` is `0`.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let a = [0, 1, 2, 3, 4, 5];
    /// let mut stream = stream::iter(&a).must_complete().step_by(2);
    ///
    /// assert_eq!(stream.next().await, Some(&0));
    /// assert_eq!(stream.next().await, Some(&2));
    /// assert_eq!(stream.next().await, Some(&4));
    /// assert_eq!(stream.next().await, None);
    /// # });
    /// ```
    fn step_by(self, step: usize) -> StepBy<Self>
    where
        Self: Sized,
    {
        StepBy::new(self, step)
    }

    /// Chain this stream with another.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
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
        Chain::new(self, other)
    }

    // TODO: zip

    /// Map this stream's items with a closure.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
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
        Map::new(self, f)
    }

    /// Map this stream's items with an asynchronous closure.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt, completion_async_move};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion_async_move! {
    /// let mut stream = stream::iter(0..5)
    ///     .must_complete()
    ///     .then(|x| completion_async_move!(x * 2 + 4));
    ///
    /// futures_lite::pin!(stream);
    ///
    /// assert_eq!(stream.next().await, Some(4));
    /// assert_eq!(stream.next().await, Some(6));
    /// assert_eq!(stream.next().await, Some(8));
    /// assert_eq!(stream.next().await, Some(10));
    /// assert_eq!(stream.next().await, Some(12));
    /// assert_eq!(stream.next().await, None);
    /// # });
    /// ```
    fn then<F: FnMut(Self::Item) -> Fut, Fut: CompletionFuture>(self, f: F) -> Then<Self, F, Fut>
    where
        Self: Sized,
    {
        Then::new(self, f)
    }

    /// Call a closure on each item the stream.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// stream::iter(0..8).must_complete().for_each(|num| println!("{}", num)).await;
    /// # });
    /// ```
    fn for_each<F: FnMut(Self::Item)>(self, f: F) -> ForEach<Self, F>
    where
        Self: Sized,
    {
        ForEach::new(self, f)
    }

    /// Keep the values in the stream for which the predicate resolves to `true`.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let mut stream = stream::iter(1..=10).must_complete().filter(|n| n % 3 == 0);
    ///
    /// assert_eq!(stream.next().await, Some(3));
    /// assert_eq!(stream.next().await, Some(6));
    /// assert_eq!(stream.next().await, Some(9));
    /// assert_eq!(stream.next().await, None);
    /// # });
    /// ```
    fn filter<F>(self, f: F) -> Filter<Self, F>
    where
        F: FnMut(&Self::Item) -> bool,
        Self: Sized,
    {
        Filter::new(self, f)
    }

    /// Filter and map the items of the stream with a closure.
    ///
    /// This will yield any values for which the closure returns [`Some`] and will discard any
    /// values for which the closure returns [`None`].
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let strings = ["5", "!", "2", "NaN", "6", ""];
    /// let stream = stream::iter(&strings).must_complete();
    /// let mut stream = stream.filter_map(|s| s.parse::<i32>().ok());
    ///
    /// assert_eq!(stream.next().await, Some(5));
    /// assert_eq!(stream.next().await, Some(2));
    /// assert_eq!(stream.next().await, Some(6));
    /// assert_eq!(stream.next().await, None);
    /// # });
    /// ```
    fn filter_map<T, F>(self, f: F) -> FilterMap<Self, F>
    where
        F: FnMut(Self::Item) -> Option<T>,
        Self: Sized,
    {
        FilterMap::new(self, f)
    }

    /// Yield the current iteration count as well as the next value.
    ///
    /// The returned stream yields pairs `(i, val)` where `i` is the current index of iteration and
    /// `val` is the value returned by the stream.
    ///
    /// # Overflow Behaviour
    ///
    /// The method does no guarding against overflows, so enumerating more than [`usize::MAX`]
    /// elements either produces the wrong result or panics. If debug assertions are enabled, a
    /// panic is guaranteed.
    ///
    /// # Panics
    ///
    /// The returned stream might panic if the to-be-returned index would overflow a [`usize`].
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let string = "Hello";
    /// let mut stream = stream::iter(string.chars()).must_complete().enumerate();
    ///
    /// assert_eq!(stream.next().await, Some((0, 'H')));
    /// assert_eq!(stream.next().await, Some((1, 'e')));
    /// assert_eq!(stream.next().await, Some((2, 'l')));
    /// assert_eq!(stream.next().await, Some((3, 'l')));
    /// assert_eq!(stream.next().await, Some((4, 'o')));
    /// assert_eq!(stream.next().await, None);
    /// # });
    /// ```
    fn enumerate(self) -> Enumerate<Self>
    where
        Self: Sized,
    {
        Enumerate::new(self)
    }

    /// Create a stream which can use [`peek`](Peekable::peek) to look at the next element of the
    /// stream without consuming it.
    ///
    /// Note that the underlying stream is still advanced when [`peek`](Peekable::peek) is called
    /// for the first time after [`next`](Self::next); in order to retrieve the next element,
    /// [`next`](Self::next) is called on the underlying stream, hence any side effects of the
    /// method with occur.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::{stream, pin};
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let mut stream = stream::iter("Hello!\n".chars()).must_complete().peekable();
    ///
    /// let mut s = String::new();
    /// while stream.peek_unpin().await != Some(&'\n') {
    ///     s.push(stream.next().await.unwrap());
    /// }
    /// assert_eq!(s, "Hello!");
    /// assert_eq!(stream.next().await, Some('\n'));
    /// assert_eq!(stream.next().await, None);
    /// # });
    /// ```
    fn peekable(self) -> Peekable<Self>
    where
        Self: Sized,
    {
        Peekable::new(self)
    }

    /// Skip items while the predicate returns `true`.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let stream = stream::iter("   Hello world!".chars()).must_complete();
    /// let text: String = stream.skip_while(char::is_ascii_whitespace).collect().await;
    /// assert_eq!(text, "Hello world!");
    /// # });
    /// ```
    fn skip_while<P>(self, predicate: P) -> SkipWhile<Self, P>
    where
        P: FnMut(&Self::Item) -> bool,
        Self: Sized,
    {
        SkipWhile::new(self, predicate)
    }

    /// Take items while the predicate returns `true`.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let stream = stream::iter("Hello world!\nFoo bar".chars()).must_complete();
    /// let text: String = stream.take_while(|&c| c != '\n').collect().await;
    /// assert_eq!(text, "Hello world!");
    /// # });
    /// ```
    fn take_while<P>(self, predicate: P) -> TakeWhile<Self, P>
    where
        P: FnMut(&Self::Item) -> bool,
        Self: Sized,
    {
        TakeWhile::new(self, predicate)
    }

    /// Skip the first `n` items in the stream.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let stream = stream::iter(0..10).must_complete();
    /// assert_eq!(stream.skip(5).collect::<Vec<_>>().await, [5, 6, 7, 8, 9]);
    /// # });
    /// ```
    fn skip(self, n: usize) -> Skip<Self>
    where
        Self: Sized,
    {
        Skip::new(self, n)
    }

    /// Takes the first `n` items of the stream. All other items will be ignored.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let stream = stream::repeat(19).must_complete();
    /// assert_eq!(stream.take(5).collect::<Vec<_>>().await, [19, 19, 19, 19, 19]);
    /// # });
    /// ```
    fn take(self, n: usize) -> Take<Self>
    where
        Self: Sized,
    {
        Take::new(self, n)
    }

    // TODO: scan

    /// Map the stream, flattening nested structure.
    ///
    /// `.flat_map(f)` is equivalent to `.[`map`](Self::map)`(f).`[`flatten`](Self::flatten)`()`.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let s: String = stream::iter(&["alpha", "beta", "gamma"])
    ///     .must_complete()
    ///     .flat_map(|s| stream::iter(s.chars()).must_complete())
    ///     .collect()
    ///     .await;
    ///
    /// assert_eq!(s, "alphabetagamma");
    /// # });
    /// ```
    fn flat_map<U, F>(self, f: F) -> FlatMap<Self, U, F>
    where
        Self: Sized,
        U: CompletionStream,
        F: FnMut(Self::Item) -> U,
    {
        FlatMap::new(self, f)
    }

    /// Flatten nested structure in the stream.
    ///
    /// This converts a stream of streams to a stream.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let streams = vec![
    ///     stream::iter(0..5).must_complete(),
    ///     stream::iter(5..7).must_complete(),
    /// ];
    /// let v: Vec<u8> = stream::iter(streams).must_complete().flatten().collect().await;
    /// assert_eq!(v, &[0, 1, 2, 3, 4, 5, 6]);
    /// # });
    /// ```
    fn flatten(self) -> Flatten<Self>
    where
        Self: Sized,
        Self::Item: CompletionStream,
    {
        Flatten::new(self)
    }

    /// Fuse the stream so that it is guaranteed to continue to yield `None` when exhausted.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
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
        Fuse::new(self)
    }

    /// Do something with each element in the stream, passing the value on.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let sum = stream::iter(0..16)
    ///     .must_complete()
    ///     .inspect(|x| println!("about to filter: {}", x))
    ///     .filter(|x| x % 2 == 0)
    ///     .inspect(|x| println!("made it through filter: {}", x))
    ///     .fold(0_u32, |sum, i| sum + i)
    ///     .await;
    ///
    /// assert_eq!(sum, 56);
    /// # });
    /// ```
    fn inspect<F>(self, f: F) -> Inspect<Self, F>
    where
        Self: Sized,
        F: FnMut(&Self::Item),
    {
        Inspect::new(self, f)
    }

    // TODO: by_ref

    /// Collect all the items in the stream into a collection.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, completion_stream};
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let stream = completion_stream! {
    ///     for i in 0..5 {
    ///         yield i;
    ///     }
    /// };
    ///
    /// let items: Vec<_> = stream.collect().await;
    /// assert_eq!(items, [0, 1, 2, 3, 4]);
    /// # });
    /// ```
    ///
    /// You can also collect into [`Result`]s or [`Option`]s.
    ///
    /// ```
    /// use completion::{CompletionStreamExt, completion_stream};
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let success_stream = completion_stream! {
    ///     for i in 0..5 {
    ///         yield Some(i);
    ///     }
    /// };
    /// let failure_stream = completion_stream! {
    ///     for i in 0..5 {
    ///         yield Some(i);
    ///     }
    ///     yield None;
    /// };
    ///
    /// assert_eq!(success_stream.collect::<Option<Vec<_>>>().await, Some(vec![0, 1, 2, 3, 4]));
    /// assert_eq!(failure_stream.collect::<Option<Vec<_>>>().await, None);
    /// # });
    /// ```
    fn collect<C: FromCompletionStream<Self::Item>>(self) -> Collect<Self, C>
    where
        Self: Sized,
    {
        Collect::new(self)
    }

    // TODO: partition
    // TODO: try_fold
    // TODO: try_for_each

    /// Accumulate a value over a stream.
    ///
    /// `Fold` stores an accumulator that is initially set to `init`. Whenever the stream produces
    /// a new item, it calls the function `f` with the accumulator and new item, which then returns
    /// the new value of the accumulator. Once the stream is finished, it returns the accumulator.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, completion_stream};
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let stream = completion_stream! {
    ///     yield 1;
    ///     yield 8;
    ///     yield 2;
    /// };
    /// assert_eq!(stream.fold(0, |acc, x| acc + x).await, 11);
    /// # });
    /// ```
    fn fold<T, F>(self, init: T, f: F) -> Fold<Self, F, T>
    where
        F: FnMut(T, Self::Item) -> T,
        Self: Sized,
    {
        Fold::new(self, init, f)
    }

    /// Check if all the elements in the stream match a predicate.
    ///
    /// This is short-circuiting; it will stop once it finds a `false`.
    ///
    /// An empty stream returns `true`.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// assert!(stream::iter(0..10).must_complete().all(|x| x < 10).await);
    ///
    /// assert!(!stream::iter(0..8).must_complete().all(|x| x < 7).await);
    ///
    /// assert!(stream::empty::<()>().must_complete().all(|_| false).await);
    /// # });
    /// ```
    fn all<F: FnMut(Self::Item) -> bool>(&mut self, f: F) -> All<'_, Self, F>
    where
        Self: Unpin,
    {
        All::new(self, f)
    }

    /// Check if any of the elements in the stream match a predicate.
    ///
    /// This is short-circuiting; it will stop once it finds a `true`.
    ///
    /// An empty stream returns `false`.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// assert!(stream::iter(0..10).must_complete().any(|x| x == 9).await);
    ///
    /// assert!(!stream::iter(0..8).must_complete().all(|x| x == 9).await);
    ///
    /// assert!(!stream::empty::<()>().must_complete().any(|_| true).await);
    /// # });
    /// ```
    fn any<F: FnMut(Self::Item) -> bool>(&mut self, f: F) -> Any<'_, Self, F>
    where
        Self: Unpin,
    {
        Any::new(self, f)
    }

    /// Search for an element in the stream that satisfies a predicate.
    ///
    /// `find()` is short-circuiting, it will stop processing as soon as the closure returns
    /// `true`.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let mut stream = stream::iter(1..10).must_complete();
    ///
    /// // Find the first even number.
    /// assert_eq!(stream.find(|x| x % 2 == 0).await, Some(2));
    ///
    /// // Find the next odd number.
    /// assert_eq!(stream.find(|x| x % 2 == 1).await, Some(3));
    ///
    /// // Find short-circuits.
    /// assert_eq!(stream.next().await, Some(4));
    /// # });
    /// ```
    fn find<P>(&mut self, predicate: P) -> Find<'_, Self, P>
    where
        Self: Unpin,
        P: FnMut(&Self::Item) -> bool,
    {
        Find::new(self, predicate)
    }

    /// Finds the first element in a stream for which a function returns [`Some`].
    ///
    /// `stream.find_map(f).await` is equivalent to `stream.filter_map(f).await.next().await`.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let mut stream = stream::iter(&["lol", "NaN", "2", "5"]).must_complete();
    ///
    /// assert_eq!(stream.find_map(|s| s.parse().ok()).await, Some(2));
    /// assert_eq!(stream.find_map(|s| s.parse().ok()).await, Some(5));
    /// assert_eq!(stream.next().await, None);
    /// # });
    /// ```
    fn find_map<B, F>(&mut self, f: F) -> FindMap<'_, Self, F>
    where
        Self: Unpin,
        F: FnMut(Self::Item) -> Option<B>,
    {
        FindMap::new(self, f)
    }

    /// Get the index of an element in the stream.
    ///
    /// `position()` is short-circuiting, it will stop processing as soon as the closure returns
    /// `true`.
    ///
    /// # Overflow Behaviour
    ///
    /// The method does no guarding against overflows, so enumerating more than [`usize::MAX`]
    /// elements either produces the wrong result or panics. If debug assertions are enabled, a
    /// panic is guaranteed.
    ///
    /// # Panics
    ///
    /// The returned stream might panic if the to-be-returned index would overflow a [`usize`].
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let mut stream = stream::iter(1..10).must_complete();
    ///
    /// assert_eq!(stream.position(|x| x == 4).await, Some(3));
    /// assert_eq!(stream.position(|x| x == 11).await, None);
    /// # });
    /// ```
    fn position<P>(&mut self, predicate: P) -> Position<'_, Self, P>
    where
        Self: Unpin,
        P: FnMut(Self::Item) -> bool,
    {
        Position::new(self, predicate)
    }

    /// Find the maximum value in the stream.
    ///
    /// If several elements are equally maximum, the last element is returned. If the stream is
    /// empty, [`None`] is returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// assert_eq!(stream::iter(&[1, 5, 2, 7, 4]).must_complete().max().await, Some(&7));
    /// assert_eq!(stream::iter(<Vec<()>>::new()).must_complete().max().await, None);
    ///
    /// let first = 5;
    /// let second = 5;
    /// let r = stream::iter(vec![&first, &second]).must_complete().max().await.unwrap();
    /// // `first` and `second` are equal in value, but `second` is chosen because it is later.
    /// assert_eq!(r as *const _, &second as *const _);
    /// # });
    /// ```
    fn max(self) -> Max<Self>
    where
        Self: Sized,
        Self::Item: Ord,
    {
        Max::new(self)
    }

    /// Find the maximum value in the stream using the specified comparison function.
    ///
    /// If several elements are equally maximum, the last element is returned. If the stream is
    /// empty, [`None`] is returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let a = [("a", 1), ("b", 7), ("c", 2), ("d", 7), ("e", 4)];
    /// let max = stream::iter(&a).must_complete().max_by(|(_, x), (_, y)| x.cmp(y)).await;
    /// assert_eq!(max, Some(&("d", 7)));
    /// # });
    /// ```
    fn max_by<F>(self, compare: F) -> MaxBy<Self, F>
    where
        Self: Sized,
        F: FnMut(&Self::Item, &Self::Item) -> cmp::Ordering,
    {
        MaxBy::new(self, compare)
    }

    /// Find the element that gives the maximum value from the specified function.
    ///
    /// If several elements are equally maximum, the last element is returned. If the stream is
    /// empty, [`None`] is returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let a: &[i32] = &[-3, 2, 10, 5, -10];
    /// let max = stream::iter(a).must_complete().max_by_key(|x| x.abs()).await;
    /// assert_eq!(max, Some(&-10));
    /// # });
    /// ```
    fn max_by_key<B, F>(self, f: F) -> MaxByKey<Self, B, F>
    where
        Self: Sized,
        B: Ord,
        F: FnMut(&Self::Item) -> B,
    {
        MaxByKey::new(self, f)
    }

    /// Find the minimum value in the stream.
    ///
    /// If several elements are equally minimum, the first element is returned. If the stream is
    /// empty, [`None`] is returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// assert_eq!(stream::iter(&[5, 7, 1, 3, 2]).must_complete().min().await, Some(&1));
    /// assert_eq!(stream::iter(<Vec<()>>::new()).must_complete().min().await, None);
    ///
    /// let first = 5;
    /// let second = 5;
    /// let r = stream::iter(vec![&first, &second]).must_complete().min().await.unwrap();
    /// // `first` and `second` are equal in value, but `first` is chosen because it is earlier.
    /// assert_eq!(r as *const _, &first as *const _);
    /// # });
    /// ```
    fn min(self) -> Min<Self>
    where
        Self: Sized,
        Self::Item: Ord,
    {
        Min::new(self)
    }

    /// Find the minimum value in the stream using the specified comparison function.
    ///
    /// If several elements are equally minimum, the last element is returned. If the stream is
    /// empty, [`None`] is returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let a = [("a", 3), ("b", 7), ("c", 1), ("d", 1), ("e", 4)];
    /// let min = stream::iter(&a).must_complete().min_by(|(_, x), (_, y)| x.cmp(y)).await;
    /// assert_eq!(min, Some(&("c", 1)));
    /// # });
    /// ```
    fn min_by<F>(self, compare: F) -> MinBy<Self, F>
    where
        Self: Sized,
        F: FnMut(&Self::Item, &Self::Item) -> cmp::Ordering,
    {
        MinBy::new(self, compare)
    }

    /// Find the element that gives the minimum value from the specified function.
    ///
    /// If several elements are equally minimum, the last element is returned. If the stream is
    /// empty, [`None`] is returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let a: &[i32] = &[-10, 9, 23, -2, 8, 2];
    /// let min = stream::iter(a).must_complete().min_by_key(|x| x.abs()).await;
    /// assert_eq!(min, Some(&-2));
    /// # });
    /// ```
    fn min_by_key<B, F>(self, f: F) -> MinByKey<Self, B, F>
    where
        Self: Sized,
        B: Ord,
        F: FnMut(&Self::Item) -> B,
    {
        MinByKey::new(self, f)
    }

    // TODO: unzip

    /// Create a stream that copies all of its elements.
    ///
    /// This is useful when you have a stream over `&T`, but you need a stream over `T`.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let a = [1, 2, 3];
    /// let v: Vec<i32> = stream::iter(&a).must_complete().copied().collect().await;
    /// assert_eq!(v, [1, 2, 3]);
    /// # });
    /// ```
    fn copied<'a, T: Copy + 'a>(self) -> Copied<Self>
    where
        Self: CompletionStream<Item = &'a T> + Sized,
    {
        Copied::new(self)
    }

    /// Create a stream that clones all of its elements.
    ///
    /// This is useful when you have a stream over `&T`, but you need a stream over `T`.
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::{CompletionStreamExt, StreamExt};
    /// use futures_lite::stream;
    ///
    /// # completion::future::block_on(completion::completion_async! {
    /// let a = ["1".to_owned(), "2".to_owned(), "3".to_owned()];
    /// let v: Vec<String> = stream::iter(&a).must_complete().cloned().collect().await;
    /// assert_eq!(v, ["1".to_owned(), "2".to_owned(), "3".to_owned()]);
    /// # });
    /// ```
    fn cloned<'a, T: Clone + 'a>(self) -> Cloned<Self>
    where
        Self: CompletionStream<Item = &'a T> + Sized,
    {
        Cloned::new(self)
    }

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
    /// use completion::{CompletionStreamExt, StreamExt};
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
    /// use completion::{CompletionStreamExt, StreamExt};
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
    /// Make sure that each element in the stream will complete. Equivalent to
    /// [`MustComplete::new`].
    ///
    /// # Examples
    ///
    /// ```
    /// use completion::StreamExt;
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
/// Requires the `std` feature.
///
/// # Examples
///
/// ```
/// use completion::{completion_stream, stream, StreamExt};
///
/// let stream = completion_stream! {
///     yield '!';
///     yield '~';
/// };
/// futures_lite::pin!(stream);
/// assert_eq!(stream::block_on(stream).collect::<Vec<_>>(), ['!', '~']);
/// ```
#[cfg(feature = "std")]
pub fn block_on<S: CompletionStream + Unpin>(stream: S) -> BlockOn<S> {
    BlockOn { stream }
}

/// Iterator for [`block_on`].
#[derive(Debug)]
#[cfg(feature = "std")]
pub struct BlockOn<S> {
    stream: S,
}

#[cfg(feature = "std")]
impl<S: CompletionStream + Unpin> Iterator for BlockOn<S> {
    type Item = S::Item;

    fn next(&mut self) -> Option<Self::Item> {
        crate::future::block_on(self.stream.next())
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.stream.size_hint()
    }
}
#[cfg(feature = "std")]
impl<S: CompletionStream + Unpin> FusedIterator for BlockOn<Fuse<S>> {}
