//! Triagebot integration testsuite.
//!
//! There are two types of tests here:
//!
//! * `github_client` — This tests the behavior `GithubClient`.
//! * `server_test` — This launches the `triagebot` executable, injects
//!   webhook events into it, and validates the behavior.
//!
//! See the individual modules for an introduction to writing these tests.
//!
//! The `common` module contains some code that is common for setting up the
//! tests. The tests generally work by launching an HTTP server and
//! intercepting HTTP requests that would normally go to external sites like
//! https://api.github.com.

mod common;
mod github_client;
mod server_test;
