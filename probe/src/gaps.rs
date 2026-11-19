//! Panics that nothing flags: neither the ban lists nor the operator lints.
//! The doc comment of each snippet is the explanation that ends up in docs/COVERAGE.md.

use std::cell::{LazyCell, RefCell};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::ffi::CStr;
use std::fmt;
use std::future::{self, Future};
use std::io::{self, Write};
use std::ops::Index;
use std::pin::pin;
use std::sync::atomic::{self, AtomicU32};
use std::task::{Context, Waker};
use std::thread;

fn held<R>(c: &RefCell<u32>, f: impl FnOnce(&RefCell<u32>) -> R) -> Option<R> { let _guard = c.try_borrow_mut().ok()?; Some(f(c)) }

/// A `Display` implementation that reports an error although nothing went wrong.
pub struct FailingDisplay;
impl fmt::Display for FailingDisplay { fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result { Err(fmt::Error) } }

thread_local! { static LAZY: LazyCell<u32> = LazyCell::new(|| LAZY.try_with(|lazy| **lazy).unwrap_or(0)); }

// ── Code that is not yours ──

/// A function in a dependency panics. Clippy only looks at the crate it is run on.
pub fn dependency_function(v: &[u32]) -> u32 { probe_dep::first(v) }

// ── Trait implementations, which clippy.toml cannot name ──

/// `RefCell`'s `Clone` panics while the value is mutably borrowed.
pub fn refcell_clone(c: &RefCell<u32>) -> Option<RefCell<u32>> { held(c, |c| c.clone()) }
/// `RefCell`'s `PartialEq` panics while either value is mutably borrowed.
pub fn refcell_eq(a: &RefCell<u32>, b: &RefCell<u32>) -> Option<bool> { held(a, |a| a == b) }
/// `RefCell`'s `Ord` and `PartialOrd` panic while either value is mutably borrowed.
pub fn refcell_cmp(a: &RefCell<u32>, b: &RefCell<u32>) -> Option<Ordering> { held(a, |a| a.cmp(b)) }
/// Dereferencing a `LazyCell` from inside its own initializer panics.
pub fn lazycell_reentrant() -> u32 { LAZY.try_with(|lazy| **lazy).unwrap_or(0) }

// ── Operators the operator lints do not look at ──

/// `indexing_slicing` flags `map[&key]` on an owned `HashMap` or a field, but not through a reference.
pub fn index_hashmap_through_ref(m: &HashMap<u32, u32>, k: u32) -> u32 { m[&k] }
/// The same for `BTreeMap`.
pub fn index_btreemap_through_ref(m: &BTreeMap<u32, u32>, k: u32) -> u32 { m[&k] }
/// The same for `VecDeque`.
pub fn index_vecdeque_through_ref(d: &VecDeque<u32>, i: usize) -> u32 { d[i] }
/// `indexing_slicing` does not flag indexing a generic `T: Index`.
pub fn index_generic<T: Index<usize, Output = u32> + ?Sized>(t: &T, i: usize) -> u32 { t[i] }
/// `arithmetic_side_effects` ignores shifts. Shifting by the bit width or more panics when overflow checks are on.
pub fn shift_left(a: u32, n: u32) -> u32 { a << n }
/// The same for `>>`.
pub fn shift_right(a: u32, n: u32) -> u32 { a >> n }
/// The same for `<<=`.
pub fn shift_assign(mut a: u32, n: u32) -> u32 { a <<= n; a }
/// `arithmetic_side_effects` accepts division by any non-zero literal, but `MIN / -1` overflows, in every build. The optional `integer_division_remainder_used` flags it.
pub fn divide_by_minus_one(a: i32) -> i32 { a / -1 }
/// It reads the literal `-1` as 1, so multiplying by it passes too. `MIN * -1` overflows.
pub fn times_minus_one(mut x: i64) -> i64 { x *= -1; x }
/// It takes a literal `0` next to `-` for a no-op. `0 - x` overflows for `MIN`, and for every unsigned `x` but 0.
pub fn zero_minus(x: i32) -> i32 { 0 - x }
/// It skips everything inside a `static` or `const` item, including closures that run later. The optional `integer_division_remainder_used` flags the divisions there.
pub fn arithmetic_in_static(i: usize, a: u32, b: u32) -> u32 { static OPS: [fn(u32, u32) -> u32; 2] = [|a, b| a / b, |a, b| a % b]; OPS.get(i).map_or(0, |op| op(a, b)) }
/// `indexing_slicing` gives up on an array whose length is a const generic, for ranges and for constant indexes.
pub fn const_generic_array_range<const N: usize>(a: &[u8; N], n: usize) -> &[u8] { &a[..n] }
/// It does not look at an index that is written as an inline `const` block.
pub fn inline_const_index<T>(table: [u8; 4], _: &T) -> u8 { table[const { size_of::<T>() }] }
/// Neither indexing lint looks at `CStr`.
pub fn cstr_slice(c: &CStr, n: usize) -> &CStr { &c[n..] }
/// An open range computes the next value before it hands out the current one, and that addition is not an operator in your code.
pub fn range_from_past_max(start: u8, n: usize) -> Option<u8> { (start..).nth(n) }
/// For `char`, and for IP addresses, the open range panics in every build.
pub fn range_from_char_max(c: char) -> Option<char> { (c..).next() }

// ── Your code, called from inside the standard library ──

/// `sort_by` panics if the comparison is not a total order, which this popular replacement for `partial_cmp(..).unwrap()` is not once NaN is involved. Use `f64::total_cmp`.
pub fn sort_by_partial_cmp(v: &mut [f64]) { v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal)) }
/// `format!` panics if a `Display` implementation returns an error.
pub fn format_failing_display(d: &FailingDisplay) -> String { format!("{d}") }
/// So does `to_string`.
pub fn to_string_failing_display(d: &FailingDisplay) -> String { d.to_string() }
/// So does `write!` to an `io::Write` (to a `fmt::Write` it returns the error).
pub fn write_failing_display(w: &mut Vec<u8>, d: &FailingDisplay) -> io::Result<()> { write!(w, "{d}") }

// ── Arguments that are checked at run time, on functions too common to ban ──

/// A width or precision taken from an argument must fit in 16 bits.
pub fn format_width(width: usize) -> String { format!("{:width$}", 1) }
/// `store` rejects `Acquire` and `AcqRel`. rustc's `invalid_atomic_ordering` only catches an ordering written as a constant.
pub fn atomic_store_ordering(a: &AtomicU32, order: atomic::Ordering) { a.store(1, order) }
/// `Builder::spawn`, the replacement for `thread::spawn`, panics if the thread name contains a NUL byte.
pub fn thread_name_with_nul(name: String) -> bool { thread::Builder::new().name(name).spawn(|| ()).is_ok() }
/// `count` panics if there are more than `usize::MAX` items. A range gets there without iterating: any `u128` range, or `0..=usize::MAX`.
pub fn range_count(n: u128) -> usize { (0..n).count() }
/// A capacity above `isize::MAX` bytes panics (a merely too large one aborts). The optional block in clippy.toml bans this call.
pub fn vec_with_capacity(n: usize) -> Vec<u64> { Vec::with_capacity(n) }
/// The same through `vec!`, which cannot be banned without banning every `vec!`.
pub fn vec_macro_with_len(n: usize) -> Vec<u64> { vec![0; n] }
/// The same through `repeat`.
pub fn str_repeat(s: &str, n: usize) -> String { s.repeat(n) }

// ── Futures ──

/// Polling an `async` block again after it has completed panics.
pub fn poll_async_block_twice() -> bool { let mut fut = pin!(async {}); let mut cx = Context::from_waker(Waker::noop()); fut.as_mut().poll(&mut cx).is_ready() && fut.as_mut().poll(&mut cx).is_ready() }
/// So does polling `future::ready` again.
pub fn poll_ready_twice() -> bool { let mut fut = pin!(future::ready(1)); let mut cx = Context::from_waker(Waker::noop()); fut.as_mut().poll(&mut cx).is_ready() && fut.as_mut().poll(&mut cx).is_ready() }
