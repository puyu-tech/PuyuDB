// SPDX-License-Identifier: Apache-2.0

//! Strict canonical CBOR subset used by the frozen profile.
//!
//! The rules are RFC 8949 core deterministic encoding restricted further by
//! [`docs/spec/canonical-profile-v1.md`](../../../docs/spec/canonical-profile-v1.md):
//! definite lengths, shortest integers, no floats, no CBOR tag items, no maps,
//! valid UTF-8 text, and full input exhaustion. The encoder is hand-written for
//! these fixed schemas on purpose; a generic serializer's "deterministic mode" is
//! not accepted as evidence.

use thiserror::Error;

/// Maximum array nesting the decoder will follow.
pub const MAX_DEPTH: usize = 16;

/// Canonical-subset failure. Every variant is an encoding error; the profile
/// layer maps them to its single `invalid_encoding` code.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    /// Input ended inside an item.
    #[error("truncated input")]
    Truncated,
    /// An integer or length used more bytes than its value requires.
    #[error("non-minimal length or integer encoding")]
    NonMinimal,
    /// Streamed (indefinite-length) items.
    #[error("indefinite length is not canonical here")]
    Indefinite,
    /// CBOR maps.
    #[error("maps are not part of this profile")]
    Map,
    /// CBOR tag items.
    #[error("CBOR tag items are not part of this profile")]
    Tag,
    /// Floats and other simple values.
    #[error("floats and simple values are not part of this profile")]
    Simple,
    /// Negative integers.
    #[error("negative integers are not part of this profile")]
    Negative,
    /// Additional-information codes 28-30.
    #[error("reserved additional information")]
    Reserved,
    /// Text string that is not valid UTF-8.
    #[error("text string is not valid UTF-8")]
    Utf8,
    /// Bytes left over after one record.
    #[error("trailing bytes after record")]
    Trailing,
    /// Nesting deeper than [`MAX_DEPTH`].
    #[error("nesting too deep")]
    Depth,
    /// Integer that does not fit in 64 bits.
    #[error("integer exceeds 64 bits")]
    Wide,
}

/// One decoded item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// Unsigned integer.
    Uint(u64),
    /// Byte string.
    Bytes(Vec<u8>),
    /// UTF-8 text string.
    Text(String),
    /// Definite-length array.
    Array(Vec<Item>),
    /// Null.
    Null,
}

const NULL: u8 = 0xf6;

fn head(out: &mut Vec<u8>, major: u8, count: u64) -> Result<(), Error> {
    let major = major << 5;
    match count {
        0..=23 => out.push(major | count as u8),
        24..=0xff => out.extend_from_slice(&[major | 24, count as u8]),
        0x100..=0xffff => {
            out.push(major | 25);
            out.extend_from_slice(&(count as u16).to_be_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(major | 26);
            out.extend_from_slice(&(count as u32).to_be_bytes());
        }
        _ => {
            out.push(major | 27);
            out.extend_from_slice(&count.to_be_bytes());
        }
    }
    Ok(())
}

/// Append an unsigned integer.
pub fn write_uint(out: &mut Vec<u8>, value: u64) -> Result<(), Error> {
    head(out, 0, value)
}

/// Append a byte string.
pub fn write_bytes(out: &mut Vec<u8>, value: &[u8]) -> Result<(), Error> {
    head(out, 2, value.len() as u64)?;
    out.extend_from_slice(value);
    Ok(())
}

/// Append a text string.
pub fn write_text(out: &mut Vec<u8>, value: &str) -> Result<(), Error> {
    head(out, 3, value.len() as u64)?;
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

/// Append an array header; the caller writes exactly `len` items afterwards.
pub fn write_array_header(out: &mut Vec<u8>, len: usize) -> Result<(), Error> {
    head(out, 4, len as u64)
}

/// Append the null item.
pub fn write_null(out: &mut Vec<u8>) {
    out.push(NULL);
}

/// Encode an item tree canonically.
pub fn encode(item: &Item) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    write_item(&mut out, item)?;
    Ok(out)
}

fn write_item(out: &mut Vec<u8>, item: &Item) -> Result<(), Error> {
    match item {
        Item::Uint(value) => write_uint(out, *value),
        Item::Bytes(value) => write_bytes(out, value),
        Item::Text(value) => write_text(out, value),
        Item::Array(items) => {
            write_array_header(out, items.len())?;
            for item in items {
                write_item(out, item)?;
            }
            Ok(())
        }
        Item::Null => {
            write_null(out);
            Ok(())
        }
    }
}

/// Decode exactly one item and reject trailing bytes.
pub fn decode(input: &[u8]) -> Result<Item, Error> {
    let (item, used) = decode_at(input, 0)?;
    if used != input.len() {
        return Err(Error::Trailing);
    }
    Ok(item)
}

fn decode_head(input: &[u8], at: usize) -> Result<(u8, u64), Error> {
    let initial = *input.get(at).ok_or(Error::Truncated)?;
    let major = initial >> 5;
    let info = initial & 0x1f;
    let (width, minimum) = match info {
        0..=23 => return Ok((major, u64::from(info))),
        24 => (1, 24),
        25 => (2, 0x100),
        26 => (4, 0x1_0000),
        27 => (8, 0x1_0000_0000),
        31 => return Err(Error::Indefinite),
        _ => return Err(Error::Reserved),
    };
    let start = at + 1;
    let end = start.checked_add(width).ok_or(Error::Truncated)?;
    let raw = input.get(start..end).ok_or(Error::Truncated)?;
    let mut value = 0u64;
    for byte in raw {
        value = (value << 8) | u64::from(*byte);
    }
    if value < minimum {
        return Err(Error::NonMinimal);
    }
    Ok((major, value))
}

fn arg_end(at: usize, width: usize) -> Result<usize, Error> {
    at.checked_add(width).ok_or(Error::Truncated)
}

fn decode_at(input: &[u8], at: usize) -> Result<(Item, usize), Error> {
    decode_depth(input, at, 0)
}

fn decode_depth(input: &[u8], at: usize, depth: usize) -> Result<(Item, usize), Error> {
    if depth > MAX_DEPTH {
        return Err(Error::Depth);
    }
    let (major, value) = decode_head(input, at)?;
    let after_head = head_width(input, at)?;
    match major {
        0 => Ok((Item::Uint(value), after_head)),
        1 => Err(Error::Negative),
        2 | 3 => {
            let start = after_head;
            let end = start
                .checked_add(usize::try_from(value).map_err(|_| Error::Truncated)?)
                .ok_or(Error::Truncated)?;
            let raw = input.get(start..end).ok_or(Error::Truncated)?;
            if major == 2 {
                Ok((Item::Bytes(raw.to_vec()), end))
            } else {
                let text = std::str::from_utf8(raw).map_err(|_| Error::Utf8)?;
                Ok((Item::Text(text.to_owned()), end))
            }
        }
        4 => {
            let count = usize::try_from(value).map_err(|_| Error::Truncated)?;
            let mut items = Vec::new();
            let mut cursor = after_head;
            for _ in 0..count {
                let (item, next) = decode_depth(input, cursor, depth + 1)?;
                items.push(item);
                cursor = next;
            }
            Ok((Item::Array(items), cursor))
        }
        5 => Err(Error::Map),
        6 => Err(Error::Tag),
        _ => {
            if value == 22 {
                Ok((Item::Null, after_head))
            } else {
                Err(Error::Simple)
            }
        }
    }
}

/// Length of the head that starts at `at`, without reinterpreting its value.
fn head_width(input: &[u8], at: usize) -> Result<usize, Error> {
    let info = input.get(at).ok_or(Error::Truncated)? & 0x1f;
    let width = match info {
        0..=23 => 0,
        24 => 1,
        25 => 2,
        26 => 4,
        27 => 8,
        31 => return Err(Error::Indefinite),
        _ => return Err(Error::Reserved),
    };
    arg_end(at + 1, width)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(value: &str) -> Vec<u8> {
        (0..value.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&value[index..index + 2], 16).expect("hex"))
            .collect()
    }

    #[test]
    fn integers_use_the_shortest_form() {
        let cases = [
            (0u64, "00"),
            (23, "17"),
            (24, "1818"),
            (255, "18ff"),
            (256, "190100"),
            (65535, "19ffff"),
            (65536, "1a00010000"),
            (4_294_967_296, "1b0000000100000000"),
        ];
        for (value, encoded) in cases {
            let mut out = Vec::new();
            write_uint(&mut out, value).expect("write");
            assert_eq!(out, hex(encoded), "encoding {value}");
            assert_eq!(decode(&out).expect("decode"), Item::Uint(value));
        }
    }

    #[test]
    fn non_canonical_input_is_rejected() {
        for encoded in [
            "1800",
            "190000",
            "1a00000000",
            "1b0000000000000000",
            "9f01ff",
            "a10102",
            "c101",
            "fb3ff0000000000000",
            "2001",
            "0100",
            "82",
            "48010203",
            "62ff80",
        ] {
            assert!(decode(&hex(encoded)).is_err(), "accepted {encoded}");
        }
    }

    #[test]
    fn round_trip_preserves_the_item_tree() {
        let item = Item::Array(vec![
            Item::Uint(1),
            Item::Bytes(vec![0, 1, 2]),
            Item::Text("é".to_owned()),
            Item::Null,
        ]);
        let encoded = encode(&item).expect("encode");
        assert_eq!(decode(&encoded).expect("decode"), item);
    }
}
