//! Core domain types, configuration, and errors shared across the
//! `tb-operator` crates.
//!
//! This crate is the dependency-light foundation of the workspace. The
//! [`domain`] module mirrors the operator API's wire contract (the Zod schemas
//! in the `Telephone-Booth-Operator` monorepo's `packages/shared`), translated
//! to `serde` types with `camelCase` field renaming so the JSON shapes agree
//! exactly. [`config`] holds the console's on-disk configuration and
//! [`error`] its shared error type.

// Wire DTOs mix `Eq`-able fields with floating-point ones (`f64`), so deriving
// `Eq` on only the subset that qualifies would make the module inconsistent
// for no semantic benefit. Keep every DTO at `PartialEq`.
#![allow(clippy::derive_partial_eq_without_eq)]

pub mod config;
pub mod domain;
pub mod error;

pub use error::{Error, Result};
