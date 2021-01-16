//! Buffered I/O wrappers.

#[cfg(test)]
use super::test_utils;
use super::{extend_lifetime, extend_lifetime_mut};

mod buf_reader;
pub use buf_reader::*;

mod buf_writer;
pub use buf_writer::*;
