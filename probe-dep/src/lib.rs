//! Stands in for any third-party dependency: code that clippy does not lint when it lints *your* crate.

/// A macro that hides an `unwrap`.
#[macro_export]
macro_rules! must {
    ($e:expr) => {
        $e.unwrap()
    };
}

/// A macro that hides an `assert!`.
#[macro_export]
macro_rules! check {
    ($e:expr) => {
        assert!($e)
    };
}

/// A macro that hides a `panic!`.
#[macro_export]
macro_rules! die {
    () => {
        panic!("probe_dep::die")
    };
}

/// A function that panics on an empty slice. Nothing in the caller's code shows it.
pub fn first(v: &[u32]) -> u32 {
    v[0]
}
