// SPDX-License-Identifier: Apache-2.0

//! PuyuDB domain and durable publication boundary.
//!
//! The I03 canonical identity/profile layer is the admission input. I04 adds
//! the local publication ledger: staged chunks, one atomic batch commit, and
//! a durable ack only after that commit.

pub mod canon;
pub mod identity;
pub mod profile;
pub mod publication;
