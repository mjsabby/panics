//! The integer methods in `clippy.toml`. Each exists once per integer type, so the snippets
//! are generated: `On::<u8>::pow` is the snippet for the entry `u8::pow`.

use std::marker::PhantomData;

/// Namespace for the snippets of one integer type.
pub struct On<T>(PhantomData<T>);

macro_rules! any_int { ($($t:ty)*) => { $( impl On<$t> {
    pub fn from_str_radix(s: &str, radix: u32) -> bool { <$t>::from_str_radix(s, radix).is_ok() }
    pub fn pow(a: $t, exp: u32) -> $t { a.pow(exp) }
    pub fn div_euclid(a: $t, b: $t) -> $t { a.div_euclid(b) }
    pub fn rem_euclid(a: $t, b: $t) -> $t { a.rem_euclid(b) }
    pub fn wrapping_div_euclid(a: $t, b: $t) -> $t { a.wrapping_div_euclid(b) }
    pub fn wrapping_div(a: $t, b: $t) -> $t { a.wrapping_div(b) }
    pub fn wrapping_rem(a: $t, b: $t) -> $t { a.wrapping_rem(b) }
    pub fn wrapping_rem_euclid(a: $t, b: $t) -> $t { a.wrapping_rem_euclid(b) }
    pub fn saturating_div(a: $t, b: $t) -> $t { a.saturating_div(b) }
    pub fn overflowing_div(a: $t, b: $t) -> ($t, bool) { a.overflowing_div(b) }
    pub fn overflowing_div_euclid(a: $t, b: $t) -> ($t, bool) { a.overflowing_div_euclid(b) }
    pub fn overflowing_rem(a: $t, b: $t) -> ($t, bool) { a.overflowing_rem(b) }
    pub fn overflowing_rem_euclid(a: $t, b: $t) -> ($t, bool) { a.overflowing_rem_euclid(b) }
    pub fn ilog(a: $t, base: $t) -> u32 { a.ilog(base) }
    pub fn ilog2(a: $t) -> u32 { a.ilog2() }
    pub fn ilog10(a: $t) -> u32 { a.ilog10() }
    pub fn strict_add(a: $t, b: $t) -> $t { a.strict_add(b) }
    pub fn strict_sub(a: $t, b: $t) -> $t { a.strict_sub(b) }
    pub fn strict_mul(a: $t, b: $t) -> $t { a.strict_mul(b) }
    pub fn strict_div(a: $t, b: $t) -> $t { a.strict_div(b) }
    pub fn strict_div_euclid(a: $t, b: $t) -> $t { a.strict_div_euclid(b) }
    pub fn strict_rem(a: $t, b: $t) -> $t { a.strict_rem(b) }
    pub fn strict_rem_euclid(a: $t, b: $t) -> $t { a.strict_rem_euclid(b) }
    pub fn strict_neg(a: $t) -> $t { a.strict_neg() }
    pub fn strict_pow(a: $t, exp: u32) -> $t { a.strict_pow(exp) }
    pub fn strict_shl(a: $t, n: u32) -> $t { a.strict_shl(n) }
    pub fn strict_shr(a: $t, n: u32) -> $t { a.strict_shr(n) }
} )* }; }

macro_rules! signed_int { ($($t:ty, $unsigned:ty);*) => { $( impl On<$t> {
    pub fn abs(a: $t) -> $t { a.abs() }
    pub fn isqrt(a: $t) -> $t { a.isqrt() }
    pub fn strict_abs(a: $t) -> $t { a.strict_abs() }
    pub fn strict_add_unsigned(a: $t, b: $unsigned) -> $t { a.strict_add_unsigned(b) }
    pub fn strict_sub_unsigned(a: $t, b: $unsigned) -> $t { a.strict_sub_unsigned(b) }
} )* }; }

macro_rules! unsigned_int { ($($t:ty, $signed:ty);*) => { $( impl On<$t> {
    pub fn next_power_of_two(a: $t) -> $t { a.next_power_of_two() }
    pub fn next_multiple_of(a: $t, b: $t) -> $t { a.next_multiple_of(b) }
    pub fn div_ceil(a: $t, b: $t) -> $t { a.div_ceil(b) }
    pub fn strict_add_signed(a: $t, b: $signed) -> $t { a.strict_add_signed(b) }
    pub fn strict_sub_signed(a: $t, b: $signed) -> $t { a.strict_sub_signed(b) }
} )* }; }

any_int!(i8 i16 i32 i64 i128 isize u8 u16 u32 u64 u128 usize);
signed_int!(i8, u8; i16, u16; i32, u32; i64, u64; i128, u128; isize, usize);
unsigned_int!(u8, i8; u16, i16; u32, i32; u64, i64; u128, i128; usize, isize);
