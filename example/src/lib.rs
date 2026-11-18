//! Code that passes the lints in this repository.
//!
//! Each function shows the usual replacement for something that `clippy.toml` or one of the
//! operator lints rejects. The last two items show the other way out, for a call that provably
//! cannot panic: an `#[expect]` that says why.
//!
//! `cargo clippy -p example --all-targets -- -D warnings` must stay clean.

use std::cell::RefCell;
use std::fmt;
use std::io::{self, Write};
use std::thread;
use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Truncated,
    Overflow,
    DivisionByZero,
    Busy,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::Truncated => "input ends too early",
            Error::Overflow => "number out of range",
            Error::DivisionByZero => "division by zero",
            Error::Busy => "value is in use",
        })
    }
}

impl std::error::Error for Error {}

/// Derived implementations are not affected by any of the lints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Reading {
    pub sensor: u8,
    pub value: u32,
}

/// Sorting, deduplicating and searching need no index arithmetic.
pub fn highest(readings: &[Reading]) -> Option<Reading> {
    let mut sorted = readings.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    sorted.last().copied()
}

/// Indexing: `get` instead of `values[index]`.
pub fn nth(values: &[u32], index: usize) -> Option<u32> {
    values.get(index).copied()
}

/// Slicing: `split_at_checked` instead of `split_at` or `&bytes[..n]`.
pub fn header_and_body(bytes: &[u8], header_len: usize) -> Result<(&[u8], &[u8]), Error> {
    bytes.split_at_checked(header_len).ok_or(Error::Truncated)
}

/// Fixed-size prefixes: `split_first_chunk` instead of indexing four bytes one by one.
pub fn read_u32(bytes: &[u8]) -> Result<(u32, &[u8]), Error> {
    let (head, rest) = bytes.split_first_chunk::<4>().ok_or(Error::Truncated)?;
    Ok((u32::from_be_bytes(*head), rest))
}

/// Strings: `get` instead of `&s[..len]`, which panics inside a multi-byte character.
pub fn prefix(s: &str, len: usize) -> Option<&str> {
    s.get(..len)
}

/// Arithmetic: `checked_*` instead of operators, and `try_fold` instead of `sum`.
pub fn average(values: &[u32]) -> Result<u32, Error> {
    let sum = values
        .iter()
        .try_fold(0_u32, |acc, v| acc.checked_add(*v))
        .ok_or(Error::Overflow)?;
    let count = u32::try_from(values.len()).map_err(|_| Error::Overflow)?;
    sum.checked_div(count).ok_or(Error::DivisionByZero)
}

/// Chunks of a constant size: `as_chunks` takes the size as a const generic, so it cannot be zero.
pub fn pair_sums(values: &[u32]) -> Vec<u32> {
    let (pairs, _rest) = values.as_chunks::<2>();
    pairs.iter().map(|[a, b]| a.saturating_add(*b)).collect()
}

/// `RefCell`: `try_borrow_mut` instead of `borrow_mut`.
pub fn bump(counter: &RefCell<u32>) -> Result<u32, Error> {
    let mut value = counter.try_borrow_mut().map_err(|_| Error::Busy)?;
    *value = value.checked_add(1).ok_or(Error::Overflow)?;
    Ok(*value)
}

/// Output: `writeln!` on a writer instead of `println!`, so that a closed pipe is an error to
/// handle and not a panic. Pass `&mut io::stdout().lock()` to print.
pub fn write_report(out: &mut impl Write, values: &[u32]) -> io::Result<()> {
    for (line, value) in values.iter().enumerate() {
        writeln!(out, "{line}: {value}")?;
    }
    Ok(())
}

/// Threads: `Builder::spawn` returns an error if the thread cannot be created; `thread::spawn` panics.
pub fn in_background(job: impl FnOnce() -> u32 + Send + 'static) -> io::Result<Option<u32>> {
    let worker = thread::Builder::new().spawn(job)?;
    Ok(worker.join().ok())
}

/// Time: `try_from_secs_f64` and `checked_add` instead of `from_secs_f64` and `+`.
pub fn deadline(base: Duration, extra_secs: f64) -> Option<Duration> {
    base.checked_add(Duration::try_from_secs_f64(extra_secs).ok()?)
}

/// The escape hatch. `clamp` panics if `min > max`; here the bounds are literals, so it cannot.
/// `expect` rather than `allow`: if the lint ever stops firing here, the build says so.
#[expect(
    clippy::disallowed_methods,
    reason = "the bounds are literals with min <= max"
)]
pub fn percent(value: u32) -> u32 {
    value.clamp(0, 100)
}

// A compile-time assertion never panics at run time, but the ban on `assert!` cannot tell.
#[expect(clippy::disallowed_macros, reason = "evaluated at compile time")]
const _: () = assert!(size_of::<u32>() == 4);

/// Tests may panic: that is how they fail. One `allow` on the test module switches the lints off.
#[cfg(test)]
#[allow(
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects,
    reason = "a test is supposed to panic when something is wrong"
)]
mod tests {
    use super::*;

    #[test]
    fn replacements_report_errors_instead_of_panicking() {
        assert_eq!(nth(&[1, 2, 3], 1), Some(2));
        assert_eq!(nth(&[1, 2, 3], 7), None);
        assert_eq!(
            header_and_body(b"abcdef", 2).unwrap(),
            (&b"ab"[..], &b"cdef"[..])
        );
        assert_eq!(header_and_body(b"a", 2), Err(Error::Truncated));
        assert_eq!(read_u32(&[0, 0, 1, 0, 9]).unwrap(), (256, &[9][..]));
        assert_eq!(read_u32(&[0, 0]), Err(Error::Truncated));
        assert_eq!(prefix("héllo", 1), Some("h"));
        assert_eq!(prefix("héllo", 2), None);
        assert_eq!(average(&[1, 2, 3]), Ok(2));
        assert_eq!(average(&[]), Err(Error::DivisionByZero));
        assert_eq!(average(&[u32::MAX, 1]), Err(Error::Overflow));
        assert_eq!(pair_sums(&[1, 2, 3, 4, 5]), [3, 7]);
        let (low, high) = (
            Reading {
                sensor: 1,
                value: 5,
            },
            Reading {
                sensor: 2,
                value: 1,
            },
        );
        assert_eq!(highest(&[high, low, high]), Some(high));
        assert_eq!(highest(&[]), None);
        assert_eq!(percent(250), 100);
        assert_eq!(
            deadline(Duration::from_secs(1), 0.5),
            Some(Duration::from_millis(1500))
        );
        assert_eq!(deadline(Duration::from_secs(1), -1.0), None);
        assert_eq!(in_background(|| 7).unwrap(), Some(7));
    }

    #[test]
    fn a_busy_refcell_is_an_error() {
        let counter = RefCell::new(0);
        assert_eq!(bump(&counter), Ok(1));
        let guard = counter.borrow();
        assert_eq!(bump(&counter), Err(Error::Busy));
        drop(guard);
    }

    #[test]
    fn a_report_goes_to_any_writer() {
        let mut out = Vec::new();
        write_report(&mut out, &[10, 20]).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "0: 10\n1: 20\n");
    }
}
