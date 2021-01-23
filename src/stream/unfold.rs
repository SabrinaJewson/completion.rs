use core::fmt::{self, Debug, Formatter};
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use completion_core::{CompletionFuture, CompletionStream};
use futures_core::{ready, Stream};
use pin_project_lite::pin_project;

/// Create a stream from a seed value and an async closure.
///
/// # Examples
///
/// ```
/// use completion::{CompletionStreamExt, completion_async_move, stream};
///
/// let mut stream = stream::unfold(0, |n| completion_async_move! {
///     if n < 3 {
///         let next_n = n + 1;
///         let yielded = n * 2;
///         Some((yielded, next_n))
///     } else {
///         None
///     }
/// });
///
/// futures_lite::pin!(stream);
///
/// # completion::future::block_on(completion_async_move! {
/// assert_eq!(stream.next().await, Some(0));
/// assert_eq!(stream.next().await, Some(2));
/// assert_eq!(stream.next().await, Some(4));
/// assert_eq!(stream.next().await, None);
/// # });
/// ```
pub fn unfold<T, F, Fut, Item>(seed: T, f: F) -> Unfold<T, F, Fut>
where
    F: FnMut(T) -> Fut,
    Fut: CompletionFuture<Output = Option<(Item, T)>>,
{
    Unfold {
        state: Some(seed),
        f,
        fut: None,
    }
}

pin_project! {
    /// Stream for [`unfold`].
    #[must_use = "streams do nothing unless you use them"]
    pub struct Unfold<T, F, Fut> {
        state: Option<T>,
        f: F,
        #[pin]
        fut: Option<Fut>,
    }
}

impl<T: Debug, F, Fut: Debug> Debug for Unfold<T, F, Fut> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Unfold")
            .field("state", &self.state)
            .field("fut", &self.fut)
            .finish()
    }
}

impl<T, F, Fut, Item> CompletionStream for Unfold<T, F, Fut>
where
    F: FnMut(T) -> Fut,
    Fut: CompletionFuture<Output = Option<(Item, T)>>,
{
    type Item = Item;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        if let Some(state) = this.state.take() {
            this.fut.set(Some((this.f)(state)));
        }

        let step = ready!(this
            .fut
            .as_mut()
            .as_pin_mut()
            .expect("`Unfold` polled after completion")
            .poll(cx));

        this.fut.set(None);

        Poll::Ready(step.map(|(item, next_state)| {
            *this.state = Some(next_state);
            item
        }))
    }
}

impl<T, F, Fut, Item> Stream for Unfold<T, F, Fut>
where
    F: FnMut(T) -> Fut,
    Fut: CompletionFuture<Output = Option<(Item, T)>> + Future<Output = Option<(Item, T)>>,
{
    type Item = Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
}

/// Create a fallible stream from a seed value and a fallible async closure.
///
/// # Examples
///
/// ```
/// use completion::{CompletionStreamExt, completion_async_move, stream};
///
/// # #[derive(Debug, PartialEq, Eq)]
/// # struct SomeError;
/// let stream = stream::try_unfold(0, |n| completion_async_move! {
///     if n == 3 {
///         return Err(SomeError);
///     }
///
///     Ok(Some((n * 2, n + 1)))
/// });
///
/// futures_lite::pin!(stream);
///
/// # completion::future::block_on(completion_async_move! {
/// assert_eq!(stream.next().await, Some(Ok(0)));
/// assert_eq!(stream.next().await, Some(Ok(2)));
/// assert_eq!(stream.next().await, Some(Ok(4)));
/// assert_eq!(stream.next().await, Some(Err(SomeError)));
/// assert_eq!(stream.next().await, None);
/// # });
/// ```
pub fn try_unfold<T, E, F, Fut, Item>(seed: T, f: F) -> TryUnfold<T, F, Fut>
where
    F: FnMut(T) -> Fut,
    Fut: CompletionFuture<Output = Result<Option<(Item, T)>, E>>,
{
    TryUnfold {
        state: Some(seed),
        f,
        fut: None,
    }
}

pin_project! {
    /// Stream for [`try_unfold`].
    #[must_use = "streams do nothing unless you use them"]
    pub struct TryUnfold<T, F, Fut> {
        state: Option<T>,
        f: F,
        #[pin]
        fut: Option<Fut>,
    }
}

impl<T: Debug, F, Fut: Debug> Debug for TryUnfold<T, F, Fut> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("TryUnfold")
            .field("state", &self.state)
            .field("fut", &self.fut)
            .finish()
    }
}

impl<T, E, F, Fut, Item> CompletionStream for TryUnfold<T, F, Fut>
where
    F: FnMut(T) -> Fut,
    Fut: CompletionFuture<Output = Result<Option<(Item, T)>, E>>,
{
    type Item = Result<Item, E>;

    unsafe fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        if let Some(state) = this.state.take() {
            this.fut.set(Some((this.f)(state)));
        }

        Poll::Ready(match this.fut.as_mut().as_pin_mut() {
            Some(fut) => {
                let step = ready!(fut.poll(cx));
                this.fut.set(None);

                match step {
                    Ok(Some((item, next_state))) => {
                        *this.state = Some(next_state);
                        Some(Ok(item))
                    }
                    Ok(None) => None,
                    Err(e) => Some(Err(e)),
                }
            }
            None => {
                // The future has errored.
                None
            }
        })
    }
}

impl<T, E, F, Fut, Item> Stream for TryUnfold<T, F, Fut>
where
    F: FnMut(T) -> Fut,
    Fut: CompletionFuture<Output = Result<Option<(Item, T)>, E>>
        + Future<Output = Result<Option<(Item, T)>, E>>,
{
    type Item = Result<Item, E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        unsafe { CompletionStream::poll_next(self, cx) }
    }
}
