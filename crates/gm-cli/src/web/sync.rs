//! The sync endpoints.
//!
//! Four of them, and no session state:
//!
//! | | |
//! |---|---|
//! | `GET /sync/info` | who this file is, and where its head is |
//! | `GET /sync/commits` | every commit hash it holds, one per line |
//! | `POST /sync/bundle` | body: the caller's commit hashes; returns what it lacks |
//! | `POST /sync/push` | body: a bundle; header `X-GM-Head`: the new head |
//!
//! Serving a bundle discloses nothing that the UI pages do not already show, so
//! it needs no extra permission. Accepting a push is a write to someone else's
//! file, so it is off unless `--allow-push` says otherwise.

use super::{Options, Reply};
use anyhow::{Context, Result};
use gm_core::store::{Repository, short_hash};
use gm_core::sync::{self, Relation};
use gm_core::wire::{self, RemoteInfo};
use std::collections::BTreeSet;
use std::path::Path;

pub const HEAD_HEADER: &str = "X-GM-Head";

/// Handle a `/sync/...` request, or return `None` if this is not one.
pub fn handle(
    path: &Path,
    opts: &Options,
    method: &str,
    route: &str,
    body: &[u8],
    head_header: Option<&str>,
) -> Option<Result<Reply>> {
    let endpoint = route.strip_prefix("/sync/")?;
    Some(dispatch(path, opts, method, endpoint, body, head_header))
}

fn dispatch(
    path: &Path,
    opts: &Options,
    method: &str,
    endpoint: &str,
    body: &[u8],
    head_header: Option<&str>,
) -> Result<Reply> {
    let repo = Repository::open(path).context("reopening the ground-model file")?;

    match (method, endpoint) {
        ("GET", "info") => {
            let info = RemoteInfo {
                protocol: RemoteInfo::PROTOCOL,
                file_id: repo.file_id().to_string(),
                schema_version: gm_core::schema::SCHEMA_VERSION.to_string(),
                head: repo.head()?,
                name: repo.working()?.file_metadata.name,
                accepts_push: opts.allow_push,
            };
            Ok(Reply::json(serde_json::to_string_pretty(&info)?))
        }

        ("GET", "commits") => {
            let mut out = sync::known_commits(&repo)?.join("\n");
            out.push('\n');
            Ok(Reply::text(200, &out))
        }

        ("POST", "bundle") => {
            let Some(head) = repo.head()? else {
                return Ok(Reply::bundle(&[]));
            };
            let peer_has = parse_hashes(body);
            let objects = sync::bundle_for(&repo, &head, &peer_has)?;
            Ok(Reply::bundle(&objects))
        }

        ("POST", "push") => accept_push(repo, opts, body, head_header),

        // A wrong method on a real endpoint deserves a better answer than 404.
        ("GET", "bundle" | "push") | ("POST", "info" | "commits") => Ok(Reply::text(
            405,
            &format!("/sync/{endpoint} does not accept {method}"),
        )),
        _ => Ok(Reply::text(
            404,
            &format!("no such endpoint: /sync/{endpoint}"),
        )),
    }
}

fn accept_push(
    mut repo: Repository,
    opts: &Options,
    body: &[u8],
    head_header: Option<&str>,
) -> Result<Reply> {
    if !opts.allow_push {
        return Ok(Reply::text(
            403,
            "this remote is read-only; it was not started with --allow-push",
        ));
    }

    let Some(new_head) = head_header else {
        return Ok(Reply::text(
            400,
            &format!("missing {HEAD_HEADER} header: a push must say what its new head is"),
        ));
    };

    // Never overwrite work someone has in progress here. They may be the only
    // person who knows what it was.
    if !repo.status()?.is_empty() {
        return Ok(Reply::text(
            409,
            "this remote has uncommitted changes in its working tree; \
             pushing would overwrite them",
        ));
    }

    let objects = match wire::decode_bundle(body) {
        Ok(objects) => objects,
        Err(err) => return Ok(Reply::text(400, &format!("{err}"))),
    };

    // Objects are verified against their own hashes as they land, so a bad
    // bundle cannot poison the store even though we accept it before deciding
    // what it means.
    let report = sync::apply_bundle(&repo, &objects)?;
    if let Err(err) = sync::check_reachable(&repo, new_head) {
        return Ok(Reply::text(400, &format!("{err}")));
    }

    let outcome = match repo.head()? {
        None => "created",
        Some(current) => match sync::relation(&repo, &current, new_head)? {
            Relation::Same | Relation::Ahead => {
                return Ok(Reply::json(serde_json::to_string_pretty(
                    &serde_json::json!({
                        "outcome": "up-to-date",
                        "head": current,
                        "received": report.objects,
                    }),
                )?));
            }
            Relation::Behind => "fast-forward",
            // Refused, not forced: the pusher must merge, because only they can
            // see both sides.
            Relation::Diverged { .. } => {
                return Ok(Reply::text(
                    409,
                    "this remote has commits you do not: pull and merge before pushing",
                ));
            }
        },
    };

    repo.set_head(new_head)?;
    let state = repo.state_at(new_head)?;
    repo.write_working(&state)?;

    Ok(Reply::json(serde_json::to_string_pretty(
        &serde_json::json!({
            "outcome": outcome,
            "head": new_head,
            "shortHead": short_hash(new_head),
            "received": report.objects,
            "commits": report.commits,
        }),
    )?))
}

/// One hash per line, blanks ignored. Deliberately forgiving: the list is a
/// hint about what not to send, so a stray line costs nothing.
fn parse_hashes(body: &[u8]) -> BTreeSet<String> {
    String::from_utf8_lossy(body)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_lists_tolerate_blank_and_padded_lines() {
        let parsed = parse_hashes(b"sha256-aa\n\n  sha256-bb  \n\n");
        assert_eq!(
            parsed,
            BTreeSet::from(["sha256-aa".to_string(), "sha256-bb".to_string()])
        );
    }

    #[test]
    fn an_empty_list_is_not_an_error() {
        assert!(parse_hashes(b"").is_empty());
    }
}
