# gm — 1D ground models

A tool and a file format for the transfer, storage and creation of 1D ground
models. One SQLite file holds every model, every material, and the full history
of how they changed. One static binary does everything to it.

Inspired by [Fossil](https://fossil-scm.org): the repository *is* the database,
the tooling is built in, and there is nothing else to install.

## Try it

```console
$ export GM_AUTHOR="you@example.com"
$ gm init route.gm --name "A13 route models" --crs EPSG:27700 --datum "Ordnance Datum Newlyn"
$ gm import examples/thames-crossing.gm.json
$ gm commit -m "Import route models from interpretation 001"

$ gm show CH-100
CH-100  Chainage 100
  location   384100.0 E, 397200.0 N
  surface    82.50    base 62.50
  water      hydrostatic, water table 2.50 m below ground level

     Level    Depth  Thickness   Material
  ------------------------------------------------------------------
     82.50     0.00       3.00   MADE_GROUND    Made Ground
     79.50     3.00      17.00   LONDON_CLAY    London Clay
     62.50    20.00              (base of model)
```

Now re-log the borehole. The materialised tables *are* the working tree, so any
SQLite client is a valid editor:

```console
$ sqlite3 route.gm "UPDATE ground_layers SET top_level = 79.9
                    WHERE material_key = 'LONDON_CLAY'
                      AND ground_model_id = (SELECT id FROM ground_models
                                             WHERE model_key = 'CH-100');"

$ gm diff
~ model CH-100
    layer 2  top  79.50 -> 79.90  (+0.40 m)

$ gm commit -m "Raise London Clay at CH-100 following BH102 re-log"
committed 2694b207e029
```

The old revision is still there:

```console
$ gm show CH-100 --rev a567f6a06e86
```

And DuckDB reads the same file with no export step:

```console
$ duckdb -c "INSTALL sqlite; LOAD sqlite;
             ATTACH 'route.gm' AS gm (TYPE sqlite, READ_ONLY);
             SELECT model_key, material_key, top_level, thickness
             FROM gm.layer_intervals ORDER BY model_key, layer_order;"
```

## Commands

| | |
|---|---|
| `gm init <path>` | Create a file. `--name --crs --datum` |
| `gm info` | What this file is, and how big its history is |
| `gm models` / `gm materials` | List them |
| `gm show <key>` | Draw a section, or show a material's parameters |
| `gm cat <key>` | The raw versioned document |
| `gm status` | Uncommitted changes |
| `gm diff [from] [to]` | Field-level diff between revisions |
| `gm commit -m <msg>` | Record the working tree |
| `gm log` | History |
| `gm checkout <rev>` | Restore an earlier revision |
| `gm validate` | Check the models. `--json --store` |
| `gm verify` | Re-hash every object, check every reference |
| `gm import <json>` / `gm export` | Flat JSON interchange |
| `gm sql <query>` | Query the materialised tables |
| `gm ui` | Local read-only web UI. `--port` |
| `gm clone <src> [dest]` | Copy a file, history and all |
| `gm pull` / `gm push [remote]` | Sync with another copy |
| `gm merge <rev>` | Three-way merge of a diverged revision |
| `gm remote add/list/remove` | Named copies to sync with |

Revisions can be named by abbreviated hash or `HEAD`. The file is found from
`--file`, `$GM_FILE`, or the single `*.gm` in the working directory.

## Working together

`gm clone` gives each engineer their own copy with the whole history in it. A
clone keeps the source's file id, because that id names the *project*, not the
copy — two files with different ids refuse to sync, so a stray pull cannot graft
one job onto another.

```console
$ gm clone shared.gm alice.gm
$ gm clone shared.gm bob.gm
# Alice re-logs CH-100; Bob independently re-logs CH-125
$ gm -f alice.gm push
$ gm -f bob.gm pull
fetched 1 commits, 2 objects
histories have diverged:
  yours   0df04eed5a77
  theirs  349ea91eeb7d
  common  2e5d79aa74d2

run `gm merge 349ea91eeb7d` to combine them

$ gm -f bob.gm merge 349ea91eeb7d
merged 349ea91eeb7d into 0df04eed5a77
  took theirs  CH-100
committed 3150a2bc34ec
```

Different models merge automatically. Two people who both re-logged the *same*
borehole get a conflict and nothing is written — only one of them can be right,
and the tool cannot know which.

## The web UI

```console
$ gm ui
gm ui: A13 route models
       http://127.0.0.1:8765/
       read-only; press Ctrl-C to stop
```

Every model gets a drawn section — strata to scale, levels and depths on the
axis, water table where it belongs — because an inverted boundary or a stratum
that is 50 mm thick is obvious in a picture and easy to miss in a table. A
material keeps the same colour on every page, so you can recognise London Clay
along a route without reading the labels.

Also: history, per-commit diffs, which revisions touched a given model, and
validation results shown against the model they concern. `/api/export`,
`/api/models` and `/api/validate` return JSON.

It is read-only and binds to loopback only. Editing goes through the CLI, where
it gets validated and attributed to a named author, and a ground model is not
something to publish to the office network because someone typed `gm ui`.

## The browser editor

`editor/` is the static counterpart to `gm ui`, designed for GitHub Pages. It
keeps the section drawing and validation-led experience, then adds forms for
file metadata, models, layers, material properties and advanced JSON fields.
Nothing is uploaded: SQLite and JSON are opened in browser memory.

It accepts both kinds of ground-model document:

- A `.gm` SQLite repository is downloaded again with its history intact and
  the edited materialised tables as its uncommitted working tree. Continue with
  `gm diff`, `gm validate` and `gm commit` locally.
- A `.gm.json` interchange document is downloaded as stable, sorted JSON. It
  represents one revision and therefore has no repository history to preserve.

Run it locally with `just editor`, or directly:

```console
$ cd editor
$ npm install
$ npm run dev
```

The Pages workflow tests and builds the app on pushes to `main`. In the GitHub
repository settings, choose **GitHub Actions** as the Pages source once; after
that deployment is automatic. The Vite base path is relative, so the build
works at both a project Pages URL and a custom domain.

## What the format guarantees

- **A revision reads back exactly.** Round-tripping is byte-stable.
- **Unchanged content is stored once.** Moving one layer boundary in a two-model
  file adds two objects: the changed model and the new manifest.
- **The same model hashes the same everywhere.** Content identity is a function
  of the document alone, never of which file it lives in. This is what
  clone/push/pull rest on.
- **History order comes from the parent graph**, not timestamps. Two commits in
  the same second, or a skewed clock on another machine, cannot reorder history.
- **Broken models do not enter history.** `gm commit` refuses on validation
  errors.
- **`gm verify` proves it.** Every object is re-hashed and every reference
  checked.

See [`docs/format.md`](docs/format.md) for the format specification, including
the deviations from `schema.dbml` and the reason for each.

## Build

```console
$ cargo build --release   # target/release/gm, one static binary
$ cargo test
```

### Tests

Three layers, because they catch different things:

| | What it drives | |
|---|---|---|
| `crates/gm-core/tests/` | the library directly | format, object store, sync, merge, validation |
| `crates/gm-cli/tests/cli.rs` | the real binary | what you type and what comes back: stdout, stderr, exit status |
| `crates/gm-cli/tests/ui.rs` | `gm ui` over HTTP | routes, status codes, escaping, refusing to write |

The CLI tests exist because the library tests would all still pass with a
completely broken command line. A correct library behind a command that prints
the wrong number is still wrong, and messages, exit codes and table arithmetic
are the whole product as far as anyone using `gm` is concerned.

`uat/run.sh` is a fourth thing and serves a different purpose: it is for a
person to watch and judge, not for CI to check.

With [just](https://just.systems), `just` on its own lists everything:

| | |
|---|---|
| `just release` / `just install` | build, or put `gm` on your PATH |
| `just check` | the pre-commit gate: lints and tests |
| `just ci` | everything, including the examples and the walkthrough |
| `just fix` / `just fmt` | apply what clippy and rustfmt can fix |
| `just examples` | check every example still imports and validates |
| `just uat` / `just walk` | the walkthrough, or the same pausing between steps |
| `just ui` / `just sandbox` | the web UI, or a shell in the demo sandbox |
| `just editor` / `just editor-check` | browser editor, or its test + build gate |
| `just gm ...` | run gm against the sandbox, e.g. `just gm show CH-150` |

## Trying it out

```console
$ just uat              # or: uat/run.sh
```

Everything works without `just` too — the recipes are thin wrappers over
`cargo` and `uat/run.sh`:

```console
$ uat/run.sh            # a narrated walkthrough of every scenario
$ uat/run.sh --step     # the same, pausing between scenarios
$ uat/run.sh ui         # seed a file and open the web UI
$ uat/run.sh shell      # a sandbox shell with gm on PATH and $GM_FILE set
$ uat/run.sh clean      # remove the sandbox
```

It builds a six-chainage A13 corridor route — Made Ground over a buried
alluvial channel, terrace gravel and London Clay — then walks through creating
a file, editing it with `sqlite3`, history and time travel, validation refusing
a broken commit, two engineers diverging and merging, a conflict being refused,
integrity checking, and DuckDB reading the same file.

Everything happens in `uat/workspace/`, which is gitignored and rebuilt on each
run. The script asserts its own expectations — including that the things which
*should* be refused still are — and exits non-zero if anything behaves
differently, so it doubles as an end-to-end smoke test.

[`uat/CHECKLIST.md`](uat/CHECKLIST.md) is the acceptance checklist: what to look
for in each scenario, and the design decisions worth settling before they get
expensive to change.

## Status

All of it works: the format, the object store, history, diff, checkout,
validation, integrity checking, JSON interchange, clone/push/pull with
three-way merge, the web UI, and the CLI. 110 tests.

Known limits, in rough order of how much they'd matter:

- **Sync is filesystem-only.** A remote is a path, so it works over a shared
  drive or a synced folder but not over the network. The transfer logic is
  already transport-agnostic — it asks for objects by hash — so an HTTP
  transport served by `gm ui` is the obvious next step.
- **Merge conflicts must be resolved by hand**, by checking out one side and
  re-committing. There is no assisted resolution.
- **No branches.** `gm_ref` supports them and the merge machinery is
  branch-shaped, but nothing creates or switches them yet.
- **Blobs are stored uncompressed.** Canonical JSON compresses well and the
  column is ready for it; dedup already does most of the work.
- **`gm ui` is single-threaded.** Fine for one engineer on localhost.
