// SPDX-License-Identifier: Apache-2.0

//! Independent CRC-32C fixture arithmetic, not a native decoder.

pub(crate) fn crc32c(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 == 0 { 0 } else { 0x82f6_3b78 };
        }
    }
    !crc
}

pub(crate) fn seal(mut prefix: Vec<u8>) -> Vec<u8> {
    let crc = crc32c(&prefix);
    prefix.extend([100, 4]); // Crc tag, four-byte little-endian payload.
    prefix.extend(crc.to_le_bytes());
    prefix
}
