//! Probe for the lints in this repository.
//!
//! Every public function here is a snippet that panics for some input. `src/main.rs` calls
//! each one with such an input and reports what happened, and `tools/verify.py` compares that
//! report with what clippy says about this crate.
//!
//! * [`bans`]: one snippet for every entry in `clippy.toml`. Clippy must flag each of them.
//! * [`ints`]: the same for the integer methods, which exist once per integer type.
//! * [`lints`]: operators. One of the three operator lints must flag each of them.
//! * [`gaps`]: panics that nothing flags. These are the limits of the approach.
//!
//! This crate is meant to fail `cargo clippy`.

// One snippet per line reads better than what rustfmt would make of these files.
#[rustfmt::skip]
pub mod bans;
#[rustfmt::skip]
pub mod gaps;
#[rustfmt::skip]
pub mod ints;
#[rustfmt::skip]
pub mod lints;
