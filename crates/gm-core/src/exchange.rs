//! The flat interchange document.
//!
//! The SQLite file is the thing you *keep*: versioned, queryable, syncable. This
//! JSON document is the thing you *send* when the other end has no tooling — an
//! email attachment, a request body, a file in a git repo where a diff needs to
//! be readable. It is one revision, flattened, with no history.
//!
//! Round-tripping is lossless for the ground model itself and lossy only for
//! history, which is the intended trade.

use crate::error::Result;
use crate::model::{FileMetadata, GroundModel, Material};
use crate::schema::SCHEMA_VERSION;
use crate::store::State;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Exchange {
    /// Discriminator and version, so a reader can reject a document it does not
    /// understand rather than silently misreading it.
    #[serde(rename = "type")]
    pub type_: String,
    pub schema_version: String,
    /// The commit this was exported from, when it came from a repository. Not
    /// required, but it lets an import say what it is a copy of.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_commit: Option<String>,
    pub file: FileMetadata,
    #[serde(default)]
    pub materials: Vec<Material>,
    #[serde(default)]
    pub models: Vec<GroundModel>,
}

impl Exchange {
    pub const TYPE: &'static str = "gm.file/1";

    pub fn from_state(state: &State, source_commit: Option<String>) -> Self {
        Self {
            type_: Self::TYPE.to_string(),
            schema_version: SCHEMA_VERSION.to_string(),
            source_commit,
            file: state.file_metadata.clone(),
            // BTreeMap iteration is already sorted by key, which keeps exported
            // JSON byte-stable and therefore diffable in git.
            materials: state.materials.values().cloned().collect(),
            models: state.models.values().cloned().collect(),
        }
    }

    pub fn into_state(self) -> State {
        State {
            file_metadata: self.file,
            materials: self
                .materials
                .into_iter()
                .map(|m| (m.material_key.clone(), m))
                .collect(),
            models: self
                .models
                .into_iter()
                .map(|m| (m.model_key.clone(), m))
                .collect(),
        }
    }

    pub fn to_json_pretty(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn from_json(text: &str) -> Result<Self> {
        let doc: Exchange = serde_json::from_str(text)?;
        if doc.type_ != Self::TYPE {
            return Err(crate::error::invalid(format!(
                "expected a '{}' document, found '{}'",
                Self::TYPE,
                doc.type_
            )));
        }
        Ok(doc)
    }
}

/// Merge `incoming` into `base`, with incoming winning on conflicts. Used by
/// `gm import` when adding models to a file that already has some.
pub fn merge_into(base: &mut State, incoming: State, replace_metadata: bool) -> MergeReport {
    let mut report = MergeReport::default();

    if replace_metadata {
        base.file_metadata = incoming.file_metadata;
    }
    merge_map(
        &mut base.materials,
        incoming.materials,
        &mut report.materials,
    );
    merge_map(&mut base.models, incoming.models, &mut report.models);
    report
}

fn merge_map<T: PartialEq>(
    base: &mut BTreeMap<String, T>,
    incoming: BTreeMap<String, T>,
    counts: &mut MergeCounts,
) {
    for (key, value) in incoming {
        match base.get(&key) {
            Some(existing) if *existing == value => counts.unchanged += 1,
            Some(_) => {
                counts.replaced += 1;
                base.insert(key, value);
            }
            None => {
                counts.added += 1;
                base.insert(key, value);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MergeCounts {
    pub added: usize,
    pub replaced: usize,
    pub unchanged: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MergeReport {
    pub materials: MergeCounts,
    pub models: MergeCounts,
}
