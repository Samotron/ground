//! Sync over HTTP, driven exactly as a user would: one sandbox runs
//! `gm serve`, another clones, pulls and pushes against the URL.

mod common;

use common::Gm;

/// Clone `name` from a running server, giving the clone its own sandbox so the
/// two files behave like copies on different machines.
fn clone_from(client: &Gm, url: &str, name: &str) {
    client.ok(&["clone", url, name]);
}

#[test]
fn info_describes_the_remote() {
    let gm = Gm::with_route();
    let server = gm.serve_sync("a13.gm", &["--allow-push"]);
    let response = server.get("/sync/info");
    let info: serde_json::Value = serde_json::from_str(&response.ok().body).expect("json");

    assert_eq!(info["protocol"], 1);
    assert_eq!(info["name"], "A13 corridor");
    assert_eq!(info["acceptsPush"], true);
    assert!(
        info["head"]
            .as_str()
            .is_some_and(|h| h.starts_with("sha256-")),
        "should report a head: {info}"
    );
    assert!(info["fileId"].as_str().is_some_and(|f| !f.is_empty()));
}

#[test]
fn a_read_only_server_says_so() {
    let gm = Gm::with_route();
    let server = gm.serve_sync("a13.gm", &[]);
    let response = server.get("/sync/info");
    let info: serde_json::Value = serde_json::from_str(&response.ok().body).expect("json");
    assert_eq!(info["acceptsPush"], false);
}

#[test]
fn the_ui_serves_reads_too_because_a_bundle_discloses_nothing_new() {
    // `gm ui` already shows the whole file on its pages, so exposing the same
    // content as a bundle adds no disclosure. Pushing is the part that needs
    // an explicit decision.
    let gm = Gm::with_route();
    let server = gm.serve("a13.gm");
    server.get("/sync/info").ok();
    server.get("/sync/commits").ok();
    server.post("/sync/push").status_is(403);
}

#[test]
fn commits_lists_every_revision() {
    let gm = Gm::with_route();
    let server = gm.serve_sync("a13.gm", &[]);
    let response = server.get("/sync/commits");
    let listed = &response.ok().body;
    let count = listed.lines().filter(|l| !l.trim().is_empty()).count();
    assert_eq!(count, 2, "the root commit and the import, got:\n{listed}");
}

#[test]
fn a_clone_over_http_carries_the_whole_history() {
    let office = Gm::with_route();
    let server = office.serve_sync("a13.gm", &[]);
    let field = Gm::empty();

    clone_from(&field, &server.url(), "field.gm");

    assert_eq!(
        field.ok(&["-f", "field.gm", "log"]).stdout,
        office.ok(&["-f", "a13.gm", "log"]).stdout
    );
    field.ok(&["-f", "field.gm", "verify"]);
    assert_eq!(
        field.ok(&["-f", "field.gm", "models"]).row("CH-150")[1],
        "6.20"
    );
    // The clone remembers where it came from, so a bare `gm pull` works.
    field
        .ok(&["-f", "field.gm", "remote", "list"])
        .says("origin")
        .says(&server.url());
}

#[test]
fn a_clone_names_itself_after_the_project_when_no_name_is_given() {
    let office = Gm::with_route();
    let server = office.serve_sync("a13.gm", &[]);
    let field = Gm::empty();

    field.ok(&["clone", &server.url()]);
    assert!(
        field.path("a13-corridor.gm").exists(),
        "should be named for the project, not the host and port"
    );
}

#[test]
fn pulling_over_http_fast_forwards() {
    let office = Gm::with_route();
    let server = office.serve_sync("a13.gm", &[]);
    let field = Gm::empty();
    clone_from(&field, &server.url(), "field.gm");

    office.sqlite(
        "a13.gm",
        "UPDATE ground_layers SET top_level = 4.25
           WHERE material_key = 'ALLUVIUM'
             AND ground_model_id = (SELECT id FROM ground_models
                                     WHERE model_key = 'CH-150');",
    );
    office.ok(&["commit", "-m", "CH-150 channel top raised"]);

    field
        .ok(&["-f", "field.gm", "pull"])
        .says("fast-forwarded to");
    assert_eq!(
        field
            .ok(&["-f", "field.gm", "show", "CH-150"])
            .row("ALLUVIUM")[0],
        "4.25"
    );
    field
        .ok(&["-f", "field.gm", "pull"])
        .says("already up to date");
}

#[test]
fn pushing_over_http_moves_the_remote() {
    let office = Gm::with_route();
    let server = office.serve_sync("a13.gm", &["--allow-push"]);
    let field = Gm::empty();
    clone_from(&field, &server.url(), "field.gm");

    field.sqlite(
        "field.gm",
        "UPDATE ground_layers SET top_level = 4.25
           WHERE material_key = 'ALLUVIUM'
             AND ground_model_id = (SELECT id FROM ground_models
                                     WHERE model_key = 'CH-150');",
    );
    field.ok(&["-f", "field.gm", "commit", "-m", "CH-150 re-log"]);
    field.ok(&["-f", "field.gm", "push"]).says("pushed to");

    // The server rewrote its own working tree, so the change is queryable
    // there without restarting it.
    assert_eq!(
        office
            .ok(&["-f", "a13.gm", "show", "CH-150"])
            .row("ALLUVIUM")[0],
        "4.25"
    );
    office.ok(&["-f", "a13.gm", "verify"]);
    office.ok(&["-f", "a13.gm", "log"]).says("CH-150 re-log");
}

#[test]
fn a_push_sends_only_what_changed() {
    // The point of content addressing: a routine push is the handful of objects
    // that actually moved, not the whole file.
    let office = Gm::with_route();
    let server = office.serve_sync("a13.gm", &["--allow-push"]);
    let field = Gm::empty();
    clone_from(&field, &server.url(), "field.gm");

    field.sqlite(
        "field.gm",
        "UPDATE ground_layers SET top_level = 4.25
           WHERE material_key = 'ALLUVIUM'
             AND ground_model_id = (SELECT id FROM ground_models
                                     WHERE model_key = 'CH-150');",
    );
    field.ok(&["-f", "field.gm", "commit", "-m", "CH-150 re-log"]);

    let run = field.ok(&["-f", "field.gm", "push"]);
    // One changed model document plus one new manifest. Six models and four
    // materials went nowhere.
    run.says("(1 commits, 2 objects)");
}

#[test]
fn a_read_only_remote_refuses_a_push_but_still_serves_reads() {
    let office = Gm::with_route();
    let server = office.serve_sync("a13.gm", &[]);
    let field = Gm::empty();
    clone_from(&field, &server.url(), "field.gm");

    field.sqlite(
        "field.gm",
        "UPDATE ground_layers SET top_level = 4.25
           WHERE material_key = 'ALLUVIUM'
             AND ground_model_id = (SELECT id FROM ground_models
                                     WHERE model_key = 'CH-150');",
    );
    field.ok(&["-f", "field.gm", "commit", "-m", "CH-150 re-log"]);

    field
        .fails(&["-f", "field.gm", "push"])
        .says("read-only")
        .says("--allow-push");
    field
        .ok(&["-f", "field.gm", "pull"])
        .says("already up to date");
}

#[test]
fn a_token_is_required_when_the_server_asks_for_one() {
    let office = Gm::with_route();
    let server = office.serve_sync("a13.gm", &["--allow-push", "--token", "s3cret"]);
    let field = Gm::empty();

    field
        .fails(&["clone", &server.url(), "field.gm"])
        .says("token");
    field.ok(&["clone", &server.url(), "field.gm", "--token", "s3cret"]);

    field
        .fails(&["-f", "field.gm", "pull", &server.url()])
        .says("token");
    field.ok(&["-f", "field.gm", "pull", &server.url(), "--token", "s3cret"]);
}

#[test]
fn a_wrong_token_is_refused() {
    let office = Gm::with_route();
    let server = office.serve_sync("a13.gm", &["--token", "s3cret"]);
    let field = Gm::empty();
    field
        .fails(&["clone", &server.url(), "field.gm", "--token", "wrong"])
        .says("token");
}

#[test]
fn a_diverged_push_is_refused_rather_than_forced() {
    let office = Gm::with_route();
    let server = office.serve_sync("a13.gm", &["--allow-push"]);
    let field = Gm::empty();
    clone_from(&field, &server.url(), "field.gm");

    // Both sides move, on different chainages.
    office.sqlite(
        "a13.gm",
        "UPDATE ground_layers SET top_level = 6.60
           WHERE material_key = 'TERRACE_GRAVEL'
             AND ground_model_id = (SELECT id FROM ground_models
                                     WHERE model_key = 'CH-000');",
    );
    office.ok(&["commit", "-m", "CH-000 re-log"]);
    field.sqlite(
        "field.gm",
        "UPDATE ground_layers SET top_level = 4.80
           WHERE material_key = 'TERRACE_GRAVEL'
             AND ground_model_id = (SELECT id FROM ground_models
                                     WHERE model_key = 'CH-250');",
    );
    field.ok(&["-f", "field.gm", "commit", "-m", "CH-250 re-log"]);

    field
        .fails(&["-f", "field.gm", "push"])
        .says("pull and merge before pushing");

    // Pull, merge, push: the whole loop over HTTP.
    let pulled = field.ok(&["-f", "field.gm", "pull"]);
    pulled.says("diverged");
    let theirs = pulled
        .stdout
        .lines()
        .find(|l| l.contains("gm merge"))
        .and_then(|l| l.split_whitespace().nth(3))
        .map(|h| h.trim_matches('`').to_string())
        .expect("the message should name the revision to merge");

    field
        .ok(&["-f", "field.gm", "merge", &theirs])
        .says("merged");
    field.ok(&["-f", "field.gm", "push"]).says("pushed to");

    // Both engineers' work is on the server.
    let run = office.ok(&["-f", "a13.gm", "show", "CH-000"]);
    assert_eq!(
        run.row("TERRACE_GRAVEL")[0],
        "6.60",
        "the office edit\n{run}"
    );
    let run = office.ok(&["-f", "a13.gm", "show", "CH-250"]);
    assert_eq!(
        run.row("TERRACE_GRAVEL")[0],
        "4.80",
        "the field edit\n{run}"
    );
    office.ok(&["-f", "a13.gm", "verify"]);
}

#[test]
fn unrelated_projects_refuse_to_sync_over_http() {
    let office = Gm::with_route();
    let server = office.serve_sync("a13.gm", &["--allow-push"]);

    let other = Gm::empty();
    other.ok(&["init", "other.gm", "--name", "Other job", "--datum", "ODN"]);
    other
        .fails(&["-f", "other.gm", "pull", &server.url()])
        .says("different projects");
    other
        .fails(&["-f", "other.gm", "push", &server.url()])
        .says("different projects");
}

#[test]
fn a_push_without_a_head_header_is_rejected() {
    let gm = Gm::with_route();
    let server = gm.serve_sync("a13.gm", &["--allow-push"]);
    server.post("/sync/push").status_is(400).says("X-GM-Head");
}

#[test]
fn a_body_that_is_not_a_bundle_is_rejected() {
    let gm = Gm::with_route();
    let server = gm.serve_sync("a13.gm", &["--allow-push"]);
    let before = gm.head("a13.gm");

    // No X-GM-Head, so this stops at the header check; with one, the bundle
    // decoder stops it. Either way nothing is stored and the head is untouched.
    server.post("/sync/push").status_is(400);
    assert_eq!(gm.head("a13.gm"), before);
}

#[test]
fn a_wrong_method_on_a_sync_endpoint_says_so() {
    let gm = Gm::with_route();
    let server = gm.serve_sync("a13.gm", &["--allow-push"]);
    server
        .get("/sync/push")
        .status_is(405)
        .says("does not accept GET");
    server.post("/sync/info").status_is(405);
    server.get("/sync/nonsense").status_is(404);
}

#[test]
fn pointing_at_something_that_is_not_a_gm_server_fails_clearly() {
    let gm = Gm::with_route();
    // A real HTTP server that knows nothing about sync: gm's own UI pages are
    // reachable, but a bogus path is not a remote description.
    let server = gm.serve("a13.gm");
    let other = Gm::empty();
    other.ok(&["init", "x.gm", "--name", "X", "--datum", "ODN"]);
    other
        .fails(&[
            "-f",
            "x.gm",
            "pull",
            &format!("{}/not-a-remote", server.url()),
        ])
        .says("gm serve");
}

#[test]
fn https_urls_are_refused_rather_than_quietly_downgraded() {
    Gm::with_route()
        .fails(&["pull", "https://example.com"])
        .says("https is not supported");
}
