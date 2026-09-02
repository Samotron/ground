//! Clone, push, pull and merge between copies of a file.

mod common;

use common::{AUTHOR, TestRepo, model, populated};
use gm_core::store::Repository;
use gm_core::sync::{self, MergeOutcome, PullOutcome, PushOutcome, Relation};
use tempfile::TempDir;

/// Two copies of one project, both already holding the baseline commit.
struct Pair {
    _dir: TempDir,
    alice: Repository,
    bob: Repository,
}

fn cloned_pair() -> Pair {
    let dir = TempDir::new().expect("temp dir");
    let origin_path = dir.path().join("origin.gm");
    let mut origin = common::repo_at(&origin_path);
    populated(&mut origin);
    origin.commit(AUTHOR, "baseline").expect("commit");

    let alice = sync::clone_to(&origin, dir.path().join("alice.gm"), "alice@example.com")
        .expect("clone alice");
    let bob =
        sync::clone_to(&origin, dir.path().join("bob.gm"), "bob@example.com").expect("clone bob");

    Pair {
        _dir: dir,
        alice,
        bob,
    }
}

#[test]
fn a_clone_holds_the_same_history_and_project_id() {
    let TestRepo {
        _dir: dir,
        mut repo,
    } = common::temp_repo();
    populated(&mut repo);
    repo.commit(AUTHOR, "baseline").expect("commit");

    let clone = sync::clone_to(&repo, dir.path().join("clone.gm"), AUTHOR).expect("clone");

    assert_eq!(
        clone.file_id(),
        repo.file_id(),
        "a clone is the same project"
    );
    assert_eq!(clone.head().expect("head"), repo.head().expect("head"));
    assert_eq!(
        clone.working().expect("working"),
        repo.working().expect("working")
    );
    assert_eq!(
        clone.log(None).expect("log").len(),
        repo.log(None).expect("log").len()
    );
    assert!(
        clone.verify().expect("verify").is_empty(),
        "clone must verify"
    );
}

#[test]
fn cloning_transfers_every_object_intact() {
    let TestRepo {
        _dir: dir,
        mut repo,
    } = common::temp_repo();
    populated(&mut repo);
    repo.commit(AUTHOR, "baseline").expect("commit");

    let clone = sync::clone_to(&repo, dir.path().join("clone.gm"), AUTHOR).expect("clone");
    assert_eq!(
        clone.object_stats().expect("stats"),
        repo.object_stats().expect("stats")
    );
}

#[test]
fn syncing_unrelated_projects_is_refused() {
    let a = common::temp_repo();
    let b = common::temp_repo();
    let mut a_repo = a.repo;
    let b_repo = b.repo;

    // Two files made independently are different projects even though their
    // contents may look alike. Grafting one onto the other would be nonsense.
    let err = sync::pull(&mut a_repo, &b_repo).expect_err("should refuse");
    assert!(err.to_string().contains("different projects"), "got: {err}");
}

#[test]
fn pull_fast_forwards_when_only_the_other_side_moved() {
    let mut pair = cloned_pair();

    let mut state = pair.alice.working().expect("working");
    state.models.get_mut("CH-100").expect("CH-100").layers[1].top_level = 78.2;
    pair.alice.write_working(&state).expect("write");
    let alice_head = pair
        .alice
        .commit("alice@example.com", "CH-100 re-log")
        .expect("commit")
        .expect("a commit");

    match sync::pull(&mut pair.bob, &pair.alice).expect("pull") {
        PullOutcome::FastForward { to, .. } => assert_eq!(to, alice_head),
        other => panic!("expected a fast-forward, got {other:?}"),
    }
    assert_eq!(
        pair.bob.working().expect("working").models["CH-100"].layers[1].top_level,
        78.2
    );
}

#[test]
fn pull_is_a_no_op_when_nothing_moved() {
    let mut pair = cloned_pair();
    assert_eq!(
        sync::pull(&mut pair.bob, &pair.alice).expect("pull"),
        PullOutcome::UpToDate
    );
}

#[test]
fn pull_refuses_to_overwrite_uncommitted_work() {
    let mut pair = cloned_pair();

    let mut state = pair.alice.working().expect("working");
    state.models.get_mut("CH-100").expect("CH-100").layers[1].top_level = 78.2;
    pair.alice.write_working(&state).expect("write");
    pair.alice
        .commit("alice@example.com", "re-log")
        .expect("commit");

    // Bob has unsaved work in progress. A fast-forward would silently bin it.
    let mut bob_state = pair.bob.working().expect("working");
    bob_state.models.get_mut("CH-125").expect("CH-125").layers[1].top_level = 80.4;
    pair.bob.write_working(&bob_state).expect("write");

    let err = sync::pull(&mut pair.bob, &pair.alice).expect_err("should refuse");
    assert!(
        matches!(err, gm_core::Error::DirtyWorkingTree),
        "got {err:?}"
    );
}

#[test]
fn push_fast_forwards_the_other_side() {
    let mut pair = cloned_pair();

    let mut state = pair.alice.working().expect("working");
    state.models.get_mut("CH-100").expect("CH-100").layers[1].top_level = 78.2;
    pair.alice.write_working(&state).expect("write");
    let head = pair
        .alice
        .commit("alice@example.com", "re-log")
        .expect("commit")
        .expect("a commit");

    match sync::push(&pair.alice, &mut pair.bob).expect("push") {
        PushOutcome::FastForward { to, .. } => assert_eq!(to, head),
        other => panic!("expected a fast-forward, got {other:?}"),
    }
    assert_eq!(pair.bob.head().expect("head"), Some(head));
    assert_eq!(
        pair.bob.working().expect("working").models["CH-100"].layers[1].top_level,
        78.2
    );
}

#[test]
fn push_is_refused_when_the_other_side_has_commits_we_lack() {
    let mut pair = cloned_pair();

    for (repo, key, level, who) in [
        (&mut pair.alice, "CH-100", 78.2, "alice@example.com"),
        (&mut pair.bob, "CH-125", 80.4, "bob@example.com"),
    ] {
        let mut state = repo.working().expect("working");
        state.models.get_mut(key).expect("model").layers[1].top_level = level;
        repo.write_working(&state).expect("write");
        repo.commit(who, "re-log").expect("commit");
    }

    // Bob's commit is not in Alice's history, so pushing would strand it.
    let err = sync::push(&pair.alice, &mut pair.bob).expect_err("should refuse");
    assert!(err.to_string().contains("pull and merge"), "got: {err}");
}

#[test]
fn edits_to_different_models_merge_automatically() {
    let mut pair = cloned_pair();

    let mut state = pair.alice.working().expect("working");
    state.models.get_mut("CH-100").expect("CH-100").layers[1].top_level = 78.2;
    pair.alice.write_working(&state).expect("write");
    let alice_head = pair
        .alice
        .commit("alice@example.com", "CH-100 deeper")
        .expect("commit")
        .expect("a commit");

    let mut state = pair.bob.working().expect("working");
    state.models.get_mut("CH-125").expect("CH-125").layers[1].top_level = 80.4;
    pair.bob.write_working(&state).expect("write");
    let bob_head = pair
        .bob
        .commit("bob@example.com", "CH-125 shallower")
        .expect("commit")
        .expect("a commit");

    match sync::pull(&mut pair.bob, &pair.alice).expect("pull") {
        PullOutcome::Diverged { ours, theirs, .. } => {
            assert_eq!(ours, bob_head);
            assert_eq!(theirs, alice_head);
        }
        other => panic!("expected divergence, got {other:?}"),
    }

    let merged = match sync::merge(&mut pair.bob, &bob_head, &alice_head, "bob@example.com")
        .expect("merge")
    {
        MergeOutcome::Merged { hash, .. } => hash,
        other => panic!("expected a clean merge, got {other:?}"),
    };

    // Both engineers' work survives.
    let state = pair.bob.working().expect("working");
    assert_eq!(
        state.models["CH-100"].layers[1].top_level, 78.2,
        "Alice's edit"
    );
    assert_eq!(
        state.models["CH-125"].layers[1].top_level, 80.4,
        "Bob's edit"
    );

    let info = pair.bob.commit_info(&merged).expect("info");
    assert_eq!(info.parents.len(), 2, "a merge records both parents");
    assert!(pair.bob.verify().expect("verify").is_empty());
}

#[test]
fn edits_to_the_same_model_conflict_and_write_nothing() {
    let mut pair = cloned_pair();

    let mut state = pair.alice.working().expect("working");
    state.models.get_mut("CH-100").expect("CH-100").layers[1].top_level = 77.0;
    pair.alice.write_working(&state).expect("write");
    let alice_head = pair
        .alice
        .commit("alice@example.com", "CH-100 at 77.0")
        .expect("commit")
        .expect("a commit");

    let mut state = pair.bob.working().expect("working");
    state.models.get_mut("CH-100").expect("CH-100").layers[1].top_level = 81.0;
    pair.bob.write_working(&state).expect("write");
    let bob_head = pair
        .bob
        .commit("bob@example.com", "CH-100 at 81.0")
        .expect("commit")
        .expect("a commit");

    sync::pull(&mut pair.bob, &pair.alice).expect("pull");

    match sync::merge(&mut pair.bob, &bob_head, &alice_head, "bob@example.com").expect("merge") {
        MergeOutcome::Conflicts(result) => {
            assert_eq!(result.conflicts.len(), 1);
            assert_eq!(result.conflicts[0].key, "CH-100");
        }
        other => panic!("expected a conflict, got {other:?}"),
    }

    // The merge must not have touched anything: Bob still has his own value and
    // his HEAD is where it was.
    assert_eq!(pair.bob.head().expect("head"), Some(bob_head));
    assert_eq!(
        pair.bob.working().expect("working").models["CH-100"].layers[1].top_level,
        81.0
    );
    assert!(pair.bob.status().expect("status").is_empty());
}

#[test]
fn a_model_added_on_one_side_only_is_taken_without_conflict() {
    let mut pair = cloned_pair();

    let mut state = pair.alice.working().expect("working");
    state
        .models
        .insert("CH-150".into(), model("CH-150", 81.0, 78.0));
    pair.alice.write_working(&state).expect("write");
    let alice_head = pair
        .alice
        .commit("alice@example.com", "add CH-150")
        .expect("commit")
        .expect("a commit");

    let mut state = pair.bob.working().expect("working");
    state.models.get_mut("CH-125").expect("CH-125").layers[1].top_level = 80.4;
    pair.bob.write_working(&state).expect("write");
    let bob_head = pair
        .bob
        .commit("bob@example.com", "CH-125 re-log")
        .expect("commit")
        .expect("a commit");

    sync::pull(&mut pair.bob, &pair.alice).expect("pull");
    match sync::merge(&mut pair.bob, &bob_head, &alice_head, "bob@example.com").expect("merge") {
        MergeOutcome::Merged { result, .. } => {
            assert!(result.took_theirs.contains(&"CH-150".to_string()));
        }
        other => panic!("expected a clean merge, got {other:?}"),
    }
    assert!(
        pair.bob
            .working()
            .expect("working")
            .models
            .contains_key("CH-150")
    );
}

#[test]
fn merging_something_already_in_our_history_does_nothing() {
    let mut pair = cloned_pair();
    let head = pair.bob.head().expect("head").expect("a head");
    let parent = pair.bob.commit_info(&head).expect("info").parents[0].clone();

    assert_eq!(
        sync::merge(&mut pair.bob, &head, &parent, "bob@example.com").expect("merge"),
        MergeOutcome::AlreadyUpToDate,
        "merging an ancestor must not manufacture an empty merge commit"
    );
    assert_eq!(pair.bob.head().expect("head"), Some(head));
}

#[test]
fn relations_between_histories_are_reported_correctly() {
    let mut pair = cloned_pair();
    let base = pair.bob.head().expect("head").expect("a head");

    let mut state = pair.bob.working().expect("working");
    state.models.get_mut("CH-100").expect("CH-100").layers[1].top_level = 78.0;
    pair.bob.write_working(&state).expect("write");
    let newer = pair
        .bob
        .commit("bob@example.com", "move on")
        .expect("commit")
        .expect("a commit");

    assert_eq!(
        sync::relation(&pair.bob, &base, &base).expect("rel"),
        Relation::Same
    );
    assert_eq!(
        sync::relation(&pair.bob, &base, &newer).expect("rel"),
        Relation::Behind
    );
    assert_eq!(
        sync::relation(&pair.bob, &newer, &base).expect("rel"),
        Relation::Ahead
    );
    assert_eq!(
        sync::merge_base(&pair.bob, &newer, &base).expect("base"),
        Some(base)
    );
}

#[test]
fn a_corrupt_object_cannot_be_laundered_through_a_transfer() {
    let TestRepo {
        _dir: dir,
        mut repo,
    } = common::temp_repo();
    populated(&mut repo);
    repo.commit(AUTHOR, "baseline").expect("commit");

    // Tamper with a stored document, leaving its hash claiming the old content.
    let hash: String = repo
        .connection()
        .query_row(
            "SELECT hash FROM gm_blob WHERE hash NOT IN (SELECT hash FROM gm_commit) LIMIT 1",
            [],
            |r| r.get(0),
        )
        .expect("a document blob");
    repo.connection()
        .execute(
            "UPDATE gm_blob SET content = ?1 WHERE hash = ?2",
            rusqlite::params![br#"{"tampered":true}"#.to_vec(), hash],
        )
        .expect("tamper");

    assert!(
        !repo.verify().expect("verify").is_empty(),
        "verify should notice"
    );

    let err = sync::clone_to(&repo, dir.path().join("clone.gm"), AUTHOR)
        .expect_err("a clone must not accept a corrupt object");
    assert!(
        matches!(err, gm_core::Error::CorruptObject { .. }),
        "got {err:?}"
    );
}
