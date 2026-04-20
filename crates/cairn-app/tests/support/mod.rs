//! Shared test support for cairn-app integration tests.
//!
//! Each `tests/*.rs` file is its own crate; include this module via
//! `mod support;` to access the helpers below.

#![allow(dead_code)] // Not every test file uses every helper.

pub mod fake_fabric;
