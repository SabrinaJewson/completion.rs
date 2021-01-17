# completion

Utilities for writing completion-based asynchronous code.

A completion future is a future that must be run to completion, unlike regular futures which
can be dropped and stopped at any time without the future's knowledge. This allows for more
flexibility for the implementor of the future and allows APIs like `io_uring` and IOCP to be
wrapped in a zero-cost way.

This is based off [this RFC by Matthias247](https://github.com/Matthias247/rfcs/pull/1).

## Features

- `std`: Enables features that require the standard library, on by default.
- `alloc`: Enables features that require allocation, on by default.
- `macro`: Enables the [`completion`], [`completion_async`], [`completion_async_move`] and
[`completion_stream`] macros, on by default.
