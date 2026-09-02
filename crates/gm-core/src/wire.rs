//! The sync wire format.
//!
//! A bundle is a bag of objects: no ordering guarantees, no deltas, no
//! compression. That is not laziness — every object carries its own hash, so a
//! receiver verifies each one independently and a truncated or reordered
//! transfer is caught rather than half-applied. Deduplication has already done
//! the work that a delta encoder would: sending a revision of a six-model route
//! that moved one layer boundary is two objects, not six.
//!
//! ```text
//! gm-bundle/1\n
//! sha256-<hex> <byte length>\n
//! <that many bytes>
//! sha256-<hex> <byte length>\n
//! <that many bytes>
//! ...
//! ```
//!
//! Lengths are byte counts, so the payload needs no escaping and the decoder
//! never has to guess where an object ends.

use crate::error::{Result, invalid};

pub const BUNDLE_MAGIC: &str = "gm-bundle/1";

/// One object on the wire: its hash, and its canonical bytes exactly as stored.
pub type Object = (String, Vec<u8>);

pub fn encode_bundle(objects: &[Object]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(BUNDLE_MAGIC.as_bytes());
    out.push(b'\n');
    for (hash, bytes) in objects {
        out.extend_from_slice(format!("{hash} {}\n", bytes.len()).as_bytes());
        out.extend_from_slice(bytes);
    }
    out
}

pub fn decode_bundle(bytes: &[u8]) -> Result<Vec<Object>> {
    let mut rest = bytes;

    let (magic, after) = split_line(rest)?;
    if magic != BUNDLE_MAGIC {
        return Err(invalid(format!(
            "not a ground-model bundle: expected '{BUNDLE_MAGIC}', found '{}'",
            magic.chars().take(40).collect::<String>()
        )));
    }
    rest = after;

    let mut objects = Vec::new();
    while !rest.is_empty() {
        let (header, after) = split_line(rest)?;
        let (hash, len) = header
            .split_once(' ')
            .ok_or_else(|| invalid(format!("malformed bundle entry: '{header}'")))?;
        let len: usize = len
            .parse()
            .map_err(|_| invalid(format!("malformed object length in '{header}'")))?;

        if after.len() < len {
            return Err(invalid(format!(
                "bundle is truncated: {hash} claims {len} bytes, {} remain",
                after.len()
            )));
        }
        objects.push((hash.to_string(), after[..len].to_vec()));
        rest = &after[len..];
    }
    Ok(objects)
}

fn split_line(bytes: &[u8]) -> Result<(&str, &[u8])> {
    let end = bytes
        .iter()
        .position(|b| *b == b'\n')
        .ok_or_else(|| invalid("bundle ended mid-line"))?;
    let line = std::str::from_utf8(&bytes[..end])
        .map_err(|_| invalid("bundle header is not valid UTF-8"))?;
    Ok((line, &bytes[end + 1..]))
}

/// What a remote says about itself, before any objects move.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteInfo {
    /// Wire protocol version. Bumped only for an incompatible change.
    pub protocol: u32,
    /// Identifies the project. Two copies may only sync if these match.
    pub file_id: String,
    pub schema_version: String,
    pub head: Option<String>,
    pub name: String,
    /// Whether this remote will accept a push.
    #[serde(default)]
    pub accepts_push: bool,
}

impl RemoteInfo {
    pub const PROTOCOL: u32 = 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(hash: &str, body: &str) -> Object {
        (hash.to_string(), body.as_bytes().to_vec())
    }

    #[test]
    fn a_bundle_round_trips() {
        let objects = vec![
            object("sha256-aa", r#"{"modelKey":"CH-100"}"#),
            object("sha256-bb", r#"{"materialKey":"LONDON_CLAY"}"#),
        ];
        let encoded = encode_bundle(&objects);
        assert_eq!(decode_bundle(&encoded).unwrap(), objects);
    }

    #[test]
    fn an_empty_bundle_is_valid() {
        assert_eq!(decode_bundle(&encode_bundle(&[])).unwrap(), vec![]);
    }

    #[test]
    fn payloads_containing_newlines_survive() {
        // Length prefixes mean the payload needs no escaping, which matters
        // because a canonical document may contain anything at all.
        let objects = vec![object("sha256-aa", "line\nline\n42 not-a-header\n")];
        let encoded = encode_bundle(&objects);
        assert_eq!(decode_bundle(&encoded).unwrap(), objects);
    }

    #[test]
    fn a_truncated_bundle_is_rejected_rather_than_half_applied() {
        let encoded = encode_bundle(&[object("sha256-aa", "0123456789")]);
        let truncated = &encoded[..encoded.len() - 4];
        let err = decode_bundle(truncated).expect_err("should refuse");
        assert!(err.to_string().contains("truncated"), "got: {err}");
    }

    #[test]
    fn something_that_is_not_a_bundle_is_rejected() {
        let err = decode_bundle(b"<!doctype html>\n<html>\n").expect_err("should refuse");
        assert!(
            err.to_string().contains("not a ground-model bundle"),
            "got: {err}"
        );
    }

    #[test]
    fn an_empty_body_is_rejected() {
        assert!(decode_bundle(b"").is_err());
    }
}
