#[cfg(feature = "alloc")]
use alloc::{
    borrow::Cow,
    boxed::Box,
    collections::{BTreeMap, BTreeSet, LinkedList, VecDeque},
    rc::Rc,
    string::String,
    sync::Arc,
    vec::Vec,
};
#[cfg(feature = "std")]
use std::collections::{HashMap, HashSet};
#[cfg(feature = "std")]
use std::hash::{BuildHasher, Hash};
#[cfg(feature = "std")]
use std::path::{Path, PathBuf};

#[cfg(doc)]
use completion_core::CompletionStream;

/// Conversion from a [`CompletionStream`].
///
/// This trait is not meant to be used directly, instead call
/// [`CompletionStreamExt::collect`](super::CompletionStreamExt::collect).
///
/// This trait cannot currently be implemented by users of this library.
pub trait FromCompletionStream<T>: FromCompletionStreamInner<T> {}

pub trait FromCompletionStreamInner<T>: Sized {
    /// The intermediate type stored when building the collection.
    type Intermediate;

    /// Start converting from a stream given its bounds as returned by
    /// [`CompletionStream::size_hint`].
    fn start(lower: usize, upper: Option<usize>) -> Self::Intermediate;

    /// Add a type to the collection.
    ///
    /// This returns `Ok` to continue reading items from the stream and `Err` to finish early.
    fn push(intermediate: Self::Intermediate, item: T) -> Result<Self::Intermediate, Self>;

    /// Finialize the intermediate type into `Self`.
    fn finalize(intermediate: Self::Intermediate) -> Self;
}

impl FromCompletionStream<()> for () {}
impl FromCompletionStreamInner<()> for () {
    type Intermediate = ();

    fn start(_lower: usize, _upper: Option<usize>) -> Self::Intermediate {}
    fn push(_intermediate: Self::Intermediate, _item: ()) -> Result<Self::Intermediate, Self> {
        Ok(())
    }
    fn finalize(_intermediate: Self::Intermediate) -> Self {}
}

impl<A, V> FromCompletionStream<Option<A>> for Option<V> where V: FromCompletionStream<A> {}
impl<A, V> FromCompletionStreamInner<Option<A>> for Option<V>
where
    V: FromCompletionStream<A>,
{
    type Intermediate = V::Intermediate;

    fn start(lower: usize, upper: Option<usize>) -> Self::Intermediate {
        V::start(lower, upper)
    }
    fn push(intermediate: Self::Intermediate, item: Option<A>) -> Result<Self::Intermediate, Self> {
        match item {
            Some(item) => V::push(intermediate, item).map_err(Some),
            None => Err(None),
        }
    }
    fn finalize(intermediate: Self::Intermediate) -> Self {
        Some(V::finalize(intermediate))
    }
}

impl<A, E, V> FromCompletionStream<Result<A, E>> for Result<V, E> where V: FromCompletionStream<A> {}
impl<A, E, V> FromCompletionStreamInner<Result<A, E>> for Result<V, E>
where
    V: FromCompletionStream<A>,
{
    type Intermediate = V::Intermediate;

    fn start(lower: usize, upper: Option<usize>) -> Self::Intermediate {
        V::start(lower, upper)
    }
    fn push(
        intermediate: Self::Intermediate,
        item: Result<A, E>,
    ) -> Result<Self::Intermediate, Self> {
        match item {
            Ok(item) => V::push(intermediate, item).map_err(Ok),
            Err(e) => Err(Err(e)),
        }
    }
    fn finalize(intermediate: Self::Intermediate) -> Self {
        Ok(V::finalize(intermediate))
    }
}

#[cfg(feature = "alloc")]
impl FromCompletionStream<char> for String {}
#[cfg(feature = "alloc")]
impl FromCompletionStreamInner<char> for String {
    type Intermediate = String;

    fn start(lower: usize, _upper: Option<usize>) -> Self::Intermediate {
        String::with_capacity(lower)
    }
    fn push(mut intermediate: Self::Intermediate, item: char) -> Result<Self::Intermediate, Self> {
        intermediate.push(item);
        Ok(intermediate)
    }
    fn finalize(intermediate: Self::Intermediate) -> Self {
        intermediate
    }
}

#[cfg(feature = "alloc")]
impl FromCompletionStream<&char> for String {}
#[cfg(feature = "alloc")]
impl<'a> FromCompletionStreamInner<&'a char> for String {
    type Intermediate = String;

    fn start(lower: usize, _upper: Option<usize>) -> Self::Intermediate {
        String::with_capacity(lower)
    }
    fn push(
        mut intermediate: Self::Intermediate,
        item: &'a char,
    ) -> Result<Self::Intermediate, Self> {
        intermediate.push(*item);
        Ok(intermediate)
    }
    fn finalize(intermediate: Self::Intermediate) -> Self {
        intermediate
    }
}

#[cfg(feature = "alloc")]
macro_rules! impl_from_completion_stream_inner_for_string {
    ($($t:ty),*) => {
        $(
            #[allow(single_use_lifetimes, unused_lifetimes)]
            impl<'a> FromCompletionStream<$t> for String {}
            #[allow(unused_lifetimes)]
            impl<'a> FromCompletionStreamInner<$t> for String {
                type Intermediate = String;

                fn start(lower: usize, _upper: Option<usize>) -> Self::Intermediate {
                    String::with_capacity(lower)
                }
                fn push(mut intermediate: Self::Intermediate, item: $t) -> Result<Self::Intermediate, Self> {
                    intermediate.push_str(&*item);
                    Ok(intermediate)
                }
                fn finalize(intermediate: Self::Intermediate) -> Self {
                    intermediate
                }
            }
        )*
    }
}
#[cfg(feature = "alloc")]
impl_from_completion_stream_inner_for_string!(&'a str, Box<str>, String, Cow<'a, str>);

#[cfg(feature = "std")]
impl<P: AsRef<Path>> FromCompletionStream<P> for PathBuf {}
#[cfg(feature = "std")]
impl<P: AsRef<Path>> FromCompletionStreamInner<P> for PathBuf {
    type Intermediate = PathBuf;

    fn start(lower: usize, _upper: Option<usize>) -> Self::Intermediate {
        PathBuf::with_capacity(lower)
    }
    fn push(mut intermediate: Self::Intermediate, item: P) -> Result<Self::Intermediate, Self> {
        intermediate.push(item.as_ref());
        Ok(intermediate)
    }
    fn finalize(intermediate: Self::Intermediate) -> Self {
        intermediate
    }
}

#[cfg(feature = "alloc")]
macro_rules! impl_from_completion_stream_inner_for_veclike {
    ($($t:ty),*) => {
        $(
            impl<T> FromCompletionStream<T> for $t {}
            impl<T> FromCompletionStreamInner<T> for $t {
                type Intermediate = Vec<T>;

                fn start(lower: usize, _upper: Option<usize>) -> Self::Intermediate {
                    Vec::with_capacity(lower)
                }
                fn push(mut intermediate: Self::Intermediate, item: T) -> Result<Self::Intermediate, Self> {
                    intermediate.push(item);
                    Ok(intermediate)
                }
                fn finalize(intermediate: Self::Intermediate) -> Self {
                    intermediate.into()
                }
            }
        )*
    }
}
#[cfg(feature = "alloc")]
impl_from_completion_stream_inner_for_veclike!(Box<[T]>, Rc<[T]>, Arc<[T]>, Vec<T>);

#[cfg(feature = "alloc")]
impl<T> FromCompletionStream<T> for VecDeque<T> {}
#[cfg(feature = "alloc")]
impl<T> FromCompletionStreamInner<T> for VecDeque<T> {
    type Intermediate = VecDeque<T>;

    fn start(lower: usize, _upper: Option<usize>) -> Self::Intermediate {
        VecDeque::with_capacity(lower)
    }
    fn push(mut intermediate: Self::Intermediate, item: T) -> Result<Self::Intermediate, Self> {
        intermediate.push_back(item);
        Ok(intermediate)
    }
    fn finalize(intermediate: Self::Intermediate) -> Self {
        intermediate
    }
}

#[cfg(feature = "alloc")]
impl<T> FromCompletionStream<T> for LinkedList<T> {}
#[cfg(feature = "alloc")]
impl<T> FromCompletionStreamInner<T> for LinkedList<T> {
    type Intermediate = LinkedList<T>;

    fn start(_lower: usize, _upper: Option<usize>) -> Self::Intermediate {
        LinkedList::new()
    }
    fn push(mut intermediate: Self::Intermediate, item: T) -> Result<Self::Intermediate, Self> {
        intermediate.push_back(item);
        Ok(intermediate)
    }
    fn finalize(intermediate: Self::Intermediate) -> Self {
        intermediate
    }
}

#[cfg(feature = "alloc")]
impl<K: Ord, V> FromCompletionStream<(K, V)> for BTreeMap<K, V> {}
#[cfg(feature = "alloc")]
impl<K: Ord, V> FromCompletionStreamInner<(K, V)> for BTreeMap<K, V> {
    type Intermediate = BTreeMap<K, V>;

    fn start(_lower: usize, _upper: Option<usize>) -> Self::Intermediate {
        BTreeMap::new()
    }
    fn push(
        mut intermediate: Self::Intermediate,
        (key, value): (K, V),
    ) -> Result<Self::Intermediate, Self> {
        intermediate.insert(key, value);
        Ok(intermediate)
    }
    fn finalize(intermediate: Self::Intermediate) -> Self {
        intermediate
    }
}

#[cfg(feature = "alloc")]
impl<T: Ord> FromCompletionStream<T> for BTreeSet<T> {}
#[cfg(feature = "alloc")]
impl<T: Ord> FromCompletionStreamInner<T> for BTreeSet<T> {
    type Intermediate = BTreeSet<T>;

    fn start(_lower: usize, _upper: Option<usize>) -> Self::Intermediate {
        BTreeSet::new()
    }
    fn push(mut intermediate: Self::Intermediate, value: T) -> Result<Self::Intermediate, Self> {
        intermediate.insert(value);
        Ok(intermediate)
    }
    fn finalize(intermediate: Self::Intermediate) -> Self {
        intermediate
    }
}

#[cfg(feature = "std")]
impl<K, V, S> FromCompletionStream<(K, V)> for HashMap<K, V, S>
where
    K: Eq + Hash,
    S: BuildHasher + Default,
{
}
#[cfg(feature = "std")]
impl<K, V, S> FromCompletionStreamInner<(K, V)> for HashMap<K, V, S>
where
    K: Eq + Hash,
    S: BuildHasher + Default,
{
    type Intermediate = HashMap<K, V, S>;

    fn start(lower: usize, _upper: Option<usize>) -> Self::Intermediate {
        HashMap::with_capacity_and_hasher(lower, S::default())
    }
    fn push(
        mut intermediate: Self::Intermediate,
        (key, value): (K, V),
    ) -> Result<Self::Intermediate, Self> {
        intermediate.insert(key, value);
        Ok(intermediate)
    }
    fn finalize(intermediate: Self::Intermediate) -> Self {
        intermediate
    }
}

#[cfg(feature = "std")]
impl<T, S> FromCompletionStream<T> for HashSet<T, S>
where
    T: Eq + Hash,
    S: BuildHasher + Default,
{
}

#[cfg(feature = "std")]
impl<T, S> FromCompletionStreamInner<T> for HashSet<T, S>
where
    T: Eq + Hash,
    S: BuildHasher + Default,
{
    type Intermediate = HashSet<T, S>;

    fn start(lower: usize, _upper: Option<usize>) -> Self::Intermediate {
        HashSet::with_capacity_and_hasher(lower, S::default())
    }
    fn push(mut intermediate: Self::Intermediate, value: T) -> Result<Self::Intermediate, Self> {
        intermediate.insert(value);
        Ok(intermediate)
    }
    fn finalize(intermediate: Self::Intermediate) -> Self {
        intermediate
    }
}
