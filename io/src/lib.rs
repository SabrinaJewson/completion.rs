//! Core types and traits for asynchronous completion-based I/O.

#![warn(
    clippy::pedantic,
    clippy::wrong_pub_self_convention,
    rust_2018_idioms,
    missing_docs,
    unused_qualifications,
    missing_debug_implementations,
    single_use_lifetimes,
    explicit_outlives_requirements,
    unused_lifetimes
)]
#![allow(
    clippy::module_name_repetitions,
    clippy::shadow_unrelated,
    clippy::mut_mut
)]

pub use std::io::{
    empty, repeat, sink, Cursor, Empty, Error, IoSlice, Repeat, Result, SeekFrom, Sink,
};

mod read;
pub use read::*;

mod buf_read;
pub use buf_read::*;

mod write;
pub use write::*;

mod seek;
pub use seek::*;
