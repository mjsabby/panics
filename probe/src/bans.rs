//! One snippet for every entry in `clippy.toml`, in the order of that file.
//! The integer methods are in `ints.rs`.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap, LinkedList, VecDeque};
use std::fmt;
use std::future::{self, Future};
use std::io::{BufRead, IoSlice, IoSliceMut};
use std::iter::{Product, Sum};
use std::mem::MaybeUninit;
use std::num::NonZero;
use std::ops::{Add, AddAssign, Div, DivAssign, Index, IndexMut, Mul, MulAssign, Neg, Rem, RemAssign, Shl, ShlAssign, Shr, ShrAssign, Sub, SubAssign};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{Child, Command};
use std::ptr::NonNull;
use std::task::{Context, Waker};
use std::time::Duration;

macro_rules! local_must { ($e:expr) => { $e.unwrap() }; }
macro_rules! local_bail { () => { panic!("local_bail") }; }
macro_rules! local_log { ($($t:tt)*) => { println!($($t)*) }; }

// ═══ disallowed-macros ═══════════════════════════════════════════════════════════════════════

// ── Explicit panics ──
pub fn core_panic() { core::panic!("boom") }
pub fn std_panic() { panic!("boom") }
pub fn unreachable() { unreachable!() }
pub fn todo() { todo!() }
pub fn unimplemented() { unimplemented!() }
// The same macros, reached through another macro. The lints `clippy::panic`, `clippy::todo`,
// `clippy::unimplemented` and `clippy::unreachable` do not see these.
pub fn panic_in_local_macro() { local_bail!() }
pub fn panic_in_dependency_macro() { probe_dep::die!() }
// A `const fn` can also be called at run time, so a panic in one counts.
pub const fn panic_in_const_fn(x: u32) -> u32 { if x == 0 { panic!("boom") } x }

// ── Assertions ──
pub fn assert(x: u32) { assert!(x > 5) }
pub fn assert_eq(x: u32) { assert_eq!(x, 5) }
pub fn assert_ne(x: u32) { assert_ne!(x, 1) }
pub fn assert_matches(x: Option<u32>) { std::assert_matches!(x, Some(_)) }
pub fn debug_assert(x: u32) { debug_assert!(x > 5) }
pub fn debug_assert_eq(x: u32) { debug_assert_eq!(x, 5) }
pub fn debug_assert_ne(x: u32) { debug_assert_ne!(x, 1) }
pub fn debug_assert_matches(x: Option<u32>) { std::debug_assert_matches!(x, Some(_)) }
pub fn assert_in_dependency_macro(x: u32) { probe_dep::check!(x > 5) }

// ── Printing ──
// stdout is line-buffered: without a newline nothing is written yet, and nothing panics.
pub fn print(x: u32) { print!("{x}\n{x}") }
pub fn println(x: u32) { println!("{x}") }
pub fn eprint(x: u32) { eprint!("{x}") }
pub fn eprintln(x: u32) { eprintln!("{x}") }
pub fn dbg(x: u32) -> u32 { dbg!(x) }
// `clippy::print_stdout` does not see this one.
pub fn println_in_local_macro(x: u32) { local_log!("{x}") }

// ═══ disallowed-methods ══════════════════════════════════════════════════════════════════════

// ── Unwrapping ──
pub fn option_unwrap(o: Option<u32>) -> u32 { o.unwrap() }
pub fn option_expect(o: Option<u32>) -> u32 { o.expect("some") }
pub fn result_unwrap(r: Result<u32, u32>) -> u32 { r.unwrap() }
pub fn result_expect(r: Result<u32, u32>) -> u32 { r.expect("ok") }
pub fn result_unwrap_err(r: Result<u32, u32>) -> u32 { r.unwrap_err() }
pub fn result_expect_err(r: Result<u32, u32>) -> u32 { r.expect_err("err") }
// Other ways to reach the same function.
pub fn option_unwrap_as_path(v: Vec<Option<u32>>) -> Vec<u32> { v.into_iter().map(Option::unwrap).collect() }
pub fn option_unwrap_in_local_macro(o: Option<u32>) -> u32 { local_must!(o) }
pub fn option_unwrap_in_dependency_macro(o: Option<u32>) -> u32 { probe_dep::must!(o) }

// ── Explicit panics ──
pub fn panic_any() { std::panic::panic_any(42_u32) }
pub fn resume_unwind() { std::panic::resume_unwind(Box::new(42_u32)) }

// ── Run-time borrow and initialization checks ──
/// Calls `f` while a mutable borrow of `c` is alive, without using anything that is banned.
fn held<R>(c: &RefCell<u32>, f: impl FnOnce(&RefCell<u32>) -> R) -> Option<R> { let _guard = c.try_borrow_mut().ok()?; Some(f(c)) }
pub fn refcell_borrow(c: &RefCell<u32>) -> Option<u32> { held(c, |c| *c.borrow()) }
pub fn refcell_borrow_mut(c: &RefCell<u32>) -> Option<u32> { held(c, |c| *c.borrow_mut()) }
pub fn refcell_replace(c: &RefCell<u32>) -> Option<u32> { held(c, |c| c.replace(1)) }
pub fn refcell_replace_with(c: &RefCell<u32>) -> Option<u32> { held(c, |c| c.replace_with(|v| *v)) }
pub fn refcell_take(c: &RefCell<u32>) -> Option<u32> { held(c, |c| c.take()) }
pub fn refcell_swap(a: &RefCell<u32>, b: &RefCell<u32>) -> Option<()> { held(a, |a| a.swap(b)) }
pub fn oncecell_get_or_init(c: &OnceCell<u32>) -> u32 { *c.get_or_init(|| *c.get_or_init(|| 1)) }

// Thread-locals with a destructor, so that they can be in the "destroyed" state.
thread_local! {
    static TLS_CELL: Cell<String> = const { Cell::new(String::new()) };
    static TLS_REFCELL: RefCell<String> = const { RefCell::new(String::new()) };
}
/// First access registers the destructors. The harness calls this on a thread that then exits.
pub fn tls_register() { let _ = TLS_CELL.try_with(|_| ()); let _ = TLS_REFCELL.try_with(|_| ()); }
/// Calls `f` while the thread-local `RefCell` is borrowed, without using anything that is banned.
fn tls_held<R>(mutably: bool, f: impl FnOnce() -> R) -> Option<R> {
    TLS_REFCELL.try_with(|c| if mutably { let _guard = c.try_borrow_mut().ok()?; Some(f()) } else { let _guard = c.try_borrow().ok()?; Some(f()) }).ok().flatten()
}
// These four are run after the thread-local has been destroyed.
pub fn localkey_with() -> usize { TLS_CELL.with(|_| 1) }
pub fn localkey_set() { TLS_CELL.set(String::new()) }
pub fn localkey_take() -> String { TLS_CELL.take() }
pub fn localkey_replace() -> String { TLS_CELL.replace(String::new()) }
// The `RefCell` flavours of the same names, run while the value is borrowed.
pub fn localkey_refcell_set() -> Option<()> { tls_held(false, || TLS_REFCELL.set(String::new())) }
pub fn localkey_refcell_take() -> Option<String> { tls_held(false, || TLS_REFCELL.take()) }
pub fn localkey_refcell_replace() -> Option<String> { tls_held(false, || TLS_REFCELL.replace(String::new())) }
pub fn localkey_with_borrow() -> Option<usize> { tls_held(true, || TLS_REFCELL.with_borrow(|s| s.len())) }
pub fn localkey_with_borrow_mut() -> Option<usize> { tls_held(false, || TLS_REFCELL.with_borrow_mut(|s| s.len())) }

// ── Indexes, ranges and lengths: slices and str ──
pub fn index_index(v: &[u32], i: usize) -> u32 { *v.index(i) }
pub fn indexmut_index_mut(v: &mut [u32], i: usize) { *v.index_mut(i) = 1 }
pub fn slice_split_at(v: &[u8], n: usize) -> usize { v.split_at(n).0.len() }
pub fn slice_split_at_mut(v: &mut [u8], n: usize) -> usize { v.split_at_mut(n).0.len() }
pub fn slice_swap(v: &mut [u8], a: usize, b: usize) { v.swap(a, b) }
pub fn slice_copy_from_slice(dst: &mut [u8], src: &[u8]) { dst.copy_from_slice(src) }
pub fn slice_clone_from_slice(dst: &mut [u8], src: &[u8]) { dst.clone_from_slice(src) }
pub fn slice_swap_with_slice(a: &mut [u8], b: &mut [u8]) { a.swap_with_slice(b) }
pub fn slice_write_copy_of_slice(dst: &mut [MaybeUninit<u8>], src: &[u8]) -> usize { dst.write_copy_of_slice(src).len() }
pub fn slice_write_clone_of_slice(dst: &mut [MaybeUninit<u8>], src: &[u8]) -> usize { dst.write_clone_of_slice(src).len() }
pub fn slice_copy_within(v: &mut [u8], n: usize) { v.copy_within(..n, 0) }
pub fn slice_chunks(v: &[u8], n: usize) -> usize { v.chunks(n).count() }
pub fn slice_chunks_mut(v: &mut [u8], n: usize) -> usize { v.chunks_mut(n).count() }
pub fn slice_chunks_exact(v: &[u8], n: usize) -> usize { v.chunks_exact(n).count() }
pub fn slice_chunks_exact_mut(v: &mut [u8], n: usize) -> usize { v.chunks_exact_mut(n).count() }
pub fn slice_rchunks(v: &[u8], n: usize) -> usize { v.rchunks(n).count() }
pub fn slice_rchunks_mut(v: &mut [u8], n: usize) -> usize { v.rchunks_mut(n).count() }
pub fn slice_rchunks_exact(v: &[u8], n: usize) -> usize { v.rchunks_exact(n).count() }
pub fn slice_rchunks_exact_mut(v: &mut [u8], n: usize) -> usize { v.rchunks_exact_mut(n).count() }
pub fn slice_windows(v: &[u8], n: usize) -> usize { v.windows(n).count() }
pub fn slice_rotate_left(v: &mut [u8], n: usize) { v.rotate_left(n) }
pub fn slice_rotate_right(v: &mut [u8], n: usize) { v.rotate_right(n) }
pub fn slice_select_nth_unstable(v: &mut [u8], n: usize) -> u8 { *v.select_nth_unstable(n).1 }
pub fn slice_select_nth_unstable_by(v: &mut [u8], n: usize) -> u8 { *v.select_nth_unstable_by(n, |a, b| a.cmp(b)).1 }
pub fn slice_select_nth_unstable_by_key(v: &mut [u8], n: usize) -> u8 { *v.select_nth_unstable_by_key(n, |a| *a).1 }
pub fn iterator_step_by(n: usize) -> usize { (0..10).step_by(n).count() }
pub fn str_split_at(s: &str, n: usize) -> usize { s.split_at(n).0.len() }
pub fn str_split_at_mut(s: &mut str, n: usize) -> usize { s.split_at_mut(n).0.len() }

// ── Indexes, ranges and lengths: collections ──
pub fn vec_remove(v: &mut Vec<u32>, i: usize) -> u32 { v.remove(i) }
pub fn vec_swap_remove(v: &mut Vec<u32>, i: usize) -> u32 { v.swap_remove(i) }
pub fn vec_insert(v: &mut Vec<u32>, i: usize) { v.insert(i, 0) }
pub fn vec_insert_mut(v: &mut Vec<u32>, i: usize) -> u32 { *v.insert_mut(i, 0) }
pub fn vec_drain(v: &mut Vec<u32>, n: usize) -> usize { v.drain(..n).count() }
pub fn vec_splice(v: &mut Vec<u32>, n: usize) -> usize { v.splice(..n, [9]).count() }
pub fn vec_extend_from_within(v: &mut Vec<u32>, n: usize) { v.extend_from_within(..n) }
pub fn vec_extract_if(v: &mut Vec<u32>, n: usize) -> usize { v.extract_if(..n, |_| true).count() }
pub fn vec_split_off(v: &mut Vec<u32>, n: usize) -> usize { v.split_off(n).len() }
pub fn string_insert(s: &mut String, i: usize) { s.insert(i, 'x') }
pub fn string_insert_str(s: &mut String, i: usize) { s.insert_str(i, "xy") }
pub fn string_remove(s: &mut String, i: usize) -> char { s.remove(i) }
pub fn string_truncate(s: &mut String, n: usize) { s.truncate(n) }
pub fn string_split_off(s: &mut String, n: usize) -> usize { s.split_off(n).len() }
pub fn string_drain(s: &mut String, n: usize) -> usize { s.drain(..n).count() }
pub fn string_replace_range(s: &mut String, n: usize) { s.replace_range(..n, "") }
pub fn string_extend_from_within(s: &mut String, n: usize) { s.extend_from_within(..n) }
pub fn vecdeque_insert(d: &mut VecDeque<u32>, i: usize) { d.insert(i, 0) }
pub fn vecdeque_insert_mut(d: &mut VecDeque<u32>, i: usize) -> u32 { *d.insert_mut(i, 0) }
pub fn vecdeque_swap(d: &mut VecDeque<u32>, a: usize, b: usize) { d.swap(a, b) }
pub fn vecdeque_drain(d: &mut VecDeque<u32>, n: usize) -> usize { d.drain(..n).count() }
pub fn vecdeque_range(d: &VecDeque<u32>, n: usize) -> usize { d.range(..n).count() }
pub fn vecdeque_range_mut(d: &mut VecDeque<u32>, n: usize) -> usize { d.range_mut(..n).count() }
pub fn vecdeque_split_off(d: &mut VecDeque<u32>, n: usize) -> usize { d.split_off(n).len() }
pub fn vecdeque_rotate_left(d: &mut VecDeque<u32>, n: usize) { d.rotate_left(n) }
pub fn vecdeque_rotate_right(d: &mut VecDeque<u32>, n: usize) { d.rotate_right(n) }
pub fn linkedlist_split_off(l: &mut LinkedList<u32>, n: usize) -> usize { l.split_off(n).len() }
pub fn btreemap_range(m: &BTreeMap<u32, u32>, a: u32, b: u32) -> usize { m.range(a..b).count() }
pub fn btreemap_range_mut(m: &mut BTreeMap<u32, u32>, a: u32, b: u32) -> usize { m.range_mut(a..b).count() }
pub fn btreeset_range(s: &BTreeSet<u32>, a: u32, b: u32) -> usize { s.range(a..b).count() }
pub fn hashmap_get_disjoint_mut(m: &mut HashMap<u32, u32>, a: u32, b: u32) -> bool { m.get_disjoint_mut([&a, &b]).iter().all(Option::is_some) }
pub fn ioslice_advance(s: &mut IoSlice<'_>, n: usize) { s.advance(n) }
pub fn ioslice_advance_slices(s: &mut &mut [IoSlice<'_>], n: usize) { IoSlice::advance_slices(s, n) }
pub fn ioslicemut_advance(s: &mut IoSliceMut<'_>, n: usize) { s.advance(n) }
pub fn ioslicemut_advance_slices(s: &mut &mut [IoSliceMut<'_>], n: usize) { IoSliceMut::advance_slices(s, n) }
pub fn bufread_consume(mut input: &[u8], n: usize) -> usize { input.consume(n); input.len() }

// ── Radix and encoding ──
pub fn char_from_digit(radix: u32) -> Option<char> { char::from_digit(1, radix) }
pub fn core_char_from_digit(radix: u32) -> Option<char> { core::char::from_digit(1, radix) }
pub fn char_to_digit(c: char, radix: u32) -> Option<u32> { c.to_digit(radix) }
pub fn char_is_digit(c: char, radix: u32) -> bool { c.is_digit(radix) }
pub fn char_encode_utf8(c: char, buf: &mut [u8]) -> usize { c.encode_utf8(buf).len() }
pub fn char_encode_utf16(c: char, buf: &mut [u16]) -> usize { c.encode_utf16(buf).len() }
pub fn nonzero_from_str_radix(s: &str, radix: u32) -> bool { NonZero::<u8>::from_str_radix(s, radix).is_ok() }

// ── Arithmetic ──
pub fn ops_add(a: u8, b: u8) -> u8 { Add::add(a, b) }
pub fn ops_sub(a: u8, b: u8) -> u8 { Sub::sub(a, b) }
pub fn ops_mul(a: u8, b: u8) -> u8 { Mul::mul(a, b) }
pub fn ops_div(a: u8, b: u8) -> u8 { Div::div(a, b) }
pub fn ops_rem(a: u8, b: u8) -> u8 { Rem::rem(a, b) }
pub fn ops_neg(a: i8) -> i8 { a.neg() }
pub fn ops_shl(a: u8, n: u32) -> u8 { a.shl(n) }
pub fn ops_shr(a: u8, n: u32) -> u8 { a.shr(n) }
pub fn ops_add_assign(a: &mut u8, b: u8) { AddAssign::add_assign(a, b) }
pub fn ops_sub_assign(a: &mut u8, b: u8) { SubAssign::sub_assign(a, b) }
pub fn ops_mul_assign(a: &mut u8, b: u8) { MulAssign::mul_assign(a, b) }
pub fn ops_div_assign(a: &mut u8, b: u8) { DivAssign::div_assign(a, b) }
pub fn ops_rem_assign(a: &mut u8, b: u8) { RemAssign::rem_assign(a, b) }
pub fn ops_shl_assign(a: &mut u8, n: u32) { a.shl_assign(n) }
pub fn ops_shr_assign(a: &mut u8, n: u32) { a.shr_assign(n) }
// The usual way such a call gets written.
pub fn ops_add_as_path(v: &[u8]) -> u8 { v.iter().copied().reduce(u8::add).unwrap_or(0) }
pub fn iterator_sum(v: &[u8]) -> u8 { v.iter().sum() }
pub fn iterator_product(v: &[u8]) -> u8 { v.iter().product() }
// A sum that panics in release builds too.
pub fn iterator_sum_of_durations(v: &[Duration]) -> Duration { v.iter().sum() }
pub fn sum_sum(v: Vec<u8>) -> u8 { <u8 as Sum>::sum(v.into_iter()) }
pub fn product_product(v: Vec<u8>) -> u8 { <u8 as Product>::product(v.into_iter()) }
pub fn ord_clamp(x: i32, lo: i32, hi: i32) -> i32 { x.clamp(lo, hi) }
pub fn f32_clamp(x: f32, lo: f32, hi: f32) -> f32 { x.clamp(lo, hi) }
pub fn f64_clamp(x: f64, lo: f64, hi: f64) -> f64 { x.clamp(lo, hi) }
pub fn nonzero_abs(x: NonZero<i32>) -> NonZero<i32> { x.abs() }

// ── Time ──
pub fn duration_new(secs: u64, nanos: u32) -> Duration { Duration::new(secs, nanos) }
pub fn duration_from_secs_f32(secs: f32) -> Duration { Duration::from_secs_f32(secs) }
pub fn duration_from_secs_f64(secs: f64) -> Duration { Duration::from_secs_f64(secs) }
pub fn duration_mul_f32(d: Duration, x: f32) -> Duration { d.mul_f32(x) }
pub fn duration_mul_f64(d: Duration, x: f64) -> Duration { d.mul_f64(x) }
pub fn duration_div_f32(d: Duration, x: f32) -> Duration { d.div_f32(x) }
pub fn duration_div_f64(d: Duration, x: f64) -> Duration { d.div_f64(x) }
pub fn duration_from_mins(mins: u64) -> Duration { Duration::from_mins(mins) }
pub fn duration_from_hours(hours: u64) -> Duration { Duration::from_hours(hours) }
pub fn duration_from_nanos_u128(nanos: u128) -> Duration { Duration::from_nanos_u128(nanos) }

// ── Threads, processes, environment and paths ──
pub fn thread_spawn() -> bool { std::thread::spawn(|| ()).join().is_ok() }
pub fn scope_spawn() -> bool { std::thread::scope(|s| s.spawn(|| ()).join().is_ok()) }
// Both unwrap the result of reading what the child wrote.
pub fn command_output(command: &mut Command) -> bool { command.output().is_ok() }
pub fn child_wait_with_output(child: Child) -> bool { child.wait_with_output().is_ok() }
pub fn env_args() -> usize { std::env::args().count() }
pub fn env_vars() -> usize { std::env::vars().count() }
// SAFETY (both): the harness has no other thread running when it calls these, and they panic before changing anything.
pub fn env_set_var(key: &str) { unsafe { std::env::set_var(key, "x") } }
pub fn env_remove_var(key: &str) { unsafe { std::env::remove_var(key) } }
pub fn pathbuf_set_extension(p: &mut PathBuf, ext: &str) -> bool { p.set_extension(ext) }
pub fn pathbuf_add_extension(p: &mut PathBuf, ext: &str) -> bool { p.add_extension(ext) }
pub fn path_with_extension(p: &Path, ext: &str) -> PathBuf { p.with_extension(ext) }
pub fn path_with_added_extension(p: &Path, ext: &str) -> PathBuf { p.with_added_extension(ext) }

// ── Other ──
pub fn debugmap_key(f: &mut fmt::Formatter<'_>) -> fmt::Result { f.debug_map().key(&1).key(&2).finish() }
pub fn debugmap_value(f: &mut fmt::Formatter<'_>) -> fmt::Result { f.debug_map().value(&1).finish() }
pub fn ready_into_inner() -> u32 { let mut ready = future::ready(1); let _ = Pin::new(&mut ready).poll(&mut Context::from_waker(Waker::noop())); ready.into_inner() }
pub fn const_ptr_align_offset(p: *const u8, align: usize) -> usize { p.align_offset(align) }
pub fn mut_ptr_align_offset(p: *mut u8, align: usize) -> usize { p.align_offset(align) }
pub fn nonnull_align_offset(p: NonNull<u8>, align: usize) -> usize { p.align_offset(align) }
