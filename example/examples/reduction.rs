//! The one place where following the lints costs run time: adding up integers in a hot loop.
//!
//!     cargo run --release --example reduction
//!
//! `sum()` is banned, because it overflows like `+` does. The obvious replacement checks every
//! addition and stops at the first overflow, and a loop that can stop early is not vectorized.
//! This program times that replacement against `sum()` and against the two ways around it.
//! Everything else the lints make you write (`get` for indexing, `checked_div` for `/`) costs
//! the same as what it replaces.

use std::hint::black_box;
use std::io::{self, Write};
use std::time::Instant;

/// Banned. Panics on overflow when overflow checks are on, wraps when they are off.
#[expect(
    clippy::disallowed_methods,
    reason = "the baseline this example measures against"
)]
#[inline(never)]
fn sum(values: &[u32]) -> u32 {
    values.iter().sum()
}

/// The obvious replacement. Exact, and slower: the early exit keeps the loop from being vectorized.
#[inline(never)]
fn checked(values: &[u32]) -> Option<u32> {
    values.iter().try_fold(0_u32, |acc, v| acc.checked_add(*v))
}

/// As fast as `sum()`. Wraps on overflow, which is what `sum()` does in a release build anyway.
#[inline(never)]
fn wrapping(values: &[u32]) -> u32 {
    values.iter().fold(0, |acc, v| acc.wrapping_add(*v))
}

/// Exact and vectorized: add into a type that cannot overflow, and convert back at the end.
#[inline(never)]
fn widened(values: &[u32]) -> Option<u32> {
    // Up to 2^32 values of at most 2^32 - 1 each fit in a u64.
    if u32::try_from(values.len()).is_err() {
        return checked(values);
    }
    let total = values
        .iter()
        .fold(0_u64, |acc, v| acc.wrapping_add(u64::from(*v)));
    u32::try_from(total).ok()
}

/// Best of nine runs, in nanoseconds per element.
fn time<R>(values: &[u32], f: impl Fn(&[u32]) -> R) -> f64 {
    let mut best = f64::INFINITY;
    for _ in 0..9 {
        let start = Instant::now();
        black_box(f(black_box(values)));
        best = best.min(start.elapsed().as_secs_f64());
    }
    best * 1e9 / values.len() as f64
}

fn main() -> io::Result<()> {
    let values: Vec<u32> = (0..1_u32 << 22).map(|i| i & 1023).collect();
    let mut out = io::stdout().lock();
    writeln!(
        out,
        "adding up {} u32 values, nanoseconds per value",
        values.len()
    )?;
    writeln!(
        out,
        "  {:6.3}  iter().sum()                          banned",
        time(&values, sum)
    )?;
    writeln!(
        out,
        "  {:6.3}  try_fold with checked_add             exact",
        time(&values, checked)
    )?;
    writeln!(
        out,
        "  {:6.3}  fold with wrapping_add                wraps on overflow",
        time(&values, wrapping)
    )?;
    writeln!(
        out,
        "  {:6.3}  fold into a u64, then u32::try_from   exact",
        time(&values, widened)
    )?;
    Ok(())
}
