//! End-to-end tests over a real file on disk.
//!
//! These exercise the properties the format promises rather than the internals:
//! that a revision round-trips exactly, that unchanged content is not stored
//! twice, that history is reachable, and that a file written by this build can
//! be read back by it byte for byte.

mod common;

use common::{AUTHOR, TestRepo, populated};
use gm_core::exchange::Exchange;
use gm_core::store::{ChangeKind, Repository};
use gm_core::validate;
use gm_core::{FileMetadata, canon};
use std::collections::BTreeSet;
use tempfile::TempDir;

#[test]
fn a_committed_revision_reads_back_exactly() {
    let TestRepo { _dir, mut repo } = common::temp_repo();
    let written = populated(&mut repo);
    let hash = repo
        .commit(AUTHOR, "initial models")
        .expect("commit")
        .expect("a commit");

    let read_back = repo.state_at(&hash).expect("state_at");
    assert_eq!(written, read_back);
    // And the working tree, which was re-materialised from that commit, agrees.
    assert_eq!(repo.working().expect("working"), read_back);
}

#[test]
fn committing_twice_with_no_changes_does_nothing() {
    let TestRepo { _dir, mut repo } = common::temp_repo();
    populated(&mut repo);
    repo.commit(AUTHOR, "first")
        .expect("commit")
        .expect("a commit");
    let second = repo.commit(AUTHOR, "again").expect("commit");
    assert_eq!(
        second, None,
        "an unchanged working tree should not make a revision"
    );
}

#[test]
fn unchanged_documents_are_stored_once_across_revisions() {
    let TestRepo { _dir, mut repo } = common::temp_repo();
    populated(&mut repo);
    repo.commit(AUTHOR, "first").expect("commit");
    let (blobs_before, _, _) = repo.object_stats().expect("stats");

    // Move one layer boundary. Both materials, the file metadata and the other
    // model are untouched, so only the changed model and the new manifest
    // should become new objects.
    let mut state = repo.working().expect("working");
    state.models.get_mut("CH-100").expect("CH-100").layers[1].top_level = 79.1;
    repo.write_working(&state).expect("write");
    repo.commit(AUTHOR, "adjust boundary").expect("commit");

    let (blobs_after, _, _) = repo.object_stats().expect("stats");
    assert_eq!(
        blobs_after - blobs_before,
        2,
        "expected one new model document and one new manifest, got {} new objects",
        blobs_after - blobs_before
    );
}

#[test]
fn status_reports_what_changed_in_the_working_tree() {
    let TestRepo { _dir, mut repo } = common::temp_repo();
    populated(&mut repo);
    repo.commit(AUTHOR, "first").expect("commit");
    assert!(repo.status().expect("status").is_empty());

    let mut state = repo.working().expect("working");
    state
        .models
        .get_mut("CH-100")
        .expect("CH-100")
        .surface_level = Some(83.0);
    state.models.get_mut("CH-100").expect("CH-100").layers[0].top_level = 83.0;
    state.models.remove("CH-125");
    repo.write_working(&state).expect("write");

    let changes = repo.status().expect("status");
    let summary: BTreeSet<(String, ChangeKind)> =
        changes.iter().map(|c| (c.key.clone(), c.change)).collect();
    assert!(summary.contains(&("CH-100".to_string(), ChangeKind::Modified)));
    assert!(summary.contains(&("CH-125".to_string(), ChangeKind::Deleted)));
    assert_eq!(changes.len(), 2);
}

#[test]
fn checkout_restores_an_earlier_revision() {
    let TestRepo { _dir, mut repo } = common::temp_repo();
    populated(&mut repo);
    let first = repo
        .commit(AUTHOR, "first")
        .expect("commit")
        .expect("a commit");

    let mut state = repo.working().expect("working");
    state.models.get_mut("CH-100").expect("CH-100").layers[1].top_level = 75.0;
    repo.write_working(&state).expect("write");
    repo.commit(AUTHOR, "second").expect("commit");

    repo.checkout(&first, false).expect("checkout");
    let restored = repo.working().expect("working");
    assert_eq!(restored.models["CH-100"].layers[1].top_level, 79.5);
    assert_eq!(repo.head().expect("head").as_deref(), Some(first.as_str()));
}

#[test]
fn checkout_refuses_to_discard_uncommitted_work() {
    let TestRepo { _dir, mut repo } = common::temp_repo();
    populated(&mut repo);
    let first = repo
        .commit(AUTHOR, "first")
        .expect("commit")
        .expect("a commit");

    let mut state = repo.working().expect("working");
    state.models.get_mut("CH-100").expect("CH-100").layers[1].top_level = 75.0;
    repo.write_working(&state).expect("write");

    assert!(
        repo.checkout(&first, false).is_err(),
        "should refuse while dirty"
    );
    assert!(
        repo.checkout(&first, true).is_ok(),
        "--force should proceed"
    );
}

#[test]
fn history_is_reachable_and_ordered() {
    let TestRepo { _dir, mut repo } = common::temp_repo();
    populated(&mut repo);
    repo.commit(AUTHOR, "first").expect("commit");

    let mut state = repo.working().expect("working");
    state.models.get_mut("CH-100").expect("CH-100").layers[1].top_level = 78.0;
    repo.write_working(&state).expect("write");
    repo.commit(AUTHOR, "second").expect("commit");

    let log = repo.log(None).expect("log");
    assert_eq!(log.len(), 3, "root, first and second");
    assert_eq!(log[0].message, "second");
    assert_eq!(log[2].message, "Initialise ground-model file");
    assert_eq!(log[0].parents.len(), 1);
}

#[test]
fn a_reopened_file_sees_the_same_history() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("reopen.gm");
    {
        let mut repo =
            Repository::create(&path, FileMetadata::new("Reopen"), AUTHOR).expect("create");
        populated(&mut repo);
        repo.commit(AUTHOR, "first").expect("commit");
    }
    let repo = Repository::open(&path).expect("reopen");
    assert_eq!(repo.log(None).expect("log").len(), 2);
    assert_eq!(repo.working().expect("working").models.len(), 2);
    assert!(repo.verify().expect("verify").is_empty());
}

#[test]
fn opening_something_that_is_not_a_ground_model_file_fails_clearly() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("plain.db");
    rusqlite::Connection::open(&path)
        .expect("sqlite")
        .execute_batch("CREATE TABLE t (a);")
        .expect("create table");

    let err = match Repository::open(&path) {
        Err(err) => err,
        Ok(_) => panic!("should reject a plain sqlite file"),
    };
    assert!(
        matches!(err, gm_core::Error::NotARepository(_)),
        "expected NotARepository, got {err:?}"
    );
}

#[test]
fn the_exchange_document_round_trips_without_loss() {
    let TestRepo { _dir, mut repo } = common::temp_repo();
    let state = populated(&mut repo);
    repo.commit(AUTHOR, "first").expect("commit");

    let doc = Exchange::from_state(&state, None);
    let json = doc.to_json_pretty().expect("to json");
    let back = Exchange::from_json(&json).expect("from json").into_state();

    assert_eq!(state, back);
}

#[test]
fn exported_json_is_byte_stable() {
    let TestRepo { _dir, mut repo } = common::temp_repo();
    let state = populated(&mut repo);
    let a = Exchange::from_state(&state, None)
        .to_json_pretty()
        .expect("json");
    let b = Exchange::from_state(&state, None)
        .to_json_pretty()
        .expect("json");
    assert_eq!(
        a, b,
        "the same state must export identically, or diffs are noise"
    );
}

#[test]
fn the_same_model_hashes_the_same_in_two_separate_files() {
    // This is the property clone/push/pull will rest on: content identity is a
    // function of the document alone, never of which file it lives in.
    let TestRepo {
        _dir: _d1,
        repo: mut a,
    } = common::temp_repo();
    let TestRepo {
        _dir: _d2,
        repo: mut b,
    } = common::temp_repo();
    populated(&mut a);
    populated(&mut b);

    let ma = serde_json::to_value(&a.working().expect("a").models["CH-100"]).expect("json");
    let mb = serde_json::to_value(&b.working().expect("b").models["CH-100"]).expect("json");
    assert_eq!(
        canon::hash_value(&ma).expect("hash"),
        canon::hash_value(&mb).expect("hash")
    );
}

#[test]
fn validation_accepts_a_sound_model_and_rejects_an_inverted_one() {
    let TestRepo { _dir, mut repo } = common::temp_repo();
    let mut state = populated(&mut repo);
    let issues = validate::validate_state(&state);
    let (errors, _) = validate::count(&issues);
    assert_eq!(
        errors, 0,
        "sound models should not produce errors: {issues:#?}"
    );

    // Put the clay above the made ground.
    state.models.get_mut("CH-100").expect("CH-100").layers[1].top_level = 90.0;
    let issues = validate::validate_state(&state);
    let (errors, _) = validate::count(&issues);
    assert!(errors > 0, "an inverted succession must be an error");
    assert!(
        issues.iter().any(|i| i.field_path.contains("topLevel")),
        "the error should point at the layer top"
    );
}

#[test]
fn a_layer_referencing_an_unknown_material_is_an_error() {
    let TestRepo { _dir, mut repo } = common::temp_repo();
    let mut state = populated(&mut repo);
    state.models.get_mut("CH-100").expect("CH-100").layers[1].material_key = "THAMES_GRAVEL".into();

    let issues = validate::validate_state(&state);
    assert!(
        issues
            .iter()
            .any(|i| i.severity == validate::Severity::Error
                && i.message.contains("THAMES_GRAVEL")),
        "expected an error naming the missing material, got {issues:#?}"
    );
}

#[test]
fn a_base_level_above_the_deepest_layer_is_an_error() {
    let TestRepo { _dir, mut repo } = common::temp_repo();
    let mut state = populated(&mut repo);
    // Deepest layer top is 79.5; a base of 80.0 would be above it.
    state.models.get_mut("CH-100").expect("CH-100").base_level = Some(80.0);

    let issues = validate::validate_state(&state);
    assert!(
        issues
            .iter()
            .any(|i| i.severity == validate::Severity::Error && i.field_path == "base_level"),
        "expected a base_level error, got {issues:#?}"
    );
}

#[test]
fn writing_a_layer_with_an_undefined_material_is_refused() {
    let TestRepo { _dir, mut repo } = common::temp_repo();
    let mut state = populated(&mut repo);
    state.models.get_mut("CH-100").expect("CH-100").layers[0].material_key = "NOPE".into();

    let err = repo.write_working(&state).expect_err("should refuse");
    assert!(
        err.to_string().contains("NOPE"),
        "the message should name the material, got: {err}"
    );
}
