//! Core traits and types for asynchronous completion-based I/O.
//!
//! See [completion](https://crates.io/crates/completion) for utilities based on this.

#![warn(
    clippy::pedantic,
    clippy::wrong_pub_self_convention,
    rust_2018_idioms,
    missing_docs,
    unused_qualifications,
    missing_debug_implementations,
    explicit_outlives_requirements,
    unused_lifetimes,
    unsafe_op_in_unsafe_fn
)]
#![allow(
    clippy::module_name_repetitions,
    clippy::shadow_unrelated,
    clippy::mut_mut
)]

#[doc(no_inline)]
pub use std::io::{
    empty, repeat, sink, Cursor, Empty, Error, ErrorKind, IoSlice, IoSliceMut, Repeat, Result,
    SeekFrom, Sink,
};

#[macro_use]
mod util;

mod sys;

mod read;
pub use read::*;

mod buf_read;
pub use buf_read::*;

mod write;
pub use write::*;

mod seek;
pub use seek::*;
