# gm — 1D ground models.
#
# Run `just` on its own to see everything available.
# Needs https://just.systems — `cargo install just`, or your package manager.

# bash rather than sh: the UAT script and several recipes below rely on it, and
# `pipefail` means a failing step in a pipeline actually fails the recipe.
set shell := ["bash", "-euo", "pipefail", "-c"]

# Lets recipe bodies use "$@", which preserves the quoting of arguments passed
# through. Without it `just gm sql "SELECT ..."` would lose the quotes and the
# SQL would arrive as a dozen separate words.
set positional-arguments

bin := "target/release/gm"
sandbox := "uat/workspace"

# Show the available recipes.
default:
    @just --list --unsorted

# ---------------------------------------------------------------- build ----

# Debug build.
[group('build')]
build:
    cargo build

# Optimised build: target/release/gm, one self-contained binary.
[group('build')]
release:
    cargo build --release

# Put gm on your PATH.
[group('build')]
install:
    cargo install --path crates/gm-cli

# Remove build output and the demo sandbox.
[group('build')]
clean:
    cargo clean
    rm -rf {{ sandbox }}

# Open the API documentation.
[group('build')]
docs:
    cargo doc --no-deps --open

# ---------------------------------------------------------------- check ----

# Run the test suite. Extra arguments go to cargo test.
[group('check')]
test *args:
    cargo test "$@"

# Just the tests that drive the binary: the command line and the web UI.
[group('check')]
test-cli:
    cargo test --test cli --test ui

# Formatting and lints, exactly as a CI gate would apply them.
[group('check')]
lint:
    cargo fmt --all --check
    cargo clippy --all-targets -- -D warnings

# The pre-commit gate: lints and tests.
[group('check')]
check: lint test

# Everything: lints, tests, the example files, and the full UAT walkthrough.
[group('check')]
ci: lint test examples uat

# Build and test the browser editor deployed to GitHub Pages.
[group('check')]
editor-check:
    cd editor && npm ci && npm test && npm run build

# Apply what clippy and rustfmt can fix on their own.
[group('check')]
fix:
    cargo clippy --all-targets --fix --allow-dirty --allow-staged
    cargo fmt --all

# Format the source.
[group('check')]
fmt:
    cargo fmt --all

# Check every example file still imports and validates cleanly.
[group('check')]
examples: _build
    #!/usr/bin/env bash
    # The examples are documentation, and documentation that has quietly
    # stopped being true is worse than none. This catches one drifting out of
    # step with the format.
    #
    # `gm commit` refuses to record anything with a validation error, so it is
    # the check: with `set -e`, a bad example fails the recipe with gm's own
    # message. Warnings are reported rather than failed on, but an example
    # carrying warnings is a smell worth looking at.
    set -euo pipefail
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    export GM_AUTHOR="just <examples@localhost>"
    for file in examples/*.gm.json; do
        printf '%-38s ' "$file"
        rm -f "$tmp"/check.gm*
        {{ bin }} init "$tmp/check.gm" --name check \
            --crs EPSG:27700 --datum "Ordnance Datum Newlyn" >/dev/null
        {{ bin }} -f "$tmp/check.gm" import "$file" >/dev/null
        {{ bin }} -f "$tmp/check.gm" commit -m check >/dev/null
        models=$({{ bin }} -f "$tmp/check.gm" sql \
            "SELECT COUNT(*) FROM ground_models" | tail -1)
        echo "ok, $models models, $({{ bin }} -f "$tmp/check.gm" validate | tail -1)"
    done

# ----------------------------------------------------------------- demo ----

# Walk through every scenario, showing each command and its output.
[group('demo')]
uat *args:
    ./uat/run.sh "$@"

# The same walkthrough, pausing between scenarios.
[group('demo')]
walk:
    ./uat/run.sh --step

# Seed a sandbox and open the read-only web UI.
[group('demo')]
ui:
    ./uat/run.sh ui

# Run the browser editor locally with hot reload.
[group('demo')]
editor:
    cd editor && npm install && npm run dev

# A shell in the sandbox, with gm on PATH and $GM_FILE set.
[group('demo')]
sandbox:
    ./uat/run.sh shell

# Run gm against the demo sandbox, e.g. `just gm models`, `just gm show CH-150`.
[group('demo')]
gm *args: _build _seeded
    @GM_FILE="{{ justfile_directory() }}/{{ sandbox }}/a13.gm" {{ bin }} "$@"

# Rebuild the sandbox from scratch.
[group('demo')]
reseed:
    ./uat/run.sh seed

# Build quietly when a build is incidental to what was asked for. `just
# release` stays loud, because there you asked for the build itself.
[private]
_build:
    @cargo build --release --quiet

[private]
_seeded:
    @test -f {{ sandbox }}/a13.gm || ./uat/run.sh seed
