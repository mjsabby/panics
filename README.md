# Clippy configuration for Rust code that must not panic

Two files that make `cargo clippy` reject code that can panic, and a probe crate that proves
what they catch and what they miss.

| File | What it is |
|---|---|
| [`clippy.toml`](clippy.toml) | A ban list: 545 macros and functions of the standard library that can panic, each with what to use instead. |
| [`Cargo.toml`](Cargo.toml) | The `[workspace.lints.clippy]` table: the five lints that enforce the ban list and cover the operators. |
| [`probe/`](probe) | 638 snippets that panic. Every entry of the ban list has one; so does every gap that is known. |
| [`example/`](example) | A small crate that passes: the usual replacements, and how to exempt tests. |

Checked with Rust 1.100 on Linux. The lists follow the standard library, so they need a
second look after a toolchain update; [three scripts](#keeping-it-current) do the looking.

## Using it

1. Copy `clippy.toml` to the root of your package or workspace.
2. Copy this table into your `Cargo.toml`:

   ```toml
   [lints.clippy]
   disallowed_macros = "deny"
   disallowed_methods = "deny"
   indexing_slicing = "deny"
   string_slice = "deny"
   arithmetic_side_effects = "deny"
   ```

   In a workspace, name the table `[workspace.lints.clippy]` and add `[lints] workspace = true`
   to every member.
3. Run `cargo clippy`.

Each finding says why the call can panic and what to do about it:

```
error: use of a disallowed method `alloc::vec::Vec::remove`
   --> probe/src/bans.rs:143:58
    |
143 | pub fn vec_remove(v: &mut Vec<u32>, i: usize) -> u32 { v.remove(i) }
    |                                                          ^^^^^^
    |
    = note: panics if `index >= len`; check it first
```

### Tests

Tests are supposed to panic, and `disallowed_macros` has no exemption for them. Plain
`cargo clippy` does not compile tests, so nothing is needed for that. If you lint with
`--all-targets`, switch the lints off for test code:

```rust
#[cfg(test)]
#[allow(
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects,
    reason = "a test is supposed to panic when something is wrong"
)]
mod tests { /* ... */ }
```

A file under `tests/` is a crate of its own: put the same list in `#![allow(...)]` at its top.
Both forms are in [`example/`](example).

### When a call cannot panic

Clippy does not look at values. `data.chunks(4)` is flagged like `data.chunks(n)`, and
`x.clamp(0, 100)` like `x.clamp(lo, hi)`. Where a replacement exists that cannot panic, prefer
it (`data.as_chunks::<4>()`). Otherwise say why the call is safe, where it is made:

```rust
#[expect(clippy::disallowed_methods, reason = "the bounds are literals with min <= max")]
pub fn percent(value: u32) -> u32 {
    value.clamp(0, 100)
}
```

`expect` rather than `allow`: if the lint stops firing there, the build tells you. The effect
is that every place that could still panic carries a written reason, and
`grep -rn 'expect(clippy::'` lists them all.

The ones you will meet most often:

- a size or bound that is a literal: `chunks(4)`, `step_by(2)`, `clamp(0, 100)`, `Duration::new(5, 0)`;
- an index that was checked on the line above;
- a compile-time assertion, `const _: () = assert!(...)`, which never runs;
- `sum()` over floats, which cannot overflow (use `fold(0.0, |a, b| a + b)`, or `expect`).

### On existing code

Expect a lot of findings. Run over a handful of published crates (base64, memchr, smallvec,
arrayvec, httparse) as a trial, the configuration reported one finding for every 20 to 75 lines,
mostly indexing, arithmetic and assertions. It is easiest to adopt for new code, or one crate
at a time, with `warn` instead of `deny` while the count comes down.

### What it costs

Almost nothing, with one exception.

- **Linting.** Measured on a 106-line and a 108,000-line crate: about 2 ms per crate to load
  the ban list, and 2 to 4% more time for a `cargo clippy` run.
- **Building.** Nothing. `cargo build` and `cargo test` do not run clippy. Editing the lint
  table in `Cargo.toml` rebuilds the crate once; editing `clippy.toml` rebuilds nothing.
- **Running.** What the lints make you write costs the same as what it replaces: `v.get(i)?`
  does the bounds check that `v[i]` does, and `checked_div` the test for zero that `/` does.
  The exception is adding up integers in a hot loop. `sum()` is banned because it overflows
  like `+`, and the obvious replacement, `try_fold` with `checked_add`, stops at the first
  overflow. A loop that can stop early is not vectorized:

  | Adding up a `&[u32]` | ns per value | On overflow |
  |---|---:|---|
  | `iter().sum()` (banned) | 0.045 | panics with overflow checks on, wraps with them off |
  | `try_fold(0, \|a, v\| a.checked_add(*v))` | 0.181 | returns `None` |
  | `fold(0, \|a, v\| a.wrapping_add(*v))` | 0.045 | wraps |
  | `fold(0_u64, \|a, v\| a.wrapping_add(u64::from(*v)))`, then `u32::try_from` | 0.086 | returns `None` |

  The checked form is four times slower here, and seven times with `-C target-cpu=native`;
  for `u64` values it is about twice as slow. So where a sum is hot, either wrap, if wrapping
  is acceptable (it is what `sum()` does in a release build), or add into a wider type that
  cannot overflow and convert back at the end, which with `-C target-cpu=native` is as fast as
  `sum()`. [`example/examples/reduction.rs`](example/examples/reduction.rs) has the four
  forms, and `cargo run --release --example reduction` times them on your machine.

### Good to know

- **A group-level `allow` switches the lints off without naming them.** `#[allow(clippy::all)]`
  and `#[allow(clippy::style)]` disable the two ban lints; `#[allow(clippy::restriction)]`
  disables the three operator lints. Search for `allow(clippy::` as well as `expect(clippy::`.
- **`forbid` instead of `deny`** turns every `#[allow]` and `#[expect]` of these lints into a
  compile error, the group-level ones included. That closes the hole above and removes the
  escape hatch with it: a call that cannot panic can no longer be annotated, only rewritten,
  and tests cannot be exempted under `--all-targets`.
- **Clippy only sees what it compiles.** Code behind a `cfg` that is off is not linted. Run it
  for every feature set, target and profile you ship: `--all-features`, `--target`, `--release`.
- **`no_std`** works unchanged. Entries for a crate that is not linked (`alloc`, `std`) are ignored.
- **Older toolchains.** Clippy warns about every entry that names something the toolchain does
  not have yet (`... does not refer to a reachable function`). Delete those entries.
- **Some findings are reported twice.** A macro is reported as itself and as what it expands
  to: `debug_assert!` (also as `assert!`), `dbg!` (also as `eprintln!`), and `todo!`,
  `unimplemented!` and `unreachable!` with a message (also as `panic!`). So is
  `a.wrapping_div(b)`, by the ban list and by `arithmetic_side_effects`.
- **Sizes that get allocated** (`Vec::with_capacity`, `resize`, `repeat` and others) are at
  the end of `clippy.toml`, commented out. Enable them if sizes can come from outside.
- **"Overflow checks"** in a reason means: the call panics in debug builds, and in release
  builds only with `overflow-checks = true`. Without them it wraps silently, which is rarely
  what was meant either. 61 of the 638 snippets are of this kind; the rest panic in every build.

## What it catches

| | Examples | How |
|---|---|---|
| Explicit panics | `panic!`, `todo!`, `unreachable!`, `unimplemented!`, `panic_any` | ban list |
| Assertions | `assert!`, `assert_eq!`, `assert_matches!`, `debug_assert!` | ban list |
| Unwrapping | `unwrap`, `expect`, `unwrap_err`, `expect_err` | ban list |
| Printing | `println!`, `eprintln!`, `dbg!` (they panic when the stream is closed: `tool \| head`) | ban list |
| Run-time borrow checks | `RefCell::borrow_mut`, `LocalKey::with`, `OnceCell::get_or_init` | ban list |
| Indexing and slicing | `v[i]`, `&v[a..b]`, `&s[..n]`, `map[&key]` | `indexing_slicing`, `string_slice` |
| Indexes, ranges, lengths | `v.remove(i)`, `s.split_at(n)`, `dst.copy_from_slice(src)`, `v.chunks(0)`, `map.range(b..a)`, `reader.consume(n)` | ban list |
| Integer arithmetic | `a + b`, `a / b`, `-a`, `len - 1` | `arithmetic_side_effects` |
| Integer methods | `pow`, `abs`, `ilog2`, `div_euclid`, `wrapping_div`, `strict_add`, on all twelve integer types | ban list |
| Sums and bounds | `iter.sum()`, `x.clamp(lo, hi)`, `fold(0, u32::add)` | ban list |
| Time | `a + b` on `Duration` and `Instant`, `Duration::from_secs_f64`, `Duration::new` | lint, ban list |
| Radix and encoding | `char::from_digit(d, 37)`, `u8::from_str_radix(s, 1)`, `c.encode_utf8(&mut [0; 1])` | ban list |
| Threads, processes, environment, paths | `thread::spawn`, `Command::output`, `env::args`, `PathBuf::set_extension` | ban list |

Clippy has lints of its own for some of these: `panic`, `todo`, `unimplemented`, `unreachable`,
`unwrap_used`, `expect_used`, `print_stdout`. They are not used here, because the ban list
covers the same ground and they have holes. The macro lints only look at the outermost macro
call, so a `panic!` or `println!` inside your own `bail!` or `log!` macro gets past them.
`unwrap_used` misses an `unwrap` inside a macro, and one passed as a function:
`.map(Option::unwrap)`. The ban list catches all of these.

[docs/COVERAGE.md](docs/COVERAGE.md) has every snippet, with the panic message it produced.

## What it does not catch

This is a net, not a proof. The probe has a snippet that panics and passes clippy for each of
these (except the `cfg` case, which one build cannot show); [docs/COVERAGE.md](docs/COVERAGE.md)
lists all 34.

- **Code that clippy does not see.** Clippy lints your crate, as it is compiled. A function in
  a dependency that indexes or unwraps is invisible, and so is the inside of the standard
  library. So is your own code behind a `cfg` that is off.
- **Holes in `arithmetic_side_effects`.** It ignores shifts (`a << n`). It misreads two
  literals: `x / -1`, `x * -1` and `0 - x` pass, and each overflows for `MIN`. And it skips
  everything inside a `static` or `const` item, including closures that run later, such as the
  initializer of a `static LazyLock`. For division, the two that panic in every build (`x / -1`,
  and `/` or `%` inside a `static` or `const`) can be closed: `integer_division_remainder_used`,
  commented out in `Cargo.toml`, flags every integer division and remainder. It also flags the
  harmless ones by any other literal, which added up to 12% more findings on the trial crates.
- **Holes in `indexing_slicing`.** It flags `map[&key]` on an owned `HashMap`, `BTreeMap` or
  `VecDeque` and on a field, but not through a reference. It gives up on an array whose length
  is a const generic (`&buf[..n]` with `buf: &[u8; N]`). It does not look at a generic
  `T: Index`, at `CStr`, or at an index written as `const { .. }`.
- **Trait implementations**, which `clippy.toml` cannot name. `RefCell`'s `clone()`, `==` and
  `cmp` panic while the value is mutably borrowed. Dereferencing a `LazyCell` panics if its
  initializer reads the cell itself. An open range panics when it steps past the largest
  value: `(200_u8..).nth(100)`.
- **Contracts your own code has to keep.** `sort_by` panics if the comparison is not a total
  order, and `a.partial_cmp(b).unwrap_or(Ordering::Equal)`, the obvious way around the ban on
  `unwrap`, is not one once NaN is involved. Use `f64::total_cmp`. `format!`, `to_string` and
  `write!` panic if a `Display` implementation returns an error. A future panics if it is
  polled again after it has finished.
- **Arguments checked at run time, on functions too common to ban.** A formatting width above
  65535 taken from an argument (`{:width$}`); an atomic `Ordering` chosen at run time; a NUL
  byte in a thread name; `count()` on a range with more than `usize::MAX` items, such as
  `0..=usize::MAX`.
- **Memory.** A size above `isize::MAX` bytes panics; the optional block in `clippy.toml` bans
  the functions that take one, but not `vec![x; n]` or `collect`. Running out of memory
  normally aborts the process instead, as does overflowing the stack, and no lint helps there.
- **Locks.** Locking a `Mutex` or `RwLock` that the same thread already holds may panic or
  deadlock, depending on the platform.

[docs/STD_PANICS.md](docs/STD_PANICS.md) accounts for every panic the standard library
documents: 676 items, of which 501 are banned, 36 are covered by an operator lint, and 139 are
left out, each with the reason.

If a function has to be proven free of panics, a linter is the wrong tool: look at the
`no-panic` crate, which fails the build if the optimizer cannot remove every panic from a
function, or at a model checker such as Kani.

## Why this is believed to be complete

Three checks that can be repeated, each a script in [`tools/`](tools), and one that cannot:

- **`audit.py` reads the documentation.** It extracts every "Panics" section from the
  documentation that ships with the toolchain, and requires each item to be banned, covered by
  an operator lint, or listed in [`tools/accounting.toml`](tools/accounting.toml) with a reason.
  It also flags functions that only link to a banned one, which is how delegating functions
  such as `Path::with_extension` were found.
- **`brute_force.py` does not trust the documentation.** It calls 1521 stable methods of the
  integer and float types, `char` and `bool` with boundary values, and requires every one that
  panics to be accounted for. That is how the overflow in `pow`, which is not documented, was
  found. (54 more methods take a parameter it has no values for, such as a buffer.)
- **`verify.py` runs the probe.** For every entry of `clippy.toml` it requires a snippet that
  panics at run time and that clippy reports as a use of exactly that entry. Every operator
  snippet must be flagged by its lint, and every known gap must still be a gap. It also checks
  that `example` passes, and that a `no_std` crate gets the `core` entries.
- **An adversarial review, done once.** A separate pass wrote several hundred snippets with the
  aim of getting a panic past the configuration. What it found is now banned
  (`BufRead::consume`, `Command::output`, `Child::wait_with_output`, and `wrapping_div` and its
  three relatives, which `arithmetic_side_effects` only sees as method calls) or listed above
  as a hole in one of the lints. Nothing in this repository repeats that review.

What none of them can find is a panic that is undocumented, outside the numeric types, and
not known to the authors.

## Keeping it current

After a toolchain update, and after any change to `clippy.toml`:

```sh
rustup component add rust-docs    # once; audit.py reads the installed documentation
tools/audit.py                    # what does the standard library document now?
tools/brute_force.py              # what do the numeric methods really do? (about 20 seconds)
tools/verify.py                   # does the probe still agree with clippy?
```

The first two print what is not accounted for, with a line ready to paste into `clippy.toml`;
the third says which entry lacks a snippet in the probe. When all three pass, regenerate the
two reports with `tools/audit.py --update` and `tools/verify.py --update`. The scripts need
Python 3.11 or later and nothing else. The probe's harness needs a Unix system for a few
snippets (it uses `sh`, `head` and `/dev/zero`), and has only been run on Linux.

## Layout

```
clippy.toml            the ban list
Cargo.toml             the lint table, and the workspace
probe/                 snippets that panic (this crate fails clippy on purpose)
  src/bans.rs            one for every entry of clippy.toml
  src/ints.rs            the same for the integer methods
  src/lints.rs           operators
  src/gaps.rs            what nothing catches
  src/main.rs            the harness that runs them
probe-dep/             stands in for a third-party dependency
example/               code that passes
docs/COVERAGE.md       generated: what the probe showed
docs/STD_PANICS.md     generated: every documented panic and what happens to it
tools/                 audit.py, brute_force.py, verify.py, and their data
```

`cargo clippy` and `cargo test` at the root only look at `example`, and pass. The probe is
linted with `cargo clippy -p probe` (660 errors, as intended) and run with `cargo run -p probe`.
