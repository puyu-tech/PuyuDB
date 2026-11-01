// SPDX-License-Identifier: Apache-2.0

use puyudb_crdt::experiment::EditContext;

pub fn context(writer: u8, sequence: u64) -> EditContext {
    EditContext {
        public_key: [writer; 32],
        branch: [1; 16],
        writer_epoch: [writer; 16],
        content_epoch: [7; 32],
        outer_sequence: sequence,
    }
}
