//! What a user types, and what they get back.
//!
//! These drive the real binary. They assert on stdout, stderr and exit status,
//! because those are the whole product as far as anyone using `gm` is
//! concerned — a correct library behind a command that prints the wrong number
//! is still wrong.

mod common;

use common::{AUTHOR, Gm, example};

// -- starting a file --------------------------------------------------------

#[test]
fn init_creates_a_file_and_reports_its_identity() {
    let gm = Gm::empty();
    gm.ok(&["init", "route.gm", "--name", "A13", "--crs", "EPSG:27700"])
        .says("initialised route.gm")
        .says("file id");
    assert!(gm.path("route.gm").exists());
}

#[test]
fn init_nudges_you_when_the_datum_is_missing() {
    // Levels without a datum are ambiguous, and silently accepting that is how
    // a file ends up meaning nothing in five years' time.
    Gm::empty()
        .ok(&["init", "route.gm", "--name", "A13"])
        .says("--crs and --datum");
}

#[test]
fn init_does_not_nag_when_everything_is_given() {
    Gm::empty()
        .ok(&[
            "init",
            "route.gm",
            "--name",
            "A13",
            "--crs",
            "EPSG:27700",
            "--datum",
            "ODN",
        ])
        .does_not_say("--crs and --datum");
}

#[test]
fn init_refuses_to_overwrite_an_existing_file() {
    let gm = Gm::empty();
    gm.ok(&["init", "route.gm", "--name", "A13"]);
    gm.fails(&["init", "route.gm", "--name", "Something else"])
        .says("already exists");
}

#[test]
fn commands_refuse_to_invent_an_author() {
    // A commit is attribution. Guessing at who made an interpretation would be
    // worse than refusing.
    let gm = Gm::empty();
    let run = gm.run_anonymous(&["init", "route.gm", "--name", "A13"]);
    assert!(!run.success(), "expected failure\n{run}");
    run.says("no author");
}

// -- finding the file -------------------------------------------------------

#[test]
fn a_single_file_in_the_directory_is_found_without_being_named() {
    Gm::with_route().ok(&["models"]).says("CH-150");
}

#[test]
fn with_no_file_present_it_says_what_to_do() {
    Gm::empty()
        .fails(&["models"])
        .says("no ground-model file found");
}

#[test]
fn with_several_files_present_it_asks_which() {
    let gm = Gm::with_route();
    gm.ok(&["init", "other.gm", "--name", "Other", "--datum", "ODN"]);
    gm.fails(&["models"])
        .says("pass --file")
        .says("a13.gm")
        .says("other.gm");
}

#[test]
fn opening_something_that_is_not_a_ground_model_file_is_refused() {
    let gm = Gm::empty();
    std::fs::write(gm.path("notes.gm"), "this is not a database").expect("write");
    gm.fails(&["-f", "notes.gm", "models"])
        .says("not a ground-model file");
}

// -- reading ----------------------------------------------------------------

#[test]
fn info_reports_the_things_that_make_the_numbers_mean_something() {
    Gm::with_route()
        .ok(&["info"])
        .says("EPSG:27700")
        .says("Ordnance Datum Newlyn")
        .says("models     6")
        .says("materials  4");
}

#[test]
fn models_lists_every_model_with_its_extent() {
    let run = Gm::with_route().ok(&["models"]);
    for key in ["CH-000", "CH-050", "CH-100", "CH-150", "CH-200", "CH-250"] {
        run.says(key);
    }
    // key, surface, base, layers, then the name
    let row = run.row("CH-150");
    assert_eq!(row[1], "6.20", "surface level\n{run}");
    assert_eq!(row[2], "-18.80", "base level\n{run}");
    assert_eq!(row[3], "4", "layer count\n{run}");
}

#[test]
fn show_draws_a_section_whose_arithmetic_holds_up() {
    let run = Gm::with_route().ok(&["show", "CH-150"]);

    // level, depth, thickness, material
    let made = run.row("MADE_GROUND");
    assert_eq!(made[0], "6.20", "top of Made Ground is ground level\n{run}");
    assert_eq!(made[1], "0.00", "so its depth is zero\n{run}");
    assert_eq!(made[2], "2.40", "thickness\n{run}");

    let alluvium = run.row("ALLUVIUM");
    assert_eq!(alluvium[0], "3.80", "top of Alluvium\n{run}");
    assert_eq!(alluvium[1], "2.40", "depth == surface - level\n{run}");
    assert_eq!(alluvium[2], "4.60", "thickness\n{run}");

    // The deepest layer must have a bottom, and the section must close on it.
    run.says("(base of model)");
    let base = run.row("(base of model)");
    assert_eq!(base[0], "-18.80");
    assert_eq!(base[1], "25.00", "the model is 25 m deep\n{run}");
}

#[test]
fn a_sections_thicknesses_sum_to_the_model_extent() {
    let run = Gm::with_route().ok(&["show", "CH-150"]);
    let total: f64 = ["MADE_GROUND", "ALLUVIUM", "TERRACE_GRAVEL", "LONDON_CLAY"]
        .iter()
        .map(|m| run.row(m)[2].parse::<f64>().expect("a thickness"))
        .sum();
    assert!(
        (total - 25.0).abs() < 1e-9,
        "layers should fill the model exactly, got {total}\n{run}"
    );
}

#[test]
fn show_reports_the_groundwater_regime() {
    Gm::with_route()
        .ok(&["show", "CH-150"])
        .says("hydrostatic")
        .says("1.80 m below ground level");
}

#[test]
fn show_on_a_material_gives_parameters_with_their_ranges() {
    Gm::with_route()
        .ok(&["show", "LONDON_CLAY"])
        .says("London Clay Formation")
        .says("undrained-tresca")
        .says("mohr-coulomb")
        // value and credible range together: a bare number cannot say how well
        // constrained an interpretation is
        .says("20 (19-21) kN/m3")
        .says("profile");
}

#[test]
fn a_depth_varying_parameter_says_what_its_depths_are_measured_from() {
    // Materials are shared between models, so "depth" is meaningless without
    // a datum.
    Gm::with_route()
        .ok(&["show", "LONDON_CLAY"])
        .says("from LayerTop");
}

#[test]
fn asking_for_something_that_is_not_there_names_it() {
    Gm::with_route()
        .fails(&["show", "CH-999"])
        .says("no model or material called 'CH-999'");
}

#[test]
fn cat_emits_the_versioned_document_as_json() {
    let run = Gm::with_route().ok(&["cat", "CH-150"]);
    let doc = run.json();
    assert_eq!(doc["modelKey"], "CH-150");
    assert_eq!(doc["layers"].as_array().expect("layers").len(), 4);
    assert_eq!(doc["surfaceLevel"], 6.2);
}

#[test]
fn sql_runs_a_query_against_the_materialised_tables() {
    let run = Gm::with_route().ok(&[
        "sql",
        "SELECT model_key, round(thickness, 2) FROM layer_intervals \
         WHERE material_key = 'ALLUVIUM' ORDER BY model_key",
    ]);
    // The channel is absent at both ends of the route and deepest in the middle.
    run.does_not_say("CH-000").does_not_say("CH-250");
    assert_eq!(run.row("CH-150")[1], "4.6", "{run}");
}

// -- editing and history ----------------------------------------------------

#[test]
fn a_fresh_file_has_nothing_to_commit() {
    let gm = Gm::with_route();
    gm.ok(&["status"]).says("working tree matches");
    gm.ok(&["commit", "-m", "again"]).says("nothing to commit");
}

#[test]
fn an_edit_made_with_plain_sql_is_picked_up() {
    // The headline claim: the materialised tables are the working tree, so any
    // SQLite client is a valid editor.
    let gm = Gm::with_route();
    gm.sqlite(
        "a13.gm",
        "UPDATE ground_layers SET top_level = -0.20
           WHERE material_key = 'TERRACE_GRAVEL'
             AND ground_model_id = (SELECT id FROM ground_models
                                     WHERE model_key = 'CH-150');",
    );

    gm.ok(&["status"]).says("M").says("CH-150");
    gm.ok(&["diff"])
        .says("~ model CH-150")
        .says("-0.80 -> -0.20")
        .says("+0.60 m");
    gm.ok(&["commit", "-m", "CH-150 gravel base raised"])
        .says("committed");
    gm.ok(&["status"]).says("working tree matches");
}

#[test]
fn history_records_who_did_what() {
    let gm = Gm::with_route();
    gm.ok(&["log"])
        .says("Import interpretation 001")
        .says("Initialise ground-model file")
        .says(AUTHOR);
}

#[test]
fn an_earlier_revision_still_reads_back() {
    let gm = Gm::with_route();
    let before = gm.head("a13.gm");

    gm.sqlite(
        "a13.gm",
        "UPDATE ground_layers SET top_level = 5.00
           WHERE material_key = 'ALLUVIUM'
             AND ground_model_id = (SELECT id FROM ground_models
                                     WHERE model_key = 'CH-150');",
    );
    gm.ok(&["commit", "-m", "move the channel top"]);

    assert_eq!(gm.ok(&["show", "CH-150"]).row("ALLUVIUM")[0], "5.00");
    assert_eq!(
        gm.ok(&["show", "CH-150", "--rev", &before]).row("ALLUVIUM")[0],
        "3.80",
        "the old revision should be unchanged"
    );
}

#[test]
fn an_abbreviated_revision_is_enough() {
    let gm = Gm::with_route();
    let short: String = gm.head("a13.gm").chars().take(8).collect();
    gm.ok(&["show", "CH-150", "--rev", &short]).says("CH-150");
}

#[test]
fn an_unknown_revision_is_reported_rather_than_guessed_at() {
    Gm::with_route()
        .fails(&["show", "CH-150", "--rev", "deadbeef"])
        .says("commit not found");
}

#[test]
fn checkout_discards_a_bad_working_tree_edit() {
    let gm = Gm::with_route();
    gm.sqlite(
        "a13.gm",
        "UPDATE ground_layers SET top_level = 5.00
           WHERE material_key = 'ALLUVIUM'
             AND ground_model_id = (SELECT id FROM ground_models
                                     WHERE model_key = 'CH-150');",
    );
    gm.fails(&["checkout", "HEAD"]).says("uncommitted changes");
    gm.ok(&["checkout", "HEAD", "--force"]);
    gm.ok(&["status"]).says("working tree matches");
    assert_eq!(gm.ok(&["show", "CH-150"]).row("ALLUVIUM")[0], "3.80");
}

// -- validation -------------------------------------------------------------

#[test]
fn a_sound_route_validates_without_complaint() {
    Gm::with_route()
        .ok(&["validate"])
        .says("0 error(s), 0 warning(s)");
}

#[test]
fn an_inverted_succession_is_an_error_that_blocks_the_commit() {
    let gm = Gm::with_route();
    gm.sqlite(
        "a13.gm",
        "UPDATE ground_layers SET top_level = 99.0
           WHERE material_key = 'LONDON_CLAY'
             AND ground_model_id = (SELECT id FROM ground_models
                                     WHERE model_key = 'CH-100');",
    );

    // Exit 2 specifically, so a script can tell "invalid" from "crashed".
    gm.fails(&["validate"])
        .exits_with(2)
        .says("CH-100")
        .says("must be ordered downwards");

    // A history full of invalid states removes the one thing a consumer could
    // rely on, so this must be refused.
    gm.fails(&["commit", "-m", "this must not land"])
        .says("validation error");

    assert_eq!(
        gm.ok(&["log"]).stdout.lines().count(),
        2,
        "no new revision should have been recorded"
    );
}

#[test]
fn validation_reports_as_json_for_a_calling_tool() {
    let gm = Gm::with_route();
    gm.sqlite(
        "a13.gm",
        "UPDATE ground_layers SET top_level = 99.0
           WHERE material_key = 'LONDON_CLAY'
             AND ground_model_id = (SELECT id FROM ground_models
                                     WHERE model_key = 'CH-100');",
    );
    let run = gm.run(&["validate", "--json"]);
    let issues = run.json();
    let issues = issues.as_array().expect("an array of issues");
    assert!(
        issues.iter().any(|i| i["severity"] == "error"
            && i["modelKey"] == "CH-100"
            && i["fieldPath"]
                .as_str()
                .is_some_and(|p| p.contains("topLevel"))),
        "expected a located error, got {issues:#?}"
    );
}

#[test]
fn validation_results_can_be_stored_for_querying() {
    let gm = Gm::with_route();
    gm.ok(&["validate", "--store"]);
    gm.ok(&["sql", "SELECT COUNT(*) FROM model_issues"])
        .says("0");
}

// -- integrity --------------------------------------------------------------

#[test]
fn verify_checks_every_object() {
    Gm::with_route().ok(&["verify"]).says("objects and");
}

#[test]
fn verify_catches_a_tampered_document() {
    let gm = Gm::with_route();
    gm.sqlite(
        "a13.gm",
        "UPDATE gm_blob SET content = '{}'
           WHERE hash = (SELECT hash FROM gm_blob
                          WHERE hash NOT IN (SELECT hash FROM gm_commit) LIMIT 1);",
    );
    gm.fails(&["verify"])
        .says("hashes to")
        .says("integrity problem");
}

#[test]
fn a_corrupt_file_cannot_be_cloned() {
    let gm = Gm::with_route();
    gm.sqlite(
        "a13.gm",
        "UPDATE gm_blob SET content = '{}'
           WHERE hash = (SELECT hash FROM gm_blob
                          WHERE hash NOT IN (SELECT hash FROM gm_commit) LIMIT 1);",
    );
    gm.fails(&["clone", "a13.gm", "copy.gm"]).says("corrupt");
}

// -- interchange ------------------------------------------------------------

#[test]
fn export_and_import_round_trip_through_a_second_file() {
    let gm = Gm::with_route();
    gm.ok(&["export", "-o", "handover.json"]);

    gm.ok(&[
        "init",
        "copy.gm",
        "--name",
        "Copy",
        "--crs",
        "EPSG:27700",
        "--datum",
        "ODN",
    ]);
    gm.ok(&["-f", "copy.gm", "import", "handover.json"])
        .says("6 added");
    gm.ok(&["-f", "copy.gm", "commit", "-m", "imported"]);

    let original = gm.ok(&["-f", "a13.gm", "cat", "CH-150"]).json();
    let copied = gm.ok(&["-f", "copy.gm", "cat", "CH-150"]).json();
    assert_eq!(original, copied, "a round trip must not change the model");
}

#[test]
fn export_to_stdout_is_valid_json_naming_its_source_revision() {
    let gm = Gm::with_route();
    let doc = gm.ok(&["export"]).json();
    assert_eq!(doc["type"], "gm.file/1");
    assert_eq!(doc["models"].as_array().expect("models").len(), 6);
    assert!(
        doc["sourceCommit"]
            .as_str()
            .is_some_and(|c| c.starts_with("sha256-")),
        "an export should say which revision it came from"
    );
}

#[test]
fn importing_something_that_is_not_an_interchange_document_is_refused() {
    let gm = Gm::with_route();
    std::fs::write(gm.path("wrong.json"), r#"{"type":"something/else"}"#).expect("write");
    gm.fails(&["import", "wrong.json"]).says("gm.file/1");
}

#[test]
fn the_minimal_example_also_imports() {
    let gm = Gm::empty();
    gm.ok(&["init", "t.gm", "--name", "T", "--datum", "ODN"]);
    gm.ok(&[
        "-f",
        "t.gm",
        "import",
        example("thames-crossing.gm.json").to_str().unwrap(),
    ]);
    gm.ok(&["-f", "t.gm", "commit", "-m", "imported"]);
    gm.ok(&["-f", "t.gm", "validate"]).says("0 error(s)");
}

// -- working with other people ----------------------------------------------

#[test]
fn a_clone_carries_the_whole_history() {
    let gm = Gm::with_route();
    gm.ok(&["clone", "a13.gm", "field.gm"]).says("cloned");

    assert_eq!(
        gm.ok(&["-f", "field.gm", "log"]).stdout,
        gm.ok(&["-f", "a13.gm", "log"]).stdout
    );
    gm.ok(&["-f", "field.gm", "verify"]);
    // A clone knows where it came from, so a bare `gm pull` works.
    gm.ok(&["-f", "field.gm", "remote", "list"]).says("origin");
}

#[test]
fn work_on_different_chainages_merges_without_asking() {
    let gm = Gm::with_route();
    gm.ok(&["clone", "a13.gm", "alice.gm"]);
    gm.ok(&["clone", "a13.gm", "bob.gm"]);

    gm.sqlite(
        "alice.gm",
        "UPDATE ground_layers SET top_level = 6.55
           WHERE material_key = 'TERRACE_GRAVEL'
             AND ground_model_id = (SELECT id FROM ground_models
                                     WHERE model_key = 'CH-000');",
    );
    gm.ok(&["-f", "alice.gm", "commit", "-m", "CH-000 deeper"]);
    gm.sqlite(
        "bob.gm",
        "UPDATE ground_layers SET top_level = 4.75
           WHERE material_key = 'TERRACE_GRAVEL'
             AND ground_model_id = (SELECT id FROM ground_models
                                     WHERE model_key = 'CH-250');",
    );
    gm.ok(&["-f", "bob.gm", "commit", "-m", "CH-250 shallower"]);

    gm.ok(&["-f", "alice.gm", "push"]).says("pushed to");
    let alice = gm.head("alice.gm");

    gm.ok(&["-f", "bob.gm", "pull"])
        .says("diverged")
        .says("gm merge");
    gm.ok(&["-f", "bob.gm", "merge", &alice])
        .says("merged")
        .says("took theirs  CH-000");

    // Both engineers' work survives.
    let run = gm.ok(&["-f", "bob.gm", "show", "CH-000"]);
    assert_eq!(run.row("TERRACE_GRAVEL")[0], "6.55", "Alice's edit\n{run}");
    let run = gm.ok(&["-f", "bob.gm", "show", "CH-250"]);
    assert_eq!(run.row("TERRACE_GRAVEL")[0], "4.75", "Bob's edit\n{run}");
}

#[test]
fn work_on_the_same_chainage_conflicts_and_writes_nothing() {
    let gm = Gm::with_route();
    gm.ok(&["clone", "a13.gm", "carol.gm"]);
    gm.ok(&["clone", "a13.gm", "dave.gm"]);

    for (file, level) in [("carol.gm", "3.90"), ("dave.gm", "4.60")] {
        gm.sqlite(
            file,
            &format!(
                "UPDATE ground_layers SET top_level = {level}
                   WHERE material_key = 'ALLUVIUM'
                     AND ground_model_id = (SELECT id FROM ground_models
                                             WHERE model_key = 'CH-100');"
            ),
        );
        gm.ok(&["-f", file, "commit", "-m", "re-log CH-100"]);
    }

    let carol = gm.head("carol.gm");
    gm.ok(&["-f", "dave.gm", "pull", "carol.gm"])
        .says("diverged");
    gm.fails(&["-f", "dave.gm", "merge", &carol])
        .says("conflicts in 1 document(s)")
        .says("CH-100");

    // Only one of them can be right, and the tool cannot know which, so it must
    // leave Dave exactly as he was.
    gm.ok(&["-f", "dave.gm", "status"])
        .says("working tree matches");
    assert_eq!(
        gm.ok(&["-f", "dave.gm", "show", "CH-100"]).row("ALLUVIUM")[0],
        "4.60"
    );
}

#[test]
fn a_push_that_would_strand_someone_elses_work_is_refused() {
    let gm = Gm::with_route();
    gm.ok(&["clone", "a13.gm", "alice.gm"]);
    gm.ok(&["clone", "a13.gm", "bob.gm"]);

    for (file, key, level) in [("alice.gm", "CH-000", "6.55"), ("bob.gm", "CH-250", "4.75")] {
        gm.sqlite(
            file,
            &format!(
                "UPDATE ground_layers SET top_level = {level}
                   WHERE material_key = 'TERRACE_GRAVEL'
                     AND ground_model_id = (SELECT id FROM ground_models
                                             WHERE model_key = '{key}');"
            ),
        );
        gm.ok(&["-f", file, "commit", "-m", "re-log"]);
    }

    gm.fails(&["-f", "alice.gm", "push", "bob.gm"])
        .says("pull and merge before pushing");
}

#[test]
fn unrelated_files_refuse_to_sync() {
    let gm = Gm::with_route();
    gm.ok(&["init", "other.gm", "--name", "Other job", "--datum", "ODN"]);
    gm.fails(&["-f", "a13.gm", "pull", "other.gm"])
        .says("different projects");
}

#[test]
fn merging_something_already_in_our_history_does_nothing() {
    let gm = Gm::with_route();
    let head = gm.head("a13.gm");
    gm.ok(&["merge", &head]).says("nothing to merge");
}

#[test]
fn remotes_can_be_added_listed_and_removed() {
    let gm = Gm::with_route();
    gm.ok(&["remote", "list"]).says("no remotes");
    gm.ok(&["remote", "add", "office", "/srv/jobs/a13.gm"]);
    gm.ok(&["remote", "list"])
        .says("office")
        .says("/srv/jobs/a13.gm");
    gm.ok(&["remote", "remove", "office"]).says("removed");
    gm.fails(&["remote", "remove", "office"])
        .says("no remote called");
}

// -- the command line itself ------------------------------------------------

#[test]
fn help_lists_the_commands() {
    let run = Gm::empty().ok(&["--help"]);
    for command in [
        "init",
        "info",
        "status",
        "log",
        "models",
        "materials",
        "show",
        "commit",
        "checkout",
        "diff",
        "validate",
        "verify",
        "import",
        "export",
        "sql",
        "ui",
        "clone",
        "pull",
        "push",
        "merge",
        "remote",
    ] {
        run.says(command);
    }
}

#[test]
fn version_is_reported() {
    Gm::empty()
        .ok(&["--version"])
        .says(env!("CARGO_PKG_VERSION"));
}

#[test]
fn an_unknown_command_is_rejected() {
    Gm::empty().fails(&["frobnicate"]);
}

#[test]
fn the_gm_file_environment_variable_selects_the_file() {
    let gm = Gm::with_route();
    gm.ok(&["init", "other.gm", "--name", "Other", "--datum", "ODN"]);
    // Two files present, so the bare command is ambiguous; GM_FILE resolves it.
    let out = std::process::Command::new(common::GM)
        .current_dir(gm.dir.path())
        .env("GM_AUTHOR", AUTHOR)
        .env("GM_FILE", "a13.gm")
        .env("NO_COLOR", "1")
        .args(["models"])
        .output()
        .expect("running gm");
    assert!(
        out.status.success(),
        "GM_FILE should have resolved the ambiguity"
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("CH-150"));
}
