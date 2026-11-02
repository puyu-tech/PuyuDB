// SPDX-License-Identifier: Apache-2.0

//! PuyuDB native text integration boundary.
//!
//! `dt_text` is the production `dt-text-v1` binding over the pinned
//! diamond-types fork (I05 P7/P8); `experiment` holds the I02 phase-0
//! evidence only: no database operation, stable public API, accepted native
//! format, or persistent PuyuDB format is provided by the experiment module.

pub mod dt_text;
pub mod experiment;
