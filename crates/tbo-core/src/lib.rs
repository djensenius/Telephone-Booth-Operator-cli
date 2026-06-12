//! Core domain types, configuration, and errors shared across the
//! `tb-operator` crates.
//!
//! This crate is the dependency-light foundation of the workspace: it mirrors
//! the operator API's data model (status, messages, questions, events,
//! sessions, stats) and holds the console's configuration and error types.
//! Concrete types are added in later phases.
