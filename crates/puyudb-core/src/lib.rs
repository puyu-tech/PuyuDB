// SPDX-License-Identifier: Apache-2.0

//! PuyuDB domain and durable publication boundary.
//!
//! Foundation only: no database operations, storage or persistent formats are
//! implemented. What exists is the I03 canonical identity/profile layer — typed
//! identifiers, the strict canonical CBOR subset, the frozen record layouts with
//! their domain-separated digests, strict validation, path rules and
//! writer-stream checks — because every later durability and engine contract
//! depends on it.

pub mod canon;
pub mod identity;
pub mod profile;
