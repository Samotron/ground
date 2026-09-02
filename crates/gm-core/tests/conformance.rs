//! Cross-implementation conformance.
//!
//! `assets/conformance.json` is read by this suite and by the browser editor's
//! suite. Both must agree on it. Two implementations of the same rules drift
//! quietly, and the way you find out is an engineer saving a file in the editor
//! that `gm commit` then refuses — so the agreement is pinned rather than
//! assumed.
//!
//! When a rule legitimately changes, regenerate the fixture and change both
//! implementations in the same commit. A red test here means they disagree,
//! not that the fixture is stale.

use gm_core::exchange::Exchange;
use gm_core::validate;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    hues: BTreeMap<String, u32>,
    document: serde_json::Value,
    expected_issues: Vec<ExpectedIssue>,
}

#[derive(Deserialize, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(rename_all = "camelCase")]
struct ExpectedIssue {
    severity: String,
    #[serde(default)]
    model_key: Option<String>,
    field_path: String,
}

fn fixture() -> Fixture {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/conformance.json");
    let text = std::fs::read_to_string(path).expect("assets/conformance.json should exist");
    serde_json::from_str(&text).expect("the fixture should parse")
}

/// Mirror of the hue rule in the CLI's section drawing. Kept here rather than
/// imported because the drawing lives in the binary crate, and this test is
/// about the *rule* agreeing across implementations, not about which module
/// happens to hold it.
fn hue(key: &str) -> u32 {
    let mut hash: u32 = 2_166_136_261;
    for byte in key.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    (hash.wrapping_mul(137) % 360).max(1)
}

#[test]
fn material_colours_match_the_agreed_vectors() {
    for (key, expected) in fixture().hues {
        assert_eq!(
            hue(&key),
            expected,
            "hue for {key:?} disagrees with the shared fixture"
        );
    }
}

#[test]
fn validation_reports_exactly_the_agreed_issues() {
    let fixture = fixture();
    let state = Exchange::from_json(&fixture.document.to_string())
        .expect("the fixture document should be a valid interchange document")
        .into_state();

    let mut actual: Vec<ExpectedIssue> = validate::validate_state(&state)
        .into_iter()
        .map(|issue| ExpectedIssue {
            severity: issue.severity.as_str().to_string(),
            model_key: issue.model_key,
            field_path: issue.field_path,
        })
        .collect();
    actual.sort();

    let mut expected = fixture.expected_issues;
    expected.sort();

    assert_eq!(
        actual, expected,
        "\nvalidation has drifted from assets/conformance.json.\n\
         If the rule change is intended, regenerate the fixture and update the\n\
         browser editor in the same commit."
    );
}

#[test]
fn the_fixture_document_survives_a_round_trip() {
    // The editor reads and writes this shape too, so it has to be stable.
    let fixture = fixture();
    let state = Exchange::from_json(&fixture.document.to_string())
        .expect("valid document")
        .into_state();
    let json = Exchange::from_state(&state, None)
        .to_json_pretty()
        .expect("json");
    let back = Exchange::from_json(&json).expect("re-read").into_state();
    assert_eq!(state, back);
}
