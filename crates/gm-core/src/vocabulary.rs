//! The advisory vocabulary shared with the browser editor.
//!
//! `assets/vocabulary.json` is what the editor offers in its dropdowns:
//! property names, unit strings, soil classes and constitutive kinds. It is a
//! suggestion list, not a schema — the format keeps all four open on purpose
//! (see `docs/format.md`) and anything outside it round-trips untouched.
//!
//! One part of it *is* load-bearing here: the set of constitutive kinds. If the
//! editor offered a kind that [`crate::validate`] then warned about, the file
//! would look wrong the moment it reached `gm commit`. Both sides read the same
//! list rather than keeping a copy each.

use serde::Deserialize;
use std::collections::BTreeSet;
use std::sync::LazyLock;

const SOURCE: &str = include_str!("../../../assets/vocabulary.json");

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Vocabulary {
    soil_classes: Vec<String>,
    properties: Vec<Term>,
    constitutive_kinds: Vec<Kind>,
}

#[derive(Debug, Deserialize)]
pub struct Term {
    pub key: String,
    pub label: String,
    pub units: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Kind {
    pub kind: String,
    pub label: String,
    #[serde(default)]
    pub parameters: Vec<Term>,
}

static VOCABULARY: LazyLock<Vocabulary> = LazyLock::new(|| {
    serde_json::from_str(SOURCE).expect("assets/vocabulary.json is built in and must parse")
});

static KINDS: LazyLock<BTreeSet<&'static str>> = LazyLock::new(|| {
    VOCABULARY
        .constitutive_kinds
        .iter()
        .map(|k| k.kind.as_str())
        .collect()
});

/// Constitutive kinds this build understands well enough to check parameters
/// for. Anything else is carried through untouched and warned about once.
pub fn knows_constitutive_kind(kind: &str) -> bool {
    KINDS.contains(kind)
}

pub fn constitutive_kinds() -> &'static [Kind] {
    &VOCABULARY.constitutive_kinds
}

pub fn properties() -> &'static [Term] {
    &VOCABULARY.properties
}

pub fn soil_classes() -> &'static [String] {
    &VOCABULARY.soil_classes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vocabulary_parses_and_is_not_empty() {
        assert!(!properties().is_empty());
        assert!(!soil_classes().is_empty());
        assert!(!constitutive_kinds().is_empty());
    }

    /// The kinds `validate` writes parameter-specific checks for must be in the
    /// list, or those checks would never run and the editor would not offer the
    /// kind that triggers them.
    #[test]
    fn kinds_with_parameter_checks_are_offered() {
        for kind in ["mohr-coulomb", "undrained-tresca"] {
            assert!(
                knows_constitutive_kind(kind),
                "{kind} is not in the vocabulary"
            );
        }
        assert!(!knows_constitutive_kind("something-newer"));
    }

    /// Every suggested term has to name a unit, because a parameter without one
    /// is a warning the moment it is saved.
    #[test]
    fn every_suggested_term_has_a_unit() {
        let terms = properties().iter().chain(
            constitutive_kinds()
                .iter()
                .flat_map(|k| k.parameters.iter()),
        );
        for term in terms {
            assert!(!term.units.is_empty(), "{} suggests no unit", term.key);
            assert!(!term.label.is_empty(), "{} has no label", term.key);
        }
    }
}
