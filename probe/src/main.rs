//! Harness of the probe: calls every snippet with an input that makes it panic, and prints
//! one tab-separated line per call:
//!
//!     kind   id   function   outcome   panic message
//!
//! * `kind` is `ban` (id: the entry in clippy.toml), `lint` (id: the lint) or `gap` (no id).
//! * `outcome` is `panic` or `ok`.
//!
//! `tools/verify.py` runs this in a debug and in a release build and compares the output with
//! clippy's diagnostics for the snippets. A few snippets only panic in a particular process
//! state (a closed stdout, say); for those the harness starts itself again as a child process.

#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects,
    reason = "the harness is test code; it is the snippets in the library that are under test"
)]

use probe::ints::On;
use probe::{bans, gaps, lints};
use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap, LinkedList, VecDeque};
use std::ffi::OsString;
use std::fmt;
use std::hint::black_box as bb;
use std::io::{self, IoSlice, IoSliceMut, Read};
use std::mem::MaybeUninit;
use std::num::{NonZero, Saturating, Wrapping};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

// ── Reporting ────────────────────────────────────────────────────────────────────────────────

static LAST_PANIC: Mutex<Option<String>> = Mutex::new(None);

fn install_panic_hook() {
    panic::set_hook(Box::new(|info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| String::from("(payload is not a string)"));
        *LAST_PANIC.lock().unwrap_or_else(|e| e.into_inner()) = Some(message);
    }));
}

fn take_panic_message() -> String {
    LAST_PANIC
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .unwrap_or_default()
}

fn report(kind: &str, id: &str, function: &str, panicked: bool, message: &str) {
    let function = function.replace(' ', "");
    let outcome = if panicked { "panic" } else { "ok" };
    let message: String = message
        .lines()
        .next()
        .unwrap_or("")
        .replace('\t', " ")
        .chars()
        .take(140)
        .collect();
    println!("{kind}\t{id}\t{function}\t{outcome}\t{message}");
}

/// Runs a snippet in this process.
fn run(kind: &str, id: &str, function: &str, snippet: impl FnOnce()) {
    let outcome = panic::catch_unwind(AssertUnwindSafe(snippet));
    report(kind, id, function, outcome.is_err(), &take_panic_message());
}

macro_rules! ban {
    ($id:literal => $($f:ident)::+ ( $($arg:expr),* $(,)? )) => {
        run("ban", $id, stringify!($($f)::+), || { let _ = bb($($f)::+($($arg),*)); })
    };
}
macro_rules! lint {
    ($id:literal => $($f:ident)::+ ( $($arg:expr),* $(,)? )) => {
        run("lint", $id, stringify!($($f)::+), || { let _ = bb($($f)::+($($arg),*)); })
    };
}
macro_rules! gap {
    ($($f:ident)::+ ( $($arg:expr),* $(,)? )) => {
        run("gap", "", stringify!($($f)::+), || { let _ = bb($($f)::+($($arg),*)); })
    };
}

// ── Snippets that need a child process ───────────────────────────────────────────────────────

/// What has to be true of the process for the snippet to panic.
#[derive(Clone, Copy)]
enum Needs {
    ClosedStdout,
    ClosedStderr,
    NonUnicodeArgument,
    NonUnicodeVariable,
    /// `RUST_MIN_STACK` so large that the OS refuses to create a thread.
    ThreadCreationFails,
    /// An address-space limit below what the snippet tries to buffer.
    TooLittleMemory,
}

#[cfg(unix)]
fn non_unicode() -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(vec![0xff])
}
#[cfg(windows)]
fn non_unicode() -> OsString {
    use std::os::windows::ffi::OsStringExt;
    OsString::from_wide(&[0xd800])
}

/// Runs a snippet in a child process and reports whether that process died of a panic.
fn run_in_child(id: &str, function: &str, needs: Needs) {
    let function = function.replace(' ', "");
    let harness = std::env::current_exe().expect("path of the harness");
    let mut command = if matches!(needs, Needs::TooLittleMemory) {
        // The shell sets the limit and then becomes the harness.
        let mut shell = Command::new("sh");
        shell
            .arg("-c")
            .arg("ulimit -v 400000 && exec \"$0\" \"$@\"")
            .arg(&harness);
        shell
    } else {
        Command::new(&harness)
    };
    command.arg("--child").arg(&function);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    match needs {
        Needs::NonUnicodeArgument => {
            command.arg(non_unicode());
        }
        Needs::NonUnicodeVariable => {
            command.env("PROBE_NON_UNICODE", non_unicode());
        }
        Needs::ThreadCreationFails => {
            command.env("RUST_MIN_STACK", (usize::MAX / 4).to_string());
        }
        Needs::ClosedStdout | Needs::ClosedStderr | Needs::TooLittleMemory => {}
    }
    let mut child = command.spawn().expect("start the child process");
    // The child waits for its stdin to close, so the stream is certain to be closed before it writes.
    match needs {
        Needs::ClosedStdout => drop(child.stdout.take()),
        Needs::ClosedStderr => drop(child.stderr.take()),
        _ => {}
    }
    drop(child.stdin.take());
    let output = child
        .wait_with_output()
        .expect("wait for the child process");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let message = stderr
        .lines()
        .skip_while(|line| !line.contains("panicked at"))
        .nth(1)
        .unwrap_or("");
    report(
        "ban",
        id,
        &function,
        output.status.code() == Some(101),
        message,
    );
}

macro_rules! child {
    ($id:literal => $($f:ident)::+, $needs:ident) => {
        run_in_child($id, stringify!($($f)::+), Needs::$needs)
    };
}

/// The other side of `run_in_child`.
fn child_main(function: &str) {
    let _ = io::stdin().read_to_end(&mut Vec::new());
    match function {
        "bans::print" => bans::print(bb(1)),
        "bans::println" => bans::println(bb(1)),
        "bans::println_in_local_macro" => bans::println_in_local_macro(bb(1)),
        "bans::eprint" => bans::eprint(bb(1)),
        "bans::eprintln" => bans::eprintln(bb(1)),
        "bans::dbg" => drop(bb(bans::dbg(bb(1)))),
        "bans::env_args" => drop(bb(bans::env_args())),
        "bans::env_vars" => drop(bb(bans::env_vars())),
        "bans::thread_spawn" => drop(bb(bans::thread_spawn())),
        "bans::scope_spawn" => drop(bb(bans::scope_spawn())),
        "bans::command_output" => {
            // One gigabyte of output, of which only stdout is captured.
            let mut command = Command::new("head");
            command
                .args(["-c", "1000000000", "/dev/zero"])
                .stderr(Stdio::null());
            let _ = bb(bans::command_output(&mut command));
        }
        "exit" => {}
        other => {
            eprintln!("unknown child snippet {other}");
            process::exit(2);
        }
    }
}

/// A child process whose stdout handle is a directory, so that reading from it fails.
#[cfg(unix)]
fn child_with_unreadable_stdout() -> process::Child {
    use std::os::fd::OwnedFd;
    let mut command = Command::new(std::env::current_exe().expect("path of the harness"));
    command
        .arg("--child")
        .arg("exit")
        .stdin(Stdio::null())
        .stdout(Stdio::piped());
    let mut child = command.spawn().expect("start the child process");
    child.stdout = Some(OwnedFd::from(std::fs::File::open("/").expect("open a directory")).into());
    child
}

// ── Snippets that need a destroyed thread-local ──────────────────────────────────────────────

struct RunAtExit(RefCell<Option<Box<dyn FnOnce()>>>);
impl Drop for RunAtExit {
    fn drop(&mut self) {
        if let Some(action) = self.0.get_mut().take() {
            action();
        }
    }
}
thread_local! { static AT_EXIT: RunAtExit = const { RunAtExit(RefCell::new(None)) }; }

/// Runs a snippet on a thread that is exiting, after the thread-locals of `bans` were destroyed.
/// Thread-locals are destroyed in the reverse order of their first use, so `AT_EXIT`, used
/// first, goes last, and its destructor runs the snippet.
fn run_after_tls_destruction(id: &str, function: &str, snippet: fn()) {
    let (sender, receiver) = mpsc::channel();
    let worker = thread::Builder::new().spawn(move || {
        AT_EXIT.with(|at_exit| {
            *at_exit.0.borrow_mut() = Some(Box::new(move || {
                let _ = sender.send(panic::catch_unwind(snippet).is_err());
            }));
        });
        bans::tls_register();
    });
    let _ = worker.expect("start a thread").join();
    report(
        "ban",
        id,
        function,
        receiver.recv().unwrap_or(false),
        &take_panic_message(),
    );
}

macro_rules! tls {
    ($id:literal => $($f:ident)::+) => {
        run_after_tls_destruction($id, stringify!($($f)::+), || { let _ = bb($($f)::+()); })
    };
}

// ── Helpers for inputs ───────────────────────────────────────────────────────────────────────

/// Lets a snippet that takes a `Formatter` be called through `to_string`.
struct Formatted(fn(&mut fmt::Formatter<'_>) -> fmt::Result);
impl fmt::Display for Formatted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (self.0)(f)
    }
}

/// Float slices of various lengths in which roughly every fourth value is NaN.
fn slices_with_nan() -> Vec<Vec<f64>> {
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut slices = Vec::new();
    for len in [32, 64, 128, 256, 1024] {
        for _ in 0..50 {
            slices.push(
                (0..len)
                    .map(|_| {
                        seed = seed
                            .wrapping_mul(6_364_136_223_846_793_005)
                            .wrapping_add(1_442_695_040_888_963_407);
                        let r = (seed >> 33) as u32;
                        if r % 4 == 0 {
                            f64::NAN
                        } else {
                            f64::from(r % 1000)
                        }
                    })
                    .collect(),
            );
        }
    }
    slices
}

// ── The snippets ─────────────────────────────────────────────────────────────────────────────

fn main() {
    let mut args = std::env::args_os().skip(1);
    if args.next().is_some_and(|arg| arg == "--child") {
        let function = args
            .next()
            .and_then(|f| f.into_string().ok())
            .unwrap_or_default();
        return child_main(&function);
    }
    install_panic_hook();
    run_bans();
    run_int_bans();
    run_lints();
    run_gaps();
}

#[rustfmt::skip]
fn run_bans() {
    // disallowed-macros: explicit panics
    ban!("core::panic" => bans::core_panic());
    ban!("std::panic" => bans::std_panic());
    ban!("core::unreachable" => bans::unreachable());
    ban!("core::todo" => bans::todo());
    ban!("core::unimplemented" => bans::unimplemented());
    ban!("std::panic" => bans::panic_in_local_macro());
    ban!("std::panic" => bans::panic_in_dependency_macro());
    ban!("std::panic" => bans::panic_in_const_fn(bb(0)));
    // assertions
    ban!("core::assert" => bans::assert(bb(1)));
    ban!("core::assert_eq" => bans::assert_eq(bb(1)));
    ban!("core::assert_ne" => bans::assert_ne(bb(1)));
    ban!("core::assert_matches" => bans::assert_matches(bb(None)));
    ban!("core::debug_assert" => bans::debug_assert(bb(1)));
    ban!("core::debug_assert_eq" => bans::debug_assert_eq(bb(1)));
    ban!("core::debug_assert_ne" => bans::debug_assert_ne(bb(1)));
    ban!("core::debug_assert_matches" => bans::debug_assert_matches(bb(None)));
    ban!("core::assert" => bans::assert_in_dependency_macro(bb(1)));
    // printing
    child!("std::print" => bans::print, ClosedStdout);
    child!("std::println" => bans::println, ClosedStdout);
    child!("std::eprint" => bans::eprint, ClosedStderr);
    child!("std::eprintln" => bans::eprintln, ClosedStderr);
    child!("std::dbg" => bans::dbg, ClosedStderr);
    child!("std::println" => bans::println_in_local_macro, ClosedStdout);

    // disallowed-methods: unwrapping
    ban!("core::option::Option::unwrap" => bans::option_unwrap(bb(None)));
    ban!("core::option::Option::expect" => bans::option_expect(bb(None)));
    ban!("core::result::Result::unwrap" => bans::result_unwrap(bb(Err(1))));
    ban!("core::result::Result::expect" => bans::result_expect(bb(Err(1))));
    ban!("core::result::Result::unwrap_err" => bans::result_unwrap_err(bb(Ok(1))));
    ban!("core::result::Result::expect_err" => bans::result_expect_err(bb(Ok(1))));
    ban!("core::option::Option::unwrap" => bans::option_unwrap_as_path(bb(vec![None])));
    ban!("core::option::Option::unwrap" => bans::option_unwrap_in_local_macro(bb(None)));
    ban!("core::option::Option::unwrap" => bans::option_unwrap_in_dependency_macro(bb(None)));
    // explicit panics
    ban!("std::panic::panic_any" => bans::panic_any());
    ban!("std::panic::resume_unwind" => bans::resume_unwind());
    // run-time borrow and initialization checks
    ban!("core::cell::RefCell::borrow" => bans::refcell_borrow(&RefCell::new(0)));
    ban!("core::cell::RefCell::borrow_mut" => bans::refcell_borrow_mut(&RefCell::new(0)));
    ban!("core::cell::RefCell::replace" => bans::refcell_replace(&RefCell::new(0)));
    ban!("core::cell::RefCell::replace_with" => bans::refcell_replace_with(&RefCell::new(0)));
    ban!("core::cell::RefCell::take" => bans::refcell_take(&RefCell::new(0)));
    ban!("core::cell::RefCell::swap" => bans::refcell_swap(&RefCell::new(0), &RefCell::new(1)));
    ban!("core::cell::OnceCell::get_or_init" => bans::oncecell_get_or_init(&OnceCell::new()));
    tls!("std::thread::LocalKey::with" => bans::localkey_with);
    tls!("std::thread::LocalKey::set" => bans::localkey_set);
    tls!("std::thread::LocalKey::take" => bans::localkey_take);
    tls!("std::thread::LocalKey::replace" => bans::localkey_replace);
    ban!("std::thread::LocalKey::set" => bans::localkey_refcell_set());
    ban!("std::thread::LocalKey::take" => bans::localkey_refcell_take());
    ban!("std::thread::LocalKey::replace" => bans::localkey_refcell_replace());
    ban!("std::thread::LocalKey::with_borrow" => bans::localkey_with_borrow());
    ban!("std::thread::LocalKey::with_borrow_mut" => bans::localkey_with_borrow_mut());
    // indexes, ranges and lengths: slices and str
    ban!("core::ops::Index::index" => bans::index_index(&[1, 2, 3], bb(7)));
    ban!("core::ops::IndexMut::index_mut" => bans::indexmut_index_mut(&mut [1, 2, 3], bb(7)));
    ban!("slice::split_at" => bans::slice_split_at(&[1], bb(7)));
    ban!("slice::split_at_mut" => bans::slice_split_at_mut(&mut [1], bb(7)));
    ban!("slice::swap" => bans::slice_swap(&mut [1], 0, bb(7)));
    ban!("slice::copy_from_slice" => bans::slice_copy_from_slice(&mut [0; 2], bb(&[1; 3])));
    ban!("slice::clone_from_slice" => bans::slice_clone_from_slice(&mut [0; 2], bb(&[1; 3])));
    ban!("slice::swap_with_slice" => bans::slice_swap_with_slice(&mut [0; 2], bb(&mut [1; 3])));
    ban!("slice::write_copy_of_slice" => bans::slice_write_copy_of_slice(&mut [MaybeUninit::uninit(); 2], bb(&[1; 3])));
    ban!("slice::write_clone_of_slice" => bans::slice_write_clone_of_slice(&mut [MaybeUninit::uninit(); 2], bb(&[1; 3])));
    ban!("slice::copy_within" => bans::slice_copy_within(&mut [1], bb(7)));
    ban!("slice::chunks" => bans::slice_chunks(&[1], bb(0)));
    ban!("slice::chunks_mut" => bans::slice_chunks_mut(&mut [1], bb(0)));
    ban!("slice::chunks_exact" => bans::slice_chunks_exact(&[1], bb(0)));
    ban!("slice::chunks_exact_mut" => bans::slice_chunks_exact_mut(&mut [1], bb(0)));
    ban!("slice::rchunks" => bans::slice_rchunks(&[1], bb(0)));
    ban!("slice::rchunks_mut" => bans::slice_rchunks_mut(&mut [1], bb(0)));
    ban!("slice::rchunks_exact" => bans::slice_rchunks_exact(&[1], bb(0)));
    ban!("slice::rchunks_exact_mut" => bans::slice_rchunks_exact_mut(&mut [1], bb(0)));
    ban!("slice::windows" => bans::slice_windows(&[1], bb(0)));
    ban!("slice::rotate_left" => bans::slice_rotate_left(&mut [1], bb(7)));
    ban!("slice::rotate_right" => bans::slice_rotate_right(&mut [1], bb(7)));
    ban!("slice::select_nth_unstable" => bans::slice_select_nth_unstable(&mut [1], bb(7)));
    ban!("slice::select_nth_unstable_by" => bans::slice_select_nth_unstable_by(&mut [1], bb(7)));
    ban!("slice::select_nth_unstable_by_key" => bans::slice_select_nth_unstable_by_key(&mut [1], bb(7)));
    ban!("core::iter::Iterator::step_by" => bans::iterator_step_by(bb(0)));
    ban!("str::split_at" => bans::str_split_at("héllo", bb(2)));
    ban!("str::split_at_mut" => bans::str_split_at_mut(&mut String::from("héllo"), bb(2)));
    // indexes, ranges and lengths: collections
    ban!("alloc::vec::Vec::remove" => bans::vec_remove(&mut vec![1], bb(7)));
    ban!("alloc::vec::Vec::swap_remove" => bans::vec_swap_remove(&mut vec![1], bb(7)));
    ban!("alloc::vec::Vec::insert" => bans::vec_insert(&mut vec![1], bb(7)));
    ban!("alloc::vec::Vec::insert_mut" => bans::vec_insert_mut(&mut vec![1], bb(7)));
    ban!("alloc::vec::Vec::drain" => bans::vec_drain(&mut vec![1], bb(7)));
    ban!("alloc::vec::Vec::splice" => bans::vec_splice(&mut vec![1], bb(7)));
    ban!("alloc::vec::Vec::extend_from_within" => bans::vec_extend_from_within(&mut vec![1], bb(7)));
    ban!("alloc::vec::Vec::extract_if" => bans::vec_extract_if(&mut vec![1], bb(7)));
    ban!("alloc::vec::Vec::split_off" => bans::vec_split_off(&mut vec![1], bb(7)));
    ban!("alloc::string::String::insert" => bans::string_insert(&mut String::from("héllo"), bb(2)));
    ban!("alloc::string::String::insert_str" => bans::string_insert_str(&mut String::from("héllo"), bb(2)));
    ban!("alloc::string::String::remove" => bans::string_remove(&mut String::from("héllo"), bb(2)));
    ban!("alloc::string::String::truncate" => bans::string_truncate(&mut String::from("héllo"), bb(2)));
    ban!("alloc::string::String::split_off" => bans::string_split_off(&mut String::from("héllo"), bb(2)));
    ban!("alloc::string::String::drain" => bans::string_drain(&mut String::from("héllo"), bb(2)));
    ban!("alloc::string::String::replace_range" => bans::string_replace_range(&mut String::from("héllo"), bb(2)));
    ban!("alloc::string::String::extend_from_within" => bans::string_extend_from_within(&mut String::from("héllo"), bb(2)));
    ban!("alloc::collections::vec_deque::VecDeque::insert" => bans::vecdeque_insert(&mut VecDeque::from([1]), bb(7)));
    ban!("alloc::collections::vec_deque::VecDeque::insert_mut" => bans::vecdeque_insert_mut(&mut VecDeque::from([1]), bb(7)));
    ban!("alloc::collections::vec_deque::VecDeque::swap" => bans::vecdeque_swap(&mut VecDeque::from([1]), 0, bb(7)));
    ban!("alloc::collections::vec_deque::VecDeque::drain" => bans::vecdeque_drain(&mut VecDeque::from([1]), bb(7)));
    ban!("alloc::collections::vec_deque::VecDeque::range" => bans::vecdeque_range(&VecDeque::from([1]), bb(7)));
    ban!("alloc::collections::vec_deque::VecDeque::range_mut" => bans::vecdeque_range_mut(&mut VecDeque::from([1]), bb(7)));
    ban!("alloc::collections::vec_deque::VecDeque::split_off" => bans::vecdeque_split_off(&mut VecDeque::from([1]), bb(7)));
    ban!("alloc::collections::vec_deque::VecDeque::rotate_left" => bans::vecdeque_rotate_left(&mut VecDeque::from([1]), bb(7)));
    ban!("alloc::collections::vec_deque::VecDeque::rotate_right" => bans::vecdeque_rotate_right(&mut VecDeque::from([1]), bb(7)));
    ban!("alloc::collections::linked_list::LinkedList::split_off" => bans::linkedlist_split_off(&mut LinkedList::from([1]), bb(7)));
    ban!("alloc::collections::btree_map::BTreeMap::range" => bans::btreemap_range(&BTreeMap::from([(1, 1)]), bb(5), bb(3)));
    ban!("alloc::collections::btree_map::BTreeMap::range_mut" => bans::btreemap_range_mut(&mut BTreeMap::from([(1, 1)]), bb(5), bb(3)));
    ban!("alloc::collections::btree_set::BTreeSet::range" => bans::btreeset_range(&BTreeSet::from([1]), bb(5), bb(3)));
    ban!("std::collections::HashMap::get_disjoint_mut" => bans::hashmap_get_disjoint_mut(&mut HashMap::from([(1, 1)]), bb(1), bb(1)));
    ban!("std::io::IoSlice::advance" => bans::ioslice_advance(&mut IoSlice::new(&[1]), bb(7)));
    ban!("std::io::IoSlice::advance_slices" => bans::ioslice_advance_slices(&mut &mut [IoSlice::new(&[1])][..], bb(7)));
    ban!("std::io::IoSliceMut::advance" => bans::ioslicemut_advance(&mut IoSliceMut::new(&mut [1]), bb(7)));
    ban!("std::io::IoSliceMut::advance_slices" => bans::ioslicemut_advance_slices(&mut &mut [IoSliceMut::new(&mut [1])][..], bb(7)));
    ban!("std::io::BufRead::consume" => bans::bufread_consume(b"abc", bb(7)));
    // radix and encoding
    ban!("char::from_digit" => bans::char_from_digit(bb(99)));
    ban!("core::char::from_digit" => bans::core_char_from_digit(bb(99)));
    ban!("char::to_digit" => bans::char_to_digit('1', bb(99)));
    ban!("char::is_digit" => bans::char_is_digit('1', bb(99)));
    ban!("char::encode_utf8" => bans::char_encode_utf8('é', bb(&mut [0; 1])));
    ban!("char::encode_utf16" => bans::char_encode_utf16('𝄞', bb(&mut [0; 1])));
    ban!("core::num::NonZero::from_str_radix" => bans::nonzero_from_str_radix("1", bb(99)));
    // arithmetic
    ban!("core::ops::Add::add" => bans::ops_add(bb(255), 1));
    ban!("core::ops::Sub::sub" => bans::ops_sub(bb(0), 1));
    ban!("core::ops::Mul::mul" => bans::ops_mul(bb(255), 2));
    ban!("core::ops::Div::div" => bans::ops_div(1, bb(0)));
    ban!("core::ops::Rem::rem" => bans::ops_rem(1, bb(0)));
    ban!("core::ops::Neg::neg" => bans::ops_neg(bb(i8::MIN)));
    ban!("core::ops::Shl::shl" => bans::ops_shl(1, bb(8)));
    ban!("core::ops::Shr::shr" => bans::ops_shr(1, bb(8)));
    ban!("core::ops::AddAssign::add_assign" => bans::ops_add_assign(&mut 255, bb(1)));
    ban!("core::ops::SubAssign::sub_assign" => bans::ops_sub_assign(&mut 0, bb(1)));
    ban!("core::ops::MulAssign::mul_assign" => bans::ops_mul_assign(&mut 255, bb(2)));
    ban!("core::ops::DivAssign::div_assign" => bans::ops_div_assign(&mut 1, bb(0)));
    ban!("core::ops::RemAssign::rem_assign" => bans::ops_rem_assign(&mut 1, bb(0)));
    ban!("core::ops::ShlAssign::shl_assign" => bans::ops_shl_assign(&mut 1, bb(8)));
    ban!("core::ops::ShrAssign::shr_assign" => bans::ops_shr_assign(&mut 1, bb(8)));
    ban!("core::ops::Add::add" => bans::ops_add_as_path(bb(&[200, 100])));
    ban!("core::iter::Iterator::sum" => bans::iterator_sum(bb(&[200, 100])));
    ban!("core::iter::Iterator::product" => bans::iterator_product(bb(&[200, 100])));
    ban!("core::iter::Iterator::sum" => bans::iterator_sum_of_durations(bb(&[Duration::MAX, Duration::MAX])));
    ban!("core::iter::Sum::sum" => bans::sum_sum(bb(vec![200, 100])));
    ban!("core::iter::Product::product" => bans::product_product(bb(vec![200, 100])));
    ban!("core::cmp::Ord::clamp" => bans::ord_clamp(1, bb(5), bb(3)));
    ban!("f32::clamp" => bans::f32_clamp(1.0, bb(5.0), bb(3.0)));
    ban!("f64::clamp" => bans::f64_clamp(1.0, bb(5.0), bb(3.0)));
    ban!("core::num::NonZero::abs" => bans::nonzero_abs(bb(NonZero::<i32>::MIN)));
    // time
    ban!("core::time::Duration::new" => bans::duration_new(bb(u64::MAX), bb(1_000_000_000)));
    ban!("core::time::Duration::from_secs_f32" => bans::duration_from_secs_f32(bb(-1.0)));
    ban!("core::time::Duration::from_secs_f64" => bans::duration_from_secs_f64(bb(-1.0)));
    ban!("core::time::Duration::mul_f32" => bans::duration_mul_f32(Duration::from_secs(1), bb(-1.0)));
    ban!("core::time::Duration::mul_f64" => bans::duration_mul_f64(Duration::from_secs(1), bb(-1.0)));
    ban!("core::time::Duration::div_f32" => bans::duration_div_f32(Duration::from_secs(1), bb(0.0)));
    ban!("core::time::Duration::div_f64" => bans::duration_div_f64(Duration::from_secs(1), bb(0.0)));
    ban!("core::time::Duration::from_mins" => bans::duration_from_mins(bb(u64::MAX)));
    ban!("core::time::Duration::from_hours" => bans::duration_from_hours(bb(u64::MAX)));
    ban!("core::time::Duration::from_nanos_u128" => bans::duration_from_nanos_u128(bb(u128::MAX)));
    // threads, processes, environment and paths
    child!("std::thread::spawn" => bans::thread_spawn, ThreadCreationFails);
    child!("std::thread::Scope::spawn" => bans::scope_spawn, ThreadCreationFails);
    child!("std::process::Command::output" => bans::command_output, TooLittleMemory);
    #[cfg(unix)]
    run("ban", "std::process::Child::wait_with_output", "bans::child_wait_with_output", || { let _ = bb(bans::child_wait_with_output(child_with_unreadable_stdout())); });
    child!("std::env::args" => bans::env_args, NonUnicodeArgument);
    child!("std::env::vars" => bans::env_vars, NonUnicodeVariable);
    ban!("std::env::set_var" => bans::env_set_var(bb("")));
    ban!("std::env::remove_var" => bans::env_remove_var(bb("")));
    ban!("std::path::PathBuf::set_extension" => bans::pathbuf_set_extension(&mut PathBuf::from("a"), bb("b/c")));
    ban!("std::path::PathBuf::add_extension" => bans::pathbuf_add_extension(&mut PathBuf::from("a"), bb("b/c")));
    ban!("std::path::Path::with_extension" => bans::path_with_extension(Path::new("a"), bb("b/c")));
    ban!("std::path::Path::with_added_extension" => bans::path_with_added_extension(Path::new("a"), bb("b/c")));
    // other
    run("ban", "core::fmt::DebugMap::key", "bans::debugmap_key", || { let _ = bb(Formatted(bans::debugmap_key).to_string()); });
    run("ban", "core::fmt::DebugMap::value", "bans::debugmap_value", || { let _ = bb(Formatted(bans::debugmap_value).to_string()); });
    ban!("core::future::Ready::into_inner" => bans::ready_into_inner());
    ban!("const_ptr::align_offset" => bans::const_ptr_align_offset(&0, bb(3)));
    ban!("mut_ptr::align_offset" => bans::mut_ptr_align_offset(&mut 0, bb(3)));
    ban!("core::ptr::NonNull::align_offset" => bans::nonnull_align_offset(NonNull::from(&mut 0), bb(3)));
}

/// `int_ban!(u8, pow(u8::MAX, 2))` runs `On::<u8>::pow` as the snippet for the entry `u8::pow`.
macro_rules! int_ban {
    ($t:ident, $method:ident ( $($arg:expr),* )) => {
        run(
            "ban",
            concat!(stringify!($t), "::", stringify!($method)),
            concat!("ints::On::<", stringify!($t), ">::", stringify!($method)),
            || { let _ = bb(On::<$t>::$method($(bb($arg)),*)); },
        )
    };
}

#[rustfmt::skip]
fn run_int_bans() {
    macro_rules! any_int { ($($t:ident)*) => { $(
        int_ban!($t, from_str_radix("1", 99));
        int_ban!($t, pow(<$t>::MAX, 2));
        int_ban!($t, div_euclid(1, 0));
        int_ban!($t, rem_euclid(1, 0));
        int_ban!($t, wrapping_div_euclid(1, 0));
        int_ban!($t, wrapping_div(1, 0));
        int_ban!($t, wrapping_rem(1, 0));
        int_ban!($t, wrapping_rem_euclid(1, 0));
        int_ban!($t, saturating_div(1, 0));
        int_ban!($t, overflowing_div(1, 0));
        int_ban!($t, overflowing_div_euclid(1, 0));
        int_ban!($t, overflowing_rem(1, 0));
        int_ban!($t, overflowing_rem_euclid(1, 0));
        int_ban!($t, ilog(0, 2));
        int_ban!($t, ilog2(0));
        int_ban!($t, ilog10(0));
        int_ban!($t, strict_add(<$t>::MAX, 1));
        int_ban!($t, strict_sub(<$t>::MIN, 1));
        int_ban!($t, strict_mul(<$t>::MAX, 2));
        int_ban!($t, strict_div(1, 0));
        int_ban!($t, strict_div_euclid(1, 0));
        int_ban!($t, strict_rem(1, 0));
        int_ban!($t, strict_rem_euclid(1, 0));
        int_ban!($t, strict_pow(<$t>::MAX, 2));
        int_ban!($t, strict_shl(1, <$t>::BITS));
        int_ban!($t, strict_shr(1, <$t>::BITS));
    )* }; }
    macro_rules! signed_int { ($($t:ident)*) => { $(
        int_ban!($t, abs(<$t>::MIN));
        int_ban!($t, isqrt(-1));
        int_ban!($t, strict_neg(<$t>::MIN));
        int_ban!($t, strict_abs(<$t>::MIN));
        int_ban!($t, strict_add_unsigned(<$t>::MAX, 1));
        int_ban!($t, strict_sub_unsigned(<$t>::MIN, 1));
    )* }; }
    macro_rules! unsigned_int { ($($t:ident)*) => { $(
        int_ban!($t, next_power_of_two(<$t>::MAX));
        int_ban!($t, next_multiple_of(1, 0));
        int_ban!($t, div_ceil(1, 0));
        int_ban!($t, strict_neg(1));
        int_ban!($t, strict_add_signed(<$t>::MAX, 1));
        int_ban!($t, strict_sub_signed(0, 1));
    )* }; }
    any_int!(i8 i16 i32 i64 i128 isize u8 u16 u32 u64 u128 usize);
    signed_int!(i8 i16 i32 i64 i128 isize);
    unsigned_int!(u8 u16 u32 u64 u128 usize);
}

#[rustfmt::skip]
fn run_lints() {
    let holder = lints::Holder { v: vec![1], s: String::from("héllo"), map: HashMap::new(), tree: BTreeMap::new(), deque: VecDeque::new() };
    lint!("indexing_slicing" => lints::index_slice(&[1, 2, 3], bb(7)));
    lint!("indexing_slicing" => lints::index_array([1, 2, 3, 4], bb(7)));
    lint!("indexing_slicing" => lints::index_array_ref(&[1, 2, 3, 4], bb(7)));
    lint!("indexing_slicing" => lints::index_vec(&holder, bb(7)));
    lint!("indexing_slicing" => lints::index_nested(&[vec![1]], 0, bb(7)));
    lint!("indexing_slicing" => lints::index_assign(&mut [1], bb(7)));
    lint!("indexing_slicing" => lints::index_bytes("abc", bb(7)));
    lint!("indexing_slicing" => lints::slice_range(&[1, 2, 3], 2, bb(7)));
    lint!("indexing_slicing" => lints::slice_range_from(&[1, 2, 3], bb(7)));
    lint!("indexing_slicing" => lints::slice_range_inclusive(&[1, 2, 3], bb(7)));
    lint!("indexing_slicing" => lints::index_hashmap_owned(HashMap::new(), bb(1)));
    lint!("indexing_slicing" => lints::index_btreemap_owned(BTreeMap::new(), bb(1)));
    lint!("indexing_slicing" => lints::index_vecdeque_owned(VecDeque::new(), bb(0)));
    lint!("indexing_slicing" => lints::index_hashmap_field(&holder, bb(1)));
    lint!("indexing_slicing" => lints::index_btreemap_field(&holder, bb(1)));
    lint!("indexing_slicing" => lints::index_vecdeque_field(&holder, bb(0)));
    lint!("string_slice" => lints::str_slice("héllo", bb(2)));
    lint!("string_slice" => lints::string_slice(&holder, bb(2)));
    lint!("arithmetic_side_effects" => lints::add(bb(255), 1));
    lint!("arithmetic_side_effects" => lints::sub(bb(0), 1));
    lint!("arithmetic_side_effects" => lints::mul(bb(u32::MAX), 2));
    lint!("arithmetic_side_effects" => lints::div(1, bb(0)));
    lint!("arithmetic_side_effects" => lints::rem(1, bb(0)));
    lint!("arithmetic_side_effects" => lints::div_overflow(bb(i32::MIN), bb(-1)));
    lint!("arithmetic_side_effects" => lints::rem_overflow(bb(i32::MIN), bb(-1)));
    lint!("arithmetic_side_effects" => lints::neg(bb(i32::MIN)));
    lint!("arithmetic_side_effects" => lints::add_assign(bb(255), 1));
    lint!("arithmetic_side_effects" => lints::sub_assign(bb(0), 1));
    lint!("arithmetic_side_effects" => lints::mul_assign(bb(255), 2));
    lint!("arithmetic_side_effects" => lints::div_assign(1, bb(0)));
    lint!("arithmetic_side_effects" => lints::rem_assign(1, bb(0)));
    lint!("arithmetic_side_effects" => lints::add_refs(&255, bb(&1)));
    lint!("arithmetic_side_effects" => lints::add_generic(bb(255_u8), 1));
    lint!("arithmetic_side_effects" => lints::nonzero_neg(bb(NonZero::<i32>::MIN)));
    lint!("arithmetic_side_effects" => lints::wrapping_div(Wrapping(1), bb(Wrapping(0))));
    lint!("arithmetic_side_effects" => lints::saturating_div(Saturating(1), bb(Saturating(0))));
    lint!("arithmetic_side_effects" => lints::duration_add(Duration::MAX, bb(Duration::from_secs(1))));
    lint!("arithmetic_side_effects" => lints::duration_sub(Duration::ZERO, bb(Duration::from_secs(1))));
    lint!("arithmetic_side_effects" => lints::duration_mul(Duration::MAX, bb(2)));
    lint!("arithmetic_side_effects" => lints::u32_mul_duration(bb(2), Duration::MAX));
    lint!("arithmetic_side_effects" => lints::duration_div(Duration::from_secs(1), bb(0)));
    lint!("arithmetic_side_effects" => lints::duration_add_assign(Duration::MAX, bb(Duration::from_secs(1))));
    lint!("arithmetic_side_effects" => lints::instant_add(Instant::now(), bb(Duration::MAX)));
    lint!("arithmetic_side_effects" => lints::instant_sub(Instant::now(), bb(Duration::MAX)));
    lint!("arithmetic_side_effects" => lints::systemtime_add(SystemTime::now(), bb(Duration::MAX)));
    lint!("arithmetic_side_effects" => lints::systemtime_sub(SystemTime::now(), bb(Duration::MAX)));
}

#[rustfmt::skip]
fn run_gaps() {
    gap!(gaps::dependency_function(bb(&[])));
    gap!(gaps::refcell_clone(&RefCell::new(0)));
    gap!(gaps::refcell_eq(&RefCell::new(0), &RefCell::new(1)));
    gap!(gaps::refcell_cmp(&RefCell::new(0), &RefCell::new(1)));
    gap!(gaps::lazycell_reentrant());
    gap!(gaps::index_hashmap_through_ref(&HashMap::new(), bb(1)));
    gap!(gaps::index_btreemap_through_ref(&BTreeMap::new(), bb(1)));
    gap!(gaps::index_vecdeque_through_ref(&VecDeque::new(), bb(0)));
    gap!(gaps::index_generic(&[1_u32][..], bb(7)));
    gap!(gaps::shift_left(1, bb(32)));
    gap!(gaps::shift_right(1, bb(32)));
    gap!(gaps::shift_assign(1, bb(32)));
    gap!(gaps::divide_by_minus_one(bb(i32::MIN)));
    gap!(gaps::times_minus_one(bb(i64::MIN)));
    gap!(gaps::zero_minus(bb(i32::MIN)));
    gap!(gaps::arithmetic_in_static(0, 7, bb(0)));
    gap!(gaps::const_generic_array_range(&[1, 2], bb(7)));
    gap!(gaps::inline_const_index([0; 4], &0_u64));
    gap!(gaps::cstr_slice(c"abc", bb(9)));
    gap!(gaps::range_from_past_max(200, bb(100)));
    gap!(gaps::range_from_char_max(bb(char::MAX)));
    run("gap", "", "gaps::sort_by_partial_cmp", || for mut slice in slices_with_nan() { gaps::sort_by_partial_cmp(bb(&mut slice)) });
    gap!(gaps::format_failing_display(&gaps::FailingDisplay));
    gap!(gaps::to_string_failing_display(&gaps::FailingDisplay));
    gap!(gaps::write_failing_display(&mut Vec::new(), &gaps::FailingDisplay));
    gap!(gaps::format_width(bb(70_000)));
    gap!(gaps::atomic_store_ordering(&AtomicU32::new(0), bb(Ordering::Acquire)));
    gap!(gaps::thread_name_with_nul(bb(String::from("a\0b"))));
    gap!(gaps::range_count(bb(u128::MAX)));
    gap!(gaps::vec_with_capacity(bb(usize::MAX)));
    gap!(gaps::vec_macro_with_len(bb(usize::MAX)));
    gap!(gaps::str_repeat("ab", bb(usize::MAX)));
    gap!(gaps::poll_async_block_twice());
    gap!(gaps::poll_ready_twice());
}
