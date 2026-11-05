// SPDX-License-Identifier: Apache-2.0

//! PuyuDB domain and durable publication boundary.
//!
//! The I03 canonical identity/profile layer is the admission input. I04 adds
//! the local publication ledger: staged chunks, one atomic batch commit, and
//! a durable ack only after that commit. I06 adds the public embedded facade
//! (`api`): create/open, the §14 error codes, and the keyed transaction
//! interface.

pub mod api;
pub mod canon;
pub mod identity;
pub mod native;
pub mod profile;
pub mod projection;
pub mod publication;
