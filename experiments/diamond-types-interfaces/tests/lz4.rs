// SPDX-License-Identifier: Apache-2.0

//! Actual native compression compatibility, not the strict input dialect.
//! The harness selects upstream defaults on Unix or LZ4 alone on Windows.

use diamond_types::list::{
    ListOpLog,
    encoding::{EncodeOptions, ParseError},
};

#[test]
fn ordinary_compressed_start_content_still_decodes_while_strict_mode_refuses_it() {
    let baseline = "abcd😀日本".repeat(512);
    let mut source = ListOpLog::new();
    let seed = source.get_or_create_agent_id("seed");
    source.add_insert(seed, 0, &baseline);
    let plain = EncodeOptions::full().compress_content(false);
    let seed_bytes = source.encode(&plain);
    let from = source.local_frontier();
    let writer = source.get_or_create_agent_id("writer");
    source.add_insert_at(writer, from.as_ref(), 4, "XYZ");
    // Full-mode encode_from stores the known start text. It exercises the local
    // decompression-buffer lifetime that the first borrowed-report patch broke.
    let update = source.encode_from(&EncodeOptions::full().compress_content(true), from.as_ref());
    assert_eq!(
        update[9], 5,
        "fixture must really contain CompressedFieldsLZ4"
    );
    let mut receiver = ListOpLog::load_from(&seed_bytes).unwrap();
    receiver.decode_and_add(&update).unwrap();
    let mut expected = baseline;
    expected.insert_str(4, "XYZ");
    assert_eq!(receiver.checkout_tip().content().to_string(), expected);
    let snapshot = source.encode(&EncodeOptions::full().compress_content(true));
    let loaded = ListOpLog::load_from(&snapshot).unwrap();
    assert_eq!(loaded.checkout_tip().content().to_string(), expected);
    let base = ListOpLog::load_from(&seed_bytes).unwrap();
    assert_eq!(
        base.decode_new_strict(&update).unwrap_err(),
        ParseError::LZ4DecoderNeeded
    );
    assert_eq!(base.encode(&plain), seed_bytes);
}
