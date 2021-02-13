//! Futures that join iterators over futures: `zip_all`, `try_zip_all`, `race_all`, `race_ok_all`.

mod base;

mod zip;
pub use zip::{zip_all, ZipAll, ZipAllOutput};

mod try_zip;
pub use try_zip::{try_zip_all, TryZipAll, TryZipAllOutput};

mod race;
pub use race::{race_all, RaceAll};

mod race_ok;
pub use race_ok::{race_ok_all, RaceOkAll, RaceOkAllErrors};
