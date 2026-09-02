//! Synchronising two copies of a ground-model file.
//!
//! Because a commit is itself a blob, and every blob is named by the hash of
//! its content, syncing is just "send me the objects I do not have". There is no
//! delta encoding, no rename detection and no negotiation protocol: the object
//! graph already says exactly what is missing.
//!
//! Divergence is resolved by a three-way merge over documents, keyed by
//! `(kind, key)`. Two engineers working on different chainages of the same route
//! therefore merge cleanly and automatically; two engineers who both re-logged
//! the same borehole get a conflict, which is correct, because only one of them
//! can be right and the tool cannot know which.

use crate::error::{Error, Result, invalid};
use crate::store::{Repository, State, short_hash};
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// How two histories relate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Relation {
    /// Both sides are at the same commit.
    Same,
    /// Ours is an ancestor of theirs: we can fast-forward.
    Behind,
    /// Theirs is an ancestor of ours: they have nothing new.
    Ahead,
    /// Both sides have commits the other lacks.
    Diverged { base: Option<String> },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransferReport {
    pub commits: usize,
    pub objects: usize,
}

/// True when `ancestor` is reachable from `descendant` by following parents.
pub fn is_ancestor(repo: &Repository, ancestor: &str, descendant: &str) -> Result<bool> {
    Ok(repo
        .ancestry(descendant)?
        .iter()
        .any(|c| c.hash == ancestor))
}

/// The best common ancestor of two commits.
///
/// [`Repository::ancestry`] returns descendants before ancestors, so the first
/// commit of `ours` that also appears in the ancestry of `theirs` is a lowest
/// common ancestor. Both histories must already be present locally, which is why
/// pull fetches objects before deciding what to do with them.
pub fn merge_base(repo: &Repository, ours: &str, theirs: &str) -> Result<Option<String>> {
    let theirs_set: HashSet<String> = repo.ancestry(theirs)?.into_iter().map(|c| c.hash).collect();
    Ok(repo
        .ancestry(ours)?
        .into_iter()
        .find(|c| theirs_set.contains(&c.hash))
        .map(|c| c.hash))
}

pub fn relation(repo: &Repository, ours: &str, theirs: &str) -> Result<Relation> {
    if ours == theirs {
        return Ok(Relation::Same);
    }
    if is_ancestor(repo, ours, theirs)? {
        return Ok(Relation::Behind);
    }
    if is_ancestor(repo, theirs, ours)? {
        return Ok(Relation::Ahead);
    }
    Ok(Relation::Diverged {
        base: merge_base(repo, ours, theirs)?,
    })
}

/// Copy every object reachable from `tip` that `to` is missing.
///
/// Blobs are copied as raw bytes rather than re-serialised, so a hash that was
/// valid in the source stays valid in the destination even across builds, and
/// [`Repository::put_blob_raw`] rejects anything that does not hash to its name.
pub fn transfer(from: &Repository, to: &Repository, tip: &str) -> Result<TransferReport> {
    let mut report = TransferReport::default();

    // Oldest first, so a commit's parents already exist when it lands. Nothing
    // depends on this for correctness — parent hashes carry no foreign key —
    // but it keeps a partially-transferred file readable.
    let mut chain = from.ancestry(tip)?;
    chain.reverse();

    for info in chain {
        if to.has_commit(&info.hash)? {
            continue;
        }
        let manifest = from.manifest(&info.hash)?;

        for entry in &manifest.entries {
            if !to.has_blob(&entry.blob)? {
                let bytes = from.get_blob_bytes(&entry.blob)?;
                to.put_blob_raw(&entry.blob, &bytes)?;
                report.objects += 1;
            }
        }

        let bytes = from.get_blob_bytes(&info.hash)?;
        to.put_blob_raw(&info.hash, &bytes)?;
        to.record_commit(&manifest)?;
        report.commits += 1;
        report.objects += 1;
    }

    Ok(report)
}

/// Make a new file holding everything in `source`.
///
/// The clone keeps the source's `file_id`, because that id names the project
/// rather than the copy. Two clones that share an id are two views of one thing
/// and may sync; two files with different ids are unrelated, and refusing to
/// sync them is what stops a stray pull from grafting one project onto another.
pub fn clone_to(
    source: &Repository,
    dest_path: impl AsRef<std::path::Path>,
    author: &str,
) -> Result<Repository> {
    let mut dest = Repository::create_empty(dest_path, source.file_id(), author)?;

    if let Some(head) = source.head()? {
        transfer(source, &dest, &head)?;
        dest.set_head(&head)?;
        let state = dest.state_at(&head)?;
        dest.write_working(&state)?;
    }

    for (name, url) in source.remotes()? {
        dest.set_remote(&name, &url)?;
    }
    Ok(dest)
}

fn check_same_project(local: &Repository, remote: &Repository) -> Result<()> {
    if local.file_id() != remote.file_id() {
        return Err(invalid(format!(
            "these are different projects: {} and {}. Use `gm import` to bring \
             models across from an unrelated file.",
            local.file_id(),
            remote.file_id()
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullOutcome {
    UpToDate,
    FastForward {
        to: String,
        transferred: TransferReport,
    },
    /// Objects were fetched, but the two histories have both moved on. Nothing
    /// has been changed locally; the caller decides whether to merge.
    Diverged {
        ours: String,
        theirs: String,
        base: Option<String>,
        transferred: TransferReport,
    },
}

/// Fetch from `remote` and fast-forward if we can.
///
/// A pull never rewrites uncommitted work: if the working tree is dirty and the
/// pull would move HEAD, it stops instead. Losing an engineer's unsaved
/// re-interpretation to a background sync would be unforgivable.
pub fn pull(local: &mut Repository, remote: &Repository) -> Result<PullOutcome> {
    check_same_project(local, remote)?;

    let Some(theirs) = remote.head()? else {
        return Ok(PullOutcome::UpToDate);
    };
    let Some(ours) = local.head()? else {
        let transferred = transfer(remote, local, &theirs)?;
        local.set_head(&theirs)?;
        let state = local.state_at(&theirs)?;
        local.write_working(&state)?;
        return Ok(PullOutcome::FastForward {
            to: theirs,
            transferred,
        });
    };

    if ours == theirs {
        return Ok(PullOutcome::UpToDate);
    }

    let transferred = transfer(remote, local, &theirs)?;

    match relation(local, &ours, &theirs)? {
        Relation::Same | Relation::Ahead => Ok(PullOutcome::UpToDate),
        Relation::Behind => {
            if !local.status()?.is_empty() {
                return Err(Error::DirtyWorkingTree);
            }
            local.set_head(&theirs)?;
            let state = local.state_at(&theirs)?;
            local.write_working(&state)?;
            Ok(PullOutcome::FastForward {
                to: theirs,
                transferred,
            })
        }
        Relation::Diverged { base } => Ok(PullOutcome::Diverged {
            ours,
            theirs,
            base,
            transferred,
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushOutcome {
    UpToDate,
    FastForward {
        to: String,
        transferred: TransferReport,
    },
}

/// Send local commits to `remote` and move its HEAD, if that is a fast-forward.
///
/// A push that is not a fast-forward is refused rather than forced. The remote
/// may be someone else's working copy, and overwriting their HEAD would discard
/// work they can no longer see.
pub fn push(local: &Repository, remote: &mut Repository) -> Result<PushOutcome> {
    check_same_project(local, remote)?;

    let Some(ours) = local.head()? else {
        return Ok(PushOutcome::UpToDate);
    };

    match remote.head()? {
        None => {}
        Some(theirs) if theirs == ours => return Ok(PushOutcome::UpToDate),
        Some(theirs) => {
            // The remote's history may contain commits we have never seen, so
            // ask the side that actually holds both. We have theirs only if we
            // have previously pulled.
            let known = local.has_commit(&theirs)?;
            if !known || !is_ancestor(local, &theirs, &ours)? {
                return Err(invalid(
                    "the remote has commits you do not: pull and merge before pushing",
                ));
            }
        }
    }

    if !remote.status()?.is_empty() {
        return Err(invalid(
            "the remote has uncommitted changes in its working tree; \
             pushing would overwrite them",
        ));
    }

    let transferred = transfer(local, remote, &ours)?;
    remote.set_head(&ours)?;
    let state = remote.state_at(&ours)?;
    remote.write_working(&state)?;
    Ok(PushOutcome::FastForward {
        to: ours,
        transferred,
    })
}

/// A document both sides changed, differently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub kind: String,
    pub key: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeResult {
    pub conflicts: Vec<Conflict>,
    /// Documents taken from the other side.
    pub took_theirs: Vec<String>,
    /// Documents where our side changed and theirs did not.
    pub kept_ours: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeOutcome {
    /// Their revision is already in our history; there is nothing to merge.
    AlreadyUpToDate,
    /// Ours is an ancestor of theirs, so no merge commit is needed.
    FastForward { to: String },
    /// A genuine merge, recorded with both revisions as parents.
    Merged { hash: String, result: MergeResult },
    /// Both sides changed the same documents. Nothing has been written.
    Conflicts(MergeResult),
}

/// Three-way merge of two diverged revisions.
///
/// On success the merged state is written to the working tree and recorded as a
/// commit with both revisions as parents. On conflict nothing is written: the
/// caller is told which documents disagree, and resolving them is an
/// engineering judgement, not a text merge.
pub fn merge(
    repo: &mut Repository,
    ours: &str,
    theirs: &str,
    author: &str,
) -> Result<MergeOutcome> {
    if !repo.status()?.is_empty() {
        return Err(Error::DirtyWorkingTree);
    }

    // Merging something we already contain, or something that simply supersedes
    // us, should not manufacture a merge commit that records no decision.
    match relation(repo, ours, theirs)? {
        Relation::Same | Relation::Ahead => return Ok(MergeOutcome::AlreadyUpToDate),
        Relation::Behind => {
            repo.checkout(theirs, false)?;
            return Ok(MergeOutcome::FastForward {
                to: theirs.to_string(),
            });
        }
        Relation::Diverged { .. } => {}
    }

    let base_hash = merge_base(repo, ours, theirs)?;
    let base = match &base_hash {
        Some(hash) => Some(repo.state_at(hash)?),
        None => None,
    };
    let our_state = repo.state_at(ours)?;
    let their_state = repo.state_at(theirs)?;

    let mut result = MergeResult::default();

    let file_metadata = match merge_one(
        base.as_ref().map(|b| &b.file_metadata),
        &our_state.file_metadata,
        &their_state.file_metadata,
    ) {
        Merged::Value(v) => v.clone(),
        Merged::Conflict => {
            result.conflicts.push(Conflict {
                kind: "file_metadata".into(),
                key: "file".into(),
            });
            our_state.file_metadata.clone()
        }
    };

    let materials = merge_maps(
        base.as_ref().map(|b| &b.materials),
        &our_state.materials,
        &their_state.materials,
        "material",
        &mut result,
    );
    let models = merge_maps(
        base.as_ref().map(|b| &b.models),
        &our_state.models,
        &their_state.models,
        "ground_model",
        &mut result,
    );

    if !result.conflicts.is_empty() {
        return Ok(MergeOutcome::Conflicts(result));
    }

    let merged = State {
        file_metadata,
        materials,
        models,
    };
    repo.write_working(&merged)?;

    let hash = repo.commit_with_parents(
        vec![ours.to_string(), theirs.to_string()],
        author,
        &format!("Merge {} into {}", short_hash(theirs), short_hash(ours)),
    )?;
    Ok(MergeOutcome::Merged { hash, result })
}

enum Merged<'a, T> {
    Value(&'a T),
    Conflict,
}

/// Standard three-way rule: a side that did not change from the base loses to
/// the side that did; if both changed to the same thing that is agreement, not
/// a conflict; if both changed to different things, only a person can decide.
fn merge_one<'a, T: PartialEq>(base: Option<&'a T>, ours: &'a T, theirs: &'a T) -> Merged<'a, T> {
    if ours == theirs {
        return Merged::Value(ours);
    }
    match base {
        Some(base) if base == ours => Merged::Value(theirs),
        Some(base) if base == theirs => Merged::Value(ours),
        _ => Merged::Conflict,
    }
}

fn merge_maps<T: PartialEq + Clone>(
    base: Option<&BTreeMap<String, T>>,
    ours: &BTreeMap<String, T>,
    theirs: &BTreeMap<String, T>,
    kind: &str,
    result: &mut MergeResult,
) -> BTreeMap<String, T> {
    let keys: BTreeSet<&String> = ours.keys().chain(theirs.keys()).collect();
    let mut out = BTreeMap::new();

    for key in keys {
        let base_doc = base.and_then(|b| b.get(key));
        match (ours.get(key), theirs.get(key)) {
            (Some(ours), Some(theirs)) => match merge_one(base_doc, ours, theirs) {
                Merged::Value(v) => {
                    if v == theirs && v != ours {
                        result.took_theirs.push(key.clone());
                    } else if v == ours && v != theirs {
                        result.kept_ours.push(key.clone());
                    }
                    out.insert(key.clone(), v.clone());
                }
                Merged::Conflict => result.conflicts.push(Conflict {
                    kind: kind.to_string(),
                    key: key.clone(),
                }),
            },
            // Present on one side only. If it was in the base, the other side
            // deleted it deliberately; if it was not, the other side added it.
            (Some(ours), None) => {
                if base_doc.is_some() {
                    if base_doc == Some(ours) {
                        // We did not touch it and they deleted it: accept.
                    } else {
                        // We edited what they deleted. Only a person can say
                        // whether the edit or the deletion was right.
                        result.conflicts.push(Conflict {
                            kind: kind.to_string(),
                            key: key.clone(),
                        });
                    }
                } else {
                    out.insert(key.clone(), ours.clone());
                }
            }
            (None, Some(theirs)) => {
                if base_doc.is_some() {
                    if base_doc == Some(theirs) {
                        // They did not touch it and we deleted it: accept.
                    } else {
                        result.conflicts.push(Conflict {
                            kind: kind.to_string(),
                            key: key.clone(),
                        });
                    }
                } else {
                    result.took_theirs.push(key.clone());
                    out.insert(key.clone(), theirs.clone());
                }
            }
            (None, None) => unreachable!("key came from one of the two maps"),
        }
    }
    out
}
