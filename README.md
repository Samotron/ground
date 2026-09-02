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

Revisions can be named by abbreviated hash or `HEAD`. The file is found from
`--file`, `$GM_FILE`, or the single `*.gm` in the working directory.

## What the format guarantees

- **A revision reads back exactly.** Round-tripping is byte-stable.
- **Unchanged content is stored once.** Moving one layer boundary in a two-model
  file adds two objects: the changed model and the new manifest.
- **The same model hashes the same everywhere.** Content identity is a function
  of the document alone, never of which file it lives in — which is what
  clone/push/pull will rest on.
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

## Status

Working: the format, the object store, history, diff, checkout, validation,
integrity checking, JSON interchange, and the CLI.

Not built yet: `gm ui` (the built-in web UI) and `gm clone` / `push` / `pull`
(sync between copies). The object store was designed for both — a commit is a
blob, so sync is "send me the blobs I don't have" — but neither is implemented.
