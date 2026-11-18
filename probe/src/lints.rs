//! Operators. They cannot be named in `clippy.toml`; the three operator lints must flag these.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::num::{NonZero, Saturating, Wrapping};
use std::ops::Add;
use std::time::{Duration, Instant, SystemTime};

pub struct Holder { pub v: Vec<u32>, pub s: String, pub map: HashMap<u32, u32>, pub tree: BTreeMap<u32, u32>, pub deque: VecDeque<u32> }

// ── indexing_slicing ──
pub fn index_slice(v: &[u32], i: usize) -> u32 { v[i] }
pub fn index_array(a: [u32; 4], i: usize) -> u32 { a[i] }
pub fn index_array_ref(a: &[u32; 4], i: usize) -> u32 { a[i] }
pub fn index_vec(h: &Holder, i: usize) -> u32 { h.v[i] }
pub fn index_nested(v: &[Vec<u32>], i: usize, j: usize) -> u32 { v[i][j] }
pub fn index_assign(v: &mut [u32], i: usize) { v[i] = 1 }
pub fn index_bytes(s: &str, i: usize) -> u8 { s.as_bytes()[i] }
pub fn slice_range(v: &[u32], a: usize, b: usize) -> &[u32] { &v[a..b] }
pub fn slice_range_from(v: &[u32], a: usize) -> &[u32] { &v[a..] }
pub fn slice_range_inclusive(v: &[u32], b: usize) -> &[u32] { &v[..=b] }
// Maps and deques are flagged when indexed as an owned value or a field...
pub fn index_hashmap_owned(m: HashMap<u32, u32>, k: u32) -> u32 { m[&k] }
pub fn index_btreemap_owned(m: BTreeMap<u32, u32>, k: u32) -> u32 { m[&k] }
pub fn index_vecdeque_owned(d: VecDeque<u32>, i: usize) -> u32 { d[i] }
pub fn index_hashmap_field(h: &Holder, k: u32) -> u32 { h.map[&k] }
pub fn index_btreemap_field(h: &Holder, k: u32) -> u32 { h.tree[&k] }
pub fn index_vecdeque_field(h: &Holder, i: usize) -> u32 { h.deque[i] }
// ...but not through a reference: see `gaps::index_hashmap_through_ref`.

// ── string_slice ──
pub fn str_slice(s: &str, n: usize) -> &str { &s[..n] }
pub fn string_slice(h: &Holder, n: usize) -> &str { &h.s[n..] }

// ── arithmetic_side_effects: integers ──
pub fn add(a: u8, b: u8) -> u8 { a + b }
pub fn sub(a: usize, b: usize) -> usize { a - b }
pub fn mul(a: u32, b: u32) -> u32 { a * b }
pub fn div(a: u32, b: u32) -> u32 { a / b }
pub fn rem(a: u32, b: u32) -> u32 { a % b }
pub fn div_overflow(a: i32, b: i32) -> i32 { a / b }
pub fn rem_overflow(a: i32, b: i32) -> i32 { a % b }
pub fn neg(a: i32) -> i32 { -a }
pub fn add_assign(mut a: u8, b: u8) -> u8 { a += b; a }
pub fn sub_assign(mut a: u8, b: u8) -> u8 { a -= b; a }
pub fn mul_assign(mut a: u8, b: u8) -> u8 { a *= b; a }
pub fn div_assign(mut a: u8, b: u8) -> u8 { a /= b; a }
pub fn rem_assign(mut a: u8, b: u8) -> u8 { a %= b; a }
pub fn add_refs(a: &u8, b: &u8) -> u8 { a + b }
pub fn add_generic<T: Add<Output = T>>(a: T, b: T) -> T { a + b }
pub fn nonzero_neg(a: NonZero<i32>) -> NonZero<i32> { -a }
pub fn wrapping_div(a: Wrapping<i32>, b: Wrapping<i32>) -> Wrapping<i32> { a / b }
pub fn saturating_div(a: Saturating<i32>, b: Saturating<i32>) -> Saturating<i32> { a / b }

// ── arithmetic_side_effects: time ──
pub fn duration_add(a: Duration, b: Duration) -> Duration { a + b }
pub fn duration_sub(a: Duration, b: Duration) -> Duration { a - b }
pub fn duration_mul(a: Duration, n: u32) -> Duration { a * n }
pub fn u32_mul_duration(n: u32, a: Duration) -> Duration { n * a }
pub fn duration_div(a: Duration, n: u32) -> Duration { a / n }
pub fn duration_add_assign(mut a: Duration, b: Duration) -> Duration { a += b; a }
pub fn instant_add(i: Instant, d: Duration) -> Instant { i + d }
pub fn instant_sub(i: Instant, d: Duration) -> Instant { i - d }
pub fn systemtime_add(t: SystemTime, d: Duration) -> SystemTime { t + d }
pub fn systemtime_sub(t: SystemTime, d: Duration) -> SystemTime { t - d }
