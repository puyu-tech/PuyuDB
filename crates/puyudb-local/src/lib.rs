// SPDX-License-Identifier: Apache-2.0

//! PuyuDB local Git and filesystem integration boundary.
//!
//! Local integration consumes core operations, not own history semantics.
//! I07 Phase 1 (P1+P2) adds read-only Git discovery, capture, and the import
//! preview in [`git`]; Phase 2 (P3) adds import execution in [`import`]
//! through `puyudb_core::api::Database` only.

pub mod git;
pub mod import;
