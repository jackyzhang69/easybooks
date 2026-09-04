//! Library surface for the EasyBooks CLI.
//!
//! The binary (`src/main.rs`) is the primary consumer; this lib exists so the
//! binary/version resolver in `bootstrap` can be unit-tested in isolation (it
//! is the only piece with non-trivial path logic and no network/IO side
//! effects). Mirrors formbro-cli's `lib.rs` shape (which exposes `bootstrap`).
pub mod bootstrap;
pub mod envelope;
