//! The web UI, over real HTTP.
//!
//! These spawn `gm ui` and make requests against it, which is the only way to
//! check the things that matter about a served page: that the routes exist,
//! that the status codes are right, that it refuses to write, and that user
//! data cannot escape into the markup.

mod common;

use common::Gm;

#[test]
fn every_page_is_served() {
    let gm = Gm::with_route();
    let server = gm.serve("a13.gm");

    server.get("/").ok().says("A13 corridor").says("CH-150");
    server.get("/materials").ok().says("LONDON_CLAY");
    server
        .get("/history")
        .ok()
        .says("Import interpretation 001");
    server.get("/validate").ok();
    server.get("/model/CH-150").ok().says("CH-150");
    server.get("/material/ALLUVIUM").ok().says("Alluvium");
}

#[test]
fn a_model_page_draws_a_section_matching_its_table() {
    let gm = Gm::with_route();
    let server = gm.serve("a13.gm");
    let page = server.get("/model/CH-150");

    page.ok()
        .says("<svg")
        .says("class=\"layer\"")
        .says("class=\"boundary base\"")
        // Every stratum at this chainage, in the drawing and the table.
        .says("MADE_GROUND")
        .says("ALLUVIUM")
        .says("TERRACE_GRAVEL")
        .says("LONDON_CLAY")
        // Levels from the model, and the water table.
        .says("6.20")
        .says("-18.80")
        .says("class=\"water\"");

    assert_eq!(
        page.body.matches("class=\"layer\"").count(),
        4,
        "one drawn box per stratum"
    );
}

#[test]
fn the_buried_channel_appears_only_where_it_exists() {
    let gm = Gm::with_route();
    let server = gm.serve("a13.gm");

    // Absent at both ends of the route, present in the middle. If the section
    // drawing ever stopped following the data, this is where it would show.
    server.get("/model/CH-000").ok().does_not_say("ALLUVIUM");
    server.get("/model/CH-250").ok().does_not_say("ALLUVIUM");
    server.get("/model/CH-150").ok().says("ALLUVIUM");

    assert_eq!(
        server
            .get("/model/CH-000")
            .body
            .matches("class=\"layer\"")
            .count(),
        3,
        "three strata where the channel has pinched out"
    );
}

#[test]
fn a_commit_page_shows_what_that_revision_changed() {
    let gm = Gm::with_route();
    gm.sqlite(
        "a13.gm",
        "UPDATE ground_layers SET top_level = -0.20
           WHERE material_key = 'TERRACE_GRAVEL'
             AND ground_model_id = (SELECT id FROM ground_models
                                     WHERE model_key = 'CH-150');",
    );
    gm.ok(&["commit", "-m", "CH-150 gravel raised"]);
    let head = gm.head("a13.gm");

    let server = gm.serve("a13.gm");
    server
        .get(&format!("/commit/{head}"))
        .ok()
        .says("CH-150 gravel raised")
        .says("-0.80 -&gt; -0.20");
}

#[test]
fn validation_problems_are_shown_against_the_model_they_concern() {
    let gm = Gm::with_route();
    gm.sqlite(
        "a13.gm",
        "UPDATE ground_layers SET top_level = 99.0
           WHERE material_key = 'LONDON_CLAY'
             AND ground_model_id = (SELECT id FROM ground_models
                                     WHERE model_key = 'CH-100');",
    );
    let server = gm.serve("a13.gm");

    server
        .get("/validate")
        .ok()
        .says("must be ordered downwards")
        .says("CH-100");
    // And on the model page itself, where someone looking at that model is
    // actually going to be.
    server
        .get("/model/CH-100")
        .ok()
        .says("must be ordered downwards");
    // Not on an unrelated model.
    server
        .get("/model/CH-250")
        .ok()
        .does_not_say("must be ordered downwards");
}

#[test]
fn the_json_endpoints_return_json() {
    let gm = Gm::with_route();
    let server = gm.serve("a13.gm");

    for path in ["/api/export", "/api/models", "/api/validate"] {
        let response = server.get(path);
        response.ok();
        assert!(
            response.content_type.starts_with("application/json"),
            "{path} served as {:?}",
            response.content_type
        );
        serde_json::from_str::<serde_json::Value>(&response.body)
            .unwrap_or_else(|e| panic!("{path} did not return JSON: {e}"));
    }

    let export: serde_json::Value =
        serde_json::from_str(&server.get("/api/export").body).expect("json");
    assert_eq!(export["type"], "gm.file/1");
    assert_eq!(export["models"].as_array().expect("models").len(), 6);
}

#[test]
fn it_refuses_to_write() {
    // Editing goes through the CLI, where it is validated and attributed to a
    // named author. The UI must not become a second, unchecked way in.
    let gm = Gm::with_route();
    let server = gm.serve("a13.gm");
    server.post("/").status_is(405);
    server.post("/model/CH-150").status_is(405);
    server.request("DELETE", "/model/CH-150").status_is(405);
}

#[test]
fn unknown_paths_are_not_found_rather_than_errors() {
    let gm = Gm::with_route();
    let server = gm.serve("a13.gm");
    server.get("/nonsense").status_is(404);
    server.get("/model/CH-999").status_is(404).says("CH-999");
    server.get("/material/NOPE").status_is(404).says("NOPE");
}

#[test]
fn a_model_key_containing_markup_cannot_escape_into_the_page() {
    // Model keys are user data and end up in headings, links and the drawing.
    let gm = Gm::empty();
    gm.ok(&["init", "x.gm", "--name", "Injection test", "--datum", "ODN"]);
    std::fs::write(
        gm.path("evil.json"),
        r#"{
          "type": "gm.file/1",
          "schemaVersion": "0.1.0",
          "file": { "name": "Injection test", "verticalDatum": "ODN" },
          "materials": [{
            "materialKey": "CLAY",
            "name": "<script>alert('material')</script>",
            "properties": { "unitWeight": { "value": 20, "unit": "kN/m3" } },
            "constitutiveModels": []
          }],
          "models": [{
            "modelKey": "<script>alert('model')</script>",
            "surfaceLevel": 10.0,
            "baseLevel": 0.0,
            "groundwater": { "kind": "dry" },
            "layers": [{ "topLevel": 10.0, "materialKey": "CLAY" }]
          }]
        }"#,
    )
    .expect("write");
    gm.ok(&["-f", "x.gm", "import", "evil.json"]);
    gm.ok(&["-f", "x.gm", "commit", "-m", "hostile keys"]);

    let server = gm.serve("x.gm");

    // No page may ever emit the raw tag.
    for path in ["/", "/materials", "/validate", "/history"] {
        server.get(path).ok().does_not_say("<script>");
    }

    // And on the pages that do show these names, they must appear escaped
    // rather than silently dropped: swallowing the key would hide a model.
    for path in ["/", "/materials"] {
        let page = server.get(path);
        assert!(
            page.body.contains("&lt;script&gt;"),
            "{path} should show the name escaped, not drop it\n{}",
            page.body
        );
    }
}

#[test]
fn the_page_reflects_edits_made_while_it_is_running() {
    // The file is reopened per request, so a commit in another terminal shows
    // up on refresh rather than going stale behind a cached handle.
    let gm = Gm::with_route();
    let server = gm.serve("a13.gm");
    server.get("/model/CH-150").ok().says("3.80");

    gm.sqlite(
        "a13.gm",
        "UPDATE ground_layers SET top_level = 4.25
           WHERE material_key = 'ALLUVIUM'
             AND ground_model_id = (SELECT id FROM ground_models
                                     WHERE model_key = 'CH-150');",
    );
    gm.ok(&["commit", "-m", "channel top raised"]);

    server.get("/model/CH-150").ok().says("4.25");
    server.get("/history").ok().says("channel top raised");
}
