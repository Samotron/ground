#!/usr/bin/env bash
#
# User acceptance testing for gm.
#
#   uat/run.sh            walk through every scenario, showing each command
#   uat/run.sh --step     the same, pausing between scenarios
#   uat/run.sh ui         seed a file and open the web UI
#   uat/run.sh shell      seed a file and drop into a shell with gm on PATH
#   uat/run.sh seed       just build the sandbox and stop
#   uat/run.sh clean      delete the sandbox
#
# Everything happens in uat/workspace, which is gitignored. Nothing outside it
# is touched, and it is rebuilt from scratch on each run.

set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK="$REPO/uat/workspace"
GM="$REPO/target/release/gm"
EXAMPLE="$REPO/examples/a13-route.gm.json"
PORT="${GM_UAT_PORT:-8765}"

export GM_AUTHOR="${GM_AUTHOR:-UAT Tester <uat@example.com>}"

STEP=0
FAILURES=0
SCENARIO=0
SERVER_PID=""

# A server started mid-walkthrough must not outlive it, including on Ctrl-C.
cleanup() {
  [[ -n "$SERVER_PID" ]] && kill "$SERVER_PID" 2>/dev/null
  return 0
}
trap cleanup EXIT INT TERM

# -- output -----------------------------------------------------------------

if [[ -t 1 && -z "${NO_COLOR:-}" ]]; then
  BOLD=$'\033[1m'; DIM=$'\033[2m'; RESET=$'\033[0m'
  CYAN=$'\033[36m'; YELLOW=$'\033[33m'; GREEN=$'\033[32m'; RED=$'\033[31m'
else
  BOLD=''; DIM=''; RESET=''; CYAN=''; YELLOW=''; GREEN=''; RED=''
fi

scenario() {
  SCENARIO=$((SCENARIO + 1))
  printf '\n%s\n' "${YELLOW}${BOLD}── ${SCENARIO}. $* ${RESET}"
  printf '%s\n' "${DIM}$(printf '─%.0s' {1..72})${RESET}"
}

note() { printf '%s\n' "${DIM}$*${RESET}"; }

pause() {
  if [[ $STEP -eq 1 && -t 0 ]]; then
    printf '\n%s' "${DIM}   [enter to continue]${RESET}"
    read -r _
  fi
}

# Render a command the way you would type it: the binary as plain `gm`, and
# quoting restored around anything the shell would otherwise mangle. The
# transcript this script produces is the acceptance record, so every line in it
# has to be something the tester can paste back and run.
fmt_cmd() {
  local out='' arg
  for arg in "$@"; do
    [[ "$arg" == "$GM" ]] && arg="gm"
    if [[ -n "$arg" && "$arg" != *[^A-Za-z0-9_/.:=@%+-]* ]]; then
      : # nothing the shell would touch; leave it bare
    elif [[ "$arg" != *[\"\$\`\\]* && "$arg" == *\'* ]]; then
      # Contains single quotes but nothing double quotes would re-interpret,
      # which is exactly the shape of the SQL below. Double-quoting keeps it
      # readable where '\'' escaping would shred it.
      arg="\"$arg\""
    elif [[ "$arg" != *\'* ]]; then
      arg="'$arg'"
    else
      arg=$(printf '%q' "$arg")
    fi
    out+="${out:+ }$arg"
  done
  printf '%s' "$out"
}

# Show a command, run it, and require success.
run() {
  printf '\n%s\n' "${CYAN}\$ $(fmt_cmd "$@")${RESET}"
  if ! "$@"; then
    printf '%s\n' "${RED}   UNEXPECTED FAILURE: the command above should have succeeded${RESET}"
    FAILURES=$((FAILURES + 1))
  fi
}

# Show a command, run it, and require *failure*. Several of gm's guarantees are
# refusals — a commit that will not record an invalid model, a push that will
# not overwrite someone else's work — so the acceptance test has to check that
# they still refuse, not just that the happy path works.
run_must_fail() {
  printf '\n%s\n' "${CYAN}\$ $(fmt_cmd "$@")${RESET}   ${DIM}(expected to be refused)${RESET}"
  if "$@"; then
    printf '%s\n' "${RED}   UNEXPECTED SUCCESS: the command above should have been refused${RESET}"
    FAILURES=$((FAILURES + 1))
  else
    printf '%s\n' "${GREEN}   correctly refused${RESET}"
  fi
}

# -- setup ------------------------------------------------------------------

build() {
  if [[ ! -x "$GM" || -n "$(find "$REPO/crates" -newer "$GM" -name '*.rs' -print -quit 2>/dev/null)" ]]; then
    note "building gm..."
    (cd "$REPO" && cargo build --release --quiet)
  fi
}

seed() {
  build
  rm -rf "$WORK"
  mkdir -p "$WORK"
  cd "$WORK"
  "$GM" init a13.gm \
    --name "A13 corridor ground models" \
    --crs EPSG:27700 \
    --datum "Ordnance Datum Newlyn" >/dev/null
  "$GM" -f a13.gm import "$EXAMPLE" >/dev/null
  "$GM" -f a13.gm commit -m "Import interpretation 001 (6 chainages)" >/dev/null
  note "sandbox ready: uat/workspace/a13.gm"
}

clean() {
  rm -rf "$WORK"
  echo "removed uat/workspace"
}

# -- scenarios --------------------------------------------------------------

demo() {
  build
  rm -rf "$WORK"; mkdir -p "$WORK"; cd "$WORK"

  printf '%s\n' "${BOLD}gm — user acceptance test${RESET}"
  note "sandbox: $WORK"
  note "author:  $GM_AUTHOR"

  scenario "Create a file and load a route into it"
  note "A ground-model file is one SQLite file. Nothing else is installed."
  run "$GM" init a13.gm --name "A13 corridor ground models" \
      --crs EPSG:27700 --datum "Ordnance Datum Newlyn"
  run "$GM" -f a13.gm import "$EXAMPLE"
  run "$GM" -f a13.gm commit -m "Import interpretation 001 (6 chainages)"
  pause

  scenario "Look at what is in it"
  run "$GM" -f a13.gm info
  run "$GM" -f a13.gm models
  note ""
  note "CH-150 sits over the deepest part of the buried channel:"
  run "$GM" -f a13.gm show CH-150
  run "$GM" -f a13.gm show LONDON_CLAY
  pause

  scenario "Edit with any SQLite client, then commit"
  note "The materialised tables ARE the working tree. No import/export step."
  run sqlite3 a13.gm \
    "UPDATE ground_layers SET top_level = -0.20
       WHERE material_key = 'TERRACE_GRAVEL'
         AND ground_model_id = (SELECT id FROM ground_models WHERE model_key = 'CH-150');"
  run "$GM" -f a13.gm status
  run "$GM" -f a13.gm diff
  run "$GM" -f a13.gm commit -m "CH-150: gravel base 600mm shallower after BH112 re-log"
  pause

  scenario "History and time travel"
  run "$GM" -f a13.gm log
  FIRST=$("$GM" -f a13.gm log | tail -2 | head -1 | cut -d' ' -f1)
  note ""
  note "The same model as it was at commit $FIRST:"
  run "$GM" -f a13.gm show CH-150 --rev "$FIRST"
  pause

  scenario "Validation refuses to record a broken model"
  note "Put the London Clay above the gravel — a physically impossible succession."
  run sqlite3 a13.gm \
    "UPDATE ground_layers SET top_level = 99.0
       WHERE material_key = 'LONDON_CLAY'
         AND ground_model_id = (SELECT id FROM ground_models WHERE model_key = 'CH-100');"
  run_must_fail "$GM" -f a13.gm validate
  run_must_fail "$GM" -f a13.gm commit -m "this must not land"
  note ""
  note "Roll the working tree back to the last good revision:"
  run "$GM" -f a13.gm checkout HEAD --force
  run "$GM" -f a13.gm validate
  pause

  scenario "Two engineers, two copies, independent work"
  run "$GM" clone a13.gm alice.gm
  run "$GM" clone a13.gm bob.gm
  note ""
  note "Alice re-logs CH-000; Bob independently re-logs CH-250."
  run sqlite3 alice.gm \
    "UPDATE ground_layers SET top_level = 6.55
       WHERE material_key = 'TERRACE_GRAVEL'
         AND ground_model_id = (SELECT id FROM ground_models WHERE model_key = 'CH-000');"
  run "$GM" -f alice.gm commit -m "CH-000: gravel 250mm deeper" \
      --author "Alice <alice@example.com>"
  run sqlite3 bob.gm \
    "UPDATE ground_layers SET top_level = 4.75
       WHERE material_key = 'TERRACE_GRAVEL'
         AND ground_model_id = (SELECT id FROM ground_models WHERE model_key = 'CH-250');"
  run "$GM" -f bob.gm commit -m "CH-250: gravel 150mm shallower" \
      --author "Bob <bob@example.com>"
  pause

  scenario "Push, diverge, and merge automatically"
  run "$GM" -f alice.gm push
  note ""
  note "Bob pulls. Both sides have moved, so nothing is changed for him yet:"
  run "$GM" -f bob.gm pull
  ALICE=$("$GM" -f alice.gm log -n 1 | cut -d' ' -f1)
  note ""
  note "Different chainages, so this merges without asking:"
  run "$GM" -f bob.gm merge "$ALICE"
  note ""
  note "Both engineers' work is present:"
  run "$GM" -f bob.gm sql \
    "SELECT model_key, top_level FROM layer_intervals
      WHERE material_key = 'TERRACE_GRAVEL' AND model_key IN ('CH-000','CH-250')
      ORDER BY model_key;"
  pause

  scenario "Conflicting edits are refused, not guessed at"
  run "$GM" clone a13.gm carol.gm
  run "$GM" clone a13.gm dave.gm
  note ""
  note "Both re-log the SAME borehole, to different answers."
  run sqlite3 carol.gm \
    "UPDATE ground_layers SET top_level = 3.90
       WHERE material_key = 'ALLUVIUM'
         AND ground_model_id = (SELECT id FROM ground_models WHERE model_key = 'CH-100');"
  run "$GM" -f carol.gm commit -m "CH-100: alluvium top at 3.90" \
      --author "Carol <carol@example.com>"
  run sqlite3 dave.gm \
    "UPDATE ground_layers SET top_level = 4.60
       WHERE material_key = 'ALLUVIUM'
         AND ground_model_id = (SELECT id FROM ground_models WHERE model_key = 'CH-100');"
  run "$GM" -f dave.gm commit -m "CH-100: alluvium top at 4.60" \
      --author "Dave <dave@example.com>"

  CAROL=$("$GM" -f carol.gm log -n 1 | cut -d' ' -f1)
  run "$GM" -f dave.gm pull carol.gm
  run_must_fail "$GM" -f dave.gm merge "$CAROL"
  note ""
  note "Nothing was written. Dave still has his own value and a clean tree:"
  run "$GM" -f dave.gm status
  pause

  scenario "Sync over the network, not just the filesystem"
  note "One person serves the file; everyone else works against the URL."
  PORT="${GM_UAT_SERVE_PORT:-8799}"
  # Started directly rather than through `run`: a backgrounded server would
  # otherwise hold this script's stdout open, so anything piping the
  # walkthrough into a pager or a file would hang waiting for EOF.
  printf '\n%s\n' "${CYAN}\$ $(fmt_cmd "$GM" -f a13.gm serve --port "$PORT" --allow-push)${RESET}"
  "$GM" -f a13.gm serve --port "$PORT" --allow-push > serve.log 2>&1 &
  SERVER_PID=$!
  for _ in $(seq 1 60); do
    curl -s -o /dev/null "http://127.0.0.1:$PORT/sync/info" 2>/dev/null && break
    sleep 0.1
  done
  sed 's/^/  /' serve.log

  note ""
  note "What the remote says about itself:"
  run curl -s "http://127.0.0.1:$PORT/sync/info"
  note ""
  note "A colleague clones over HTTP:"
  run "$GM" clone "http://127.0.0.1:$PORT" remote-copy.gm
  run sqlite3 remote-copy.gm \
    "UPDATE ground_layers SET top_level = 3.60
       WHERE material_key = 'ALLUVIUM'
         AND ground_model_id = (SELECT id FROM ground_models WHERE model_key = 'CH-200');"
  run "$GM" -f remote-copy.gm commit -m "CH-200: alluvium top lowered" \
      --author "Erin <erin@example.com>"

  note ""
  note "The push carries only what changed, not the whole file:"
  run "$GM" -f remote-copy.gm push
  run "$GM" -f a13.gm show CH-200

  kill "$SERVER_PID" 2>/dev/null
  SERVER_PID=""
  pause

  scenario "Integrity and interchange"
  run "$GM" -f bob.gm verify
  run "$GM" -f a13.gm export -o handover.json
  note ""
  note "The flat JSON is what you send to someone with no tooling:"
  run head -14 handover.json
  pause

  scenario "Read it from DuckDB, no export step"
  if command -v duckdb >/dev/null 2>&1; then
    run duckdb -c "INSTALL sqlite; LOAD sqlite;
      ATTACH 'a13.gm' AS gm (TYPE sqlite, READ_ONLY);
      SELECT model_key, material_key, top_level, base_level, round(thickness, 2) AS thickness
        FROM gm.layer_intervals
       WHERE model_key IN ('CH-000', 'CH-150')
       ORDER BY model_key, layer_order;"
  else
    note "duckdb not installed — skipping. Install it and rerun to check this."
  fi

  # -- summary --------------------------------------------------------------
  printf '\n%s\n' "${DIM}$(printf '─%.0s' {1..72})${RESET}"
  if [[ $FAILURES -eq 0 ]]; then
    printf '%s\n' "${GREEN}${BOLD}All ${SCENARIO} scenarios behaved as expected.${RESET}"
  else
    printf '%s\n' "${RED}${BOLD}${FAILURES} unexpected result(s). See the marked lines above.${RESET}"
  fi
  cat <<EOF

Next:
  ${BOLD}just ui${RESET}       the web UI, with drawn sections
  ${BOLD}just sandbox${RESET}  a shell in the sandbox, gm on PATH
  ${BOLD}just gm ...${RESET}   run gm against the sandbox, e.g. just gm show CH-150

  ${BOLD}uat/CHECKLIST.md${RESET}  what to sign off, and what to look for

(Without just: uat/run.sh ui, uat/run.sh shell.)

The sandbox is left in place at uat/workspace.
EOF
  [[ $FAILURES -eq 0 ]] || exit 1
}

ui() {
  [[ -f "$WORK/a13.gm" ]] || seed
  cd "$WORK"
  cat <<EOF

${BOLD}Opening the web UI.${RESET} Worth looking at:

  - the drawn section on any model page, and how the buried channel
    thickens from CH-000 through CH-150 and pinches out again by CH-250
  - a material keeps the same colour on every page
  - History, then any commit, to see what that revision changed
  - Validation, and the issues shown on the model they concern

EOF
  exec "$GM" -f a13.gm ui --port "$PORT"
}

interactive_shell() {
  [[ -f "$WORK/a13.gm" ]] || seed
  cd "$WORK"
  export PATH="$REPO/target/release:$PATH"
  export GM_FILE="$WORK/a13.gm"
  cat <<EOF

${BOLD}Sandbox shell.${RESET} gm is on PATH and \$GM_FILE points at a13.gm,
so you can just type ${BOLD}gm models${RESET}. Try:

  gm models                  gm show CH-150
  gm log                     gm show CH-150 --rev <hash>
  gm validate                gm verify
  gm sql "SELECT * FROM layer_intervals LIMIT 10"
  sqlite3 a13.gm             then gm status / gm diff / gm commit -m ...

Type 'exit' to leave.

EOF
  exec "${SHELL:-/bin/bash}"
}

# -- entry point ------------------------------------------------------------

CMD="demo"
for arg in "$@"; do
  case "$arg" in
    --step|-s) STEP=1 ;;
    demo|ui|shell|seed|clean) CMD="$arg" ;;
    -h|--help) sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

case "$CMD" in
  demo)  demo ;;
  ui)    ui ;;
  shell) interactive_shell ;;
  seed)  seed ;;
  clean) clean ;;
esac
