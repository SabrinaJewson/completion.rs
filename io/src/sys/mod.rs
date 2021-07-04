cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod unix;
        pub(crate) use self::unix::*;
    } else if #[cfg(windows)] {
        mod windows;
        pub(crate) use self::windows::*;
    } else if #[cfg(target_os = "wasi")] {
        mod wasi;
        pub(crate) use self::wasi::*;
    } else {
        mod unsupported;
        pub(crate) use self::unsupported::*;
    }
}
