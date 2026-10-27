// SPDX-License-Identifier: Apache-2.0

//! Typed identifiers and their presentation form.
//!
//! Widths are frozen by the canonical profile. Presentation is deliberately not
//! identity: a presented string is `prefix:lowercase-hex` and is parsed strictly,
//! so an uppercase, truncated, padded, or differently prefixed string is an
//! error rather than a second spelling of one ID.

use std::fmt;
use std::str::FromStr;

use thiserror::Error;

/// Identifier parse failure.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum IdError {
    /// Presented width does not match the type.
    #[error("expected {expected} hex characters, found {found}")]
    Width {
        /// Characters the type requires.
        expected: usize,
        /// Characters supplied.
        found: usize,
    },
    /// Presentation prefix does not name this type.
    #[error("unexpected identifier prefix")]
    Prefix,
    /// Characters outside `0-9a-f`.
    #[error("identifier body must be lowercase hex")]
    NotHex,
}

macro_rules! identifier {
    ($(#[$meta:meta])* $name:ident, $width:literal, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; $width]);

        impl $name {
            /// Exact byte width of this identifier.
            pub const WIDTH: usize = $width;

            /// Wrap already-width-checked bytes.
            #[must_use]
            pub const fn from_array(bytes: [u8; $width]) -> Self {
                Self(bytes)
            }

            /// Borrow the raw bytes.
            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; $width] {
                &self.0
            }

            /// Copy the raw bytes out.
            #[must_use]
            pub const fn to_bytes(self) -> [u8; $width] {
                self.0
            }

            /// Copy from a slice of exactly the right width.
            ///
            /// # Errors
            /// Returns [`IdError::Width`] for any other length.
            pub fn from_bytes(bytes: &[u8]) -> Result<Self, IdError> {
                let array: [u8; $width] = bytes.try_into().map_err(|_| IdError::Width {
                    expected: $width,
                    found: bytes.len(),
                })?;
                Ok(Self(array))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}:{}", $prefix, hex(&self.0))
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(self, formatter)
            }
        }

        impl FromStr for $name {
            type Err = IdError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                let (prefix, body) = value.split_once(':').ok_or(IdError::Prefix)?;
                if prefix != $prefix {
                    return Err(IdError::Prefix);
                }
                if body.len() != $width * 2 {
                    return Err(IdError::Width {
                        expected: $width * 2,
                        found: body.len(),
                    });
                }
                let mut bytes = [0u8; $width];
                for (index, byte) in bytes.iter_mut().enumerate() {
                    let pair = body.as_bytes().get(index * 2..index * 2 + 2).ok_or(IdError::Width {
                        expected: $width * 2,
                        found: body.len(),
                    })?;
                    *byte = unhex(pair[0])? << 4 | unhex(pair[1])?;
                }
                Ok(Self(bytes))
            }
        }
    };
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

fn unhex(byte: u8) -> Result<u8, IdError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(IdError::NotHex),
    }
}

identifier!(
    /// Sharing, authorization and replication unit.
    SpaceId,
    16,
    "space1"
);
identifier!(
    /// Branch view inside one space.
    BranchId,
    16,
    "branch1"
);
identifier!(
    /// Stable logical file identity.
    FileId,
    16,
    "file1"
);
identifier!(
    /// One writer-stream incarnation in one branch.
    WriterEpoch,
    16,
    "writer1"
);
identifier!(
    /// Epoch-descriptor discriminator.
    GenerationNonce,
    16,
    "nonce1"
);
identifier!(
    /// Content-lineage digest.
    EpochId,
    32,
    "epoch1"
);
identifier!(
    /// Immutable batch digest.
    BatchId,
    32,
    "batch1"
);
identifier!(
    /// Artifact-manifest digest.
    ArtifactId,
    32,
    "artifact1"
);
identifier!(
    /// Raw-chunk digest.
    ChunkId,
    32,
    "chunk1"
);
identifier!(
    /// Frontier (version) digest.
    VersionId,
    32,
    "version1"
);
identifier!(
    /// Conflict identity digest.
    ConflictId,
    32,
    "conflict1"
);

/// Git object identity: the algorithm stays separate from the digest so a 20-byte
/// and a 32-byte object can never be compared or confused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GitOid {
    /// SHA-1 object name.
    Sha1([u8; 20]),
    /// SHA-256 object name.
    Sha256([u8; 32]),
}

impl GitOid {
    /// Algorithm tag used by the canonical record.
    #[must_use]
    pub const fn algorithm_tag(self) -> u64 {
        match self {
            Self::Sha1(_) => 1,
            Self::Sha256(_) => 2,
        }
    }

    /// Raw digest bytes.
    #[must_use]
    pub fn digest(&self) -> &[u8] {
        match self {
            Self::Sha1(bytes) => bytes,
            Self::Sha256(bytes) => bytes,
        }
    }
}

impl fmt::Display for GitOid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let algorithm = match self {
            Self::Sha1(_) => "sha1",
            Self::Sha256(_) => "sha256",
        };
        write!(formatter, "gitoid1:{algorithm}:{}", hex(self.digest()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presentation_round_trips() {
        let space = SpaceId::from_array([0xab; 16]);
        let text = space.to_string();
        assert_eq!(text, format!("space1:{}", "ab".repeat(16)));
        assert_eq!(SpaceId::from_str(&text).expect("parse"), space);
        let epoch = EpochId::from_array([0x01; 32]);
        assert!(epoch.to_string().ends_with(&"01".repeat(32)));
    }

    #[test]
    fn loose_spellings_are_rejected() {
        let canonical = format!("space1:{}", "ab".repeat(16));
        for broken in [
            canonical.to_uppercase(),
            canonical.replace(':', ""),
            format!("{canonical}0"),
            canonical.replace("space1", "branch1"),
            "space1:zz".repeat(16),
        ] {
            assert!(SpaceId::from_str(&broken).is_err(), "accepted {broken}");
        }
    }

    #[test]
    fn widths_are_enforced() {
        assert!(SpaceId::from_bytes(&[0; 15]).is_err());
        assert!(EpochId::from_bytes(&[0; 32]).is_ok());
        assert_eq!(SpaceId::WIDTH, 16);
        assert_eq!(BatchId::WIDTH, 32);
    }

    #[test]
    fn git_oids_keep_algorithm_separate() {
        assert_eq!(GitOid::Sha1([0; 20]).algorithm_tag(), 1);
        assert_eq!(GitOid::Sha256([0; 32]).algorithm_tag(), 2);
        assert_eq!(GitOid::Sha1([1; 20]).digest().len(), 20);
        assert!(
            GitOid::Sha256([1; 32])
                .to_string()
                .starts_with("gitoid1:sha256:")
        );
    }
}
