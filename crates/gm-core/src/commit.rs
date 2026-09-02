//! Commit manifests.
//!
//! A commit is not a special kind of record: it is a blob like any other, and
//! its hash is the hash of its canonical JSON. That single decision is what
//! makes sync trivial — "send me the blobs I do not have" transfers history,
//! models and materials through one code path, and a commit id verifies its own
//! contents by construction.

use crate::canon;
use crate::error::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One document in a commit, addressed by kind and stable key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub kind: String,
    pub key: String,
    /// Hash of the document blob.
    pub blob: String,
}

/// The complete contents of a revision. Manifests are snapshots, not deltas:
/// every document present at that revision is listed, so reading any commit
/// costs one pass and never needs history replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    /// Discriminator, so a blob can be identified without an index.
    #[serde(rename = "type")]
    pub type_: String,
    /// First parent first. Empty for the root commit, two or more for a merge.
    #[serde(default)]
    pub parents: Vec<String>,
    pub author: String,
    /// RFC 3339, always UTC. Local time would make hashes depend on the
    /// committer's timezone.
    pub committed_at: String,
    pub message: String,
    /// Sorted by `(kind, key)` before hashing, so two tools that assemble the
    /// same revision in different orders still agree on the commit id.
    pub entries: Vec<ManifestEntry>,
}

impl Manifest {
    pub const TYPE: &'static str = "gm.commit/1";

    pub fn new(
        parents: Vec<String>,
        author: impl Into<String>,
        committed_at: impl Into<String>,
        message: impl Into<String>,
        mut entries: Vec<ManifestEntry>,
    ) -> Self {
        entries.sort_by(|a, b| (&a.kind, &a.key).cmp(&(&b.kind, &b.key)));
        Self {
            type_: Self::TYPE.to_string(),
            parents,
            author: author.into(),
            committed_at: committed_at.into(),
            message: message.into(),
            entries,
        }
    }

    pub fn to_value(&self) -> Result<Value> {
        Ok(serde_json::to_value(self)?)
    }

    /// The commit id: the hash of this manifest's canonical form.
    pub fn hash(&self) -> Result<String> {
        canon::hash_value(&self.to_value()?)
    }

    pub fn entry(&self, kind: &str, key: &str) -> Option<&ManifestEntry> {
        self.entries.iter().find(|e| e.kind == kind && e.key == key)
    }

    pub fn entries_of_kind<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a ManifestEntry> {
        self.entries.iter().filter(move |e| e.kind == kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: &str, key: &str, blob: &str) -> ManifestEntry {
        ManifestEntry {
            kind: kind.into(),
            key: key.into(),
            blob: blob.into(),
        }
    }

    #[test]
    fn entry_order_does_not_affect_the_commit_id() {
        let a = Manifest::new(
            vec![],
            "a@example.com",
            "2026-09-02T14:00:00Z",
            "initial",
            vec![
                entry("material", "LONDON_CLAY", "sha256-bb"),
                entry("ground_model", "CH-100", "sha256-aa"),
            ],
        );
        let b = Manifest::new(
            vec![],
            "a@example.com",
            "2026-09-02T14:00:00Z",
            "initial",
            vec![
                entry("ground_model", "CH-100", "sha256-aa"),
                entry("material", "LONDON_CLAY", "sha256-bb"),
            ],
        );
        assert_eq!(a.hash().unwrap(), b.hash().unwrap());
    }

    #[test]
    fn changing_one_document_changes_the_commit_id() {
        let base = Manifest::new(
            vec![],
            "a@example.com",
            "2026-09-02T14:00:00Z",
            "initial",
            vec![entry("ground_model", "CH-100", "sha256-aa")],
        );
        let changed = Manifest::new(
            vec![],
            "a@example.com",
            "2026-09-02T14:00:00Z",
            "initial",
            vec![entry("ground_model", "CH-100", "sha256-ab")],
        );
        assert_ne!(base.hash().unwrap(), changed.hash().unwrap());
    }

    #[test]
    fn manifests_round_trip_through_json() {
        let m = Manifest::new(
            vec!["sha256-parent".into()],
            "a@example.com",
            "2026-09-02T14:00:00Z",
            "second",
            vec![entry("material", "MADE_GROUND", "sha256-cc")],
        );
        let value = m.to_value().unwrap();
        let back: Manifest = serde_json::from_value(value).unwrap();
        assert_eq!(m, back);
    }
}
