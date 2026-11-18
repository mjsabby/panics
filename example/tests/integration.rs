//! An integration test is a crate of its own, so the `allow` goes at the top of the file.
#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects,
    reason = "a test is supposed to panic when something is wrong"
)]

use example::{Error, average, read_u32};

#[test]
fn a_length_prefixed_record() {
    let input = [0, 0, 0, 2, 10, 20, 99];
    let (len, rest) = read_u32(&input).unwrap();
    let body = &rest[..len as usize];
    assert_eq!(
        average(&body.iter().map(|b| u32::from(*b)).collect::<Vec<_>>()),
        Ok(15)
    );
    assert_eq!(read_u32(&input[..3]), Err(Error::Truncated));
}
