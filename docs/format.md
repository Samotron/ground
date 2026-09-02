# The ground-model file format

Version `0.1.0`.

A ground-model file is a single SQLite database holding one or more 1D ground
models, their materials, and the complete history of how they got that way.

There are two representations, and the difference matters:

| | What it is | Use it for |
|---|---|---|
| **`.gm` file** | SQLite. Object store + materialised tables. | Keeping, querying, versioning, syncing. |
| **`gm.file/1` JSON** | One revision, flattened. No history. | Sending to someone with no tooling. |

## Why SQLite rather than DuckDB

The `.gm` file is SQLite even though the tooling reads it happily from DuckDB.
SQLite's on-disk format is documented and explicitly committed to backwards
compatibility; it is the format you want for something a project might have to
open in twenty years. DuckDB's storage format is fast and columnar but does not
make that promise across versions, which is the wrong trade for an archival
interchange format.

Nothing is lost by this. DuckDB reads a `.gm` file directly:

```sql
INSTALL sqlite; LOAD sqlite;
ATTACH 'route.gm' AS gm (TYPE sqlite, READ_ONLY);
SELECT * FROM gm.layer_intervals;
```

## The two layers

```
  gm_blob ── gm_commit ── gm_entry ── gm_ref        authoritative, versioned
                     │
                     │  materialise
                     ▼
  file_metadata ── materials ── ground_models ── ground_layers
                                                  queryable, disposable
```

### Object store (`gm_*`)

Content-addressed and append-only, in the manner of Fossil and Git.

- **`gm_blob`** — every versioned document, keyed by the SHA-256 of its
  canonical JSON. Identical content stored twice occupies one row, so a file
  holding fifty revisions of a model whose clay boundary moved once stays small.
- **`gm_commit`** — a commit *is* a blob: its hash is the hash of its manifest.
  This table is an index over those manifest blobs, not a separate record.
- **`gm_entry`** — what each commit contains, addressed by `(kind, key)`.
- **`gm_ref`** — named pointers. `HEAD` is what the working tree came from.

Because a commit is just a blob, sync reduces to "send me the blobs I don't
have" and a commit id verifies its own contents by construction.

### Materialised tables

A projection of whichever commit is checked out. These follow `schema.dbml`, and
they are also the **working tree**: edit them with `gm`, or with any SQLite
client, and `gm commit` reads them back, validates them, and writes new objects.
`gm status` is a diff of these tables against `HEAD`.

They can be dropped and rebuilt from the object store at any time.

## Identity

Documents are addressed by **stable human keys**, not surrogate ids:

- a model by `model_key` (`CH-100`)
- a material by `material_key` (`LONDON_CLAY`)
- file metadata by the reserved key `file`

A model keeps its identity across revisions because its key does not change.
Layers have no independent identity — they exist only inside their model, are
stored as an ordered array in the model document, and their `layer_order` is
their array position, so it cannot contradict `top_level`.

## Canonical JSON

Every hash in the file is the SHA-256 of a document's canonical form, so two
implementations that agree on canonicalisation agree on every id. The rules are
RFC 8785 (JCS):

- UTF-8, no insignificant whitespace.
- Object keys sorted by **UTF-16 code unit**, not UTF-8 byte. These differ for
  characters outside the Basic Multilingual Plane.
- Numbers formatted by ECMAScript `Number::toString`. `1e21` serialises as
  `1e+21`, `0.0000001` as `1e-7`, and `-0.0` as `0`.
- Shortest legal string escapes.

Object ids are written `sha256-<64 lowercase hex>`.

## Deviations from `schema.dbml`

The original DBML is the starting point; these are the changes and why.

**`ground_models.base_level` added.** In the original, `ground_layers` carried
only `top_level`, so the deepest layer had no bottom and a model had no vertical
extent. The base of layer *i* is the top of layer *i+1*, and the base of the last
layer is the model's `base_level`. Gaps between layers are therefore structurally
impossible rather than something a validator has to catch.

**`id` columns hold content hashes, not UUIDs.** The DBML specified "UUID or
ULID". Random ids would mean the same model materialises to a different row in
every copy of the file, so two byte-identical files would diff as different and
sync would have nothing stable to match on. A model's `id` is the hash of its
document; a layer's is `<model id>:<3-digit order>`. `model_key` remains the
human handle and is in the same row.

**`created_at` / `updated_at` are derived, not stored fields.** They are
computed from commit history: `created_at` is the commit where the key first
appeared, `updated_at` the commit where its content last changed. If they lived
in the document, every save would change the document's hash even when nothing
about the ground changed, which would defeat deduplication and fill diffs with
noise. This also means they cannot be wrong.

**`layer_order` is derived from array position** rather than being an
independently stored ordinal that could fall out of step with the levels.

**`Profile` gained a `datum`.** The original example had a London Clay
undrained-strength profile with points at "depth" 5 and 20. Materials are
file-scoped and reused across models, so a depth below ground level means
something different in every model that uses the material. `datum` is one of
`layer-top` (default — the only one that travels correctly with a shared
material), `ground-level`, or `level` (absolute, read as a level increasing
upwards).

**`model_issues` is not part of the model.** It is regenerated validation output,
written only by `gm validate --store`.

## The document types

### `FileMetadata`

```json
{
  "name": "A13 route models",
  "description": "…",
  "crs": "EPSG:27700",
  "verticalDatum": "Ordnance Datum Newlyn",
  "metadata": {}
}
```

Every level in the file is in `verticalDatum`; every coordinate is in `crs`.
Without them the numbers are ambiguous, so both are warned about if absent.

### `Bounded` — the numeric primitive

A ground model is an interpretation, and a bare number cannot say how well
constrained it is. Every quantity is:

```json
{ "value": 19.0, "lower": 17.0, "upper": 21.0, "unit": "kN/m3" }
```

`value` is the best estimate; `lower`/`upper` a credible range. `unit` is free
text by design — pinning a unit ontology is a bigger fight than this format
should pick. A quantity may instead (or additionally) carry a `profile`:

```json
{
  "unit": "kPa",
  "profile": {
    "interpolation": "linear",
    "datum": "layer-top",
    "points": [
      { "depth": 0,  "value": 60,  "lower": 50, "upper": 70 },
      { "depth": 15, "value": 120, "lower": 95, "upper": 145 }
    ]
  }
}
```

`interpolation` is `linear` or `step`. When a profile is present it is
authoritative and any scalar `value` is a representative summary.

### `Material`

File-scoped and reusable across models. Carries general `properties`
(`unitWeight`, `permeability`, …) and an array of `constitutiveModels`:

```json
{
  "id": "mc-01",
  "kind": "mohr-coulomb",
  "drainage": "drained",
  "parameters": {
    "frictionAngleDeg": { "value": 30, "lower": 27, "upper": 33, "unit": "deg" }
  }
}
```

`kind` is an **open set**. Kinds this build knows are checked for the parameters
they need; unknown kinds are carried through untouched and warned about once, so
a file written by a newer tool still round-trips through an older one intact.

### The suggested vocabulary

`assets/vocabulary.json` lists the property names, unit strings, soil classes
and constitutive kinds that come up over and over, with the unit each quantity
is normally given in. It is advisory, not a schema: none of those four are
closed sets and anything outside the file is valid and round-trips untouched.

It exists because the openness has a cost — `kN/m2` where `kN/m3` was meant is a
value no validator can catch — so the browser editor offers the list first and
takes free text second. One part of it is load-bearing: `constitutiveKinds` is
the set `gm_core` checks parameters for, read from this file rather than copied,
so the editor cannot offer a kind that `gm validate` would then warn about.

### `GroundModel`

One vertical succession at one location. `layers` is ordered top-down; each
layer gives its `topLevel` and a `materialKey`.

`groundwater` is tagged by `kind`: `dry`, `hydrostatic` (with `depth` below
ground level), `piezometric` (with a pore-pressure `profile`), or `unknown`.
`unknown` is deliberately distinct from `dry`, which is an assertion.

## Validation

Two severities, on one principle: an **error** means the file does not describe a
coherent ground model and a consumer would be wrong to use it. A **warning**
means it is coherent but suspicious. Ground is legitimately odd often enough
that the bar for an error is high — a validator that rejects plausible ground is
one that gets switched off.

Errors include: layers out of order or zero-thickness; a base level above the
deepest layer; a layer referencing an undefined material; a value outside its own
bounds; a friction angle outside 0–90°.

Warnings include: no vertical datum; no surface or base level; unknown
groundwater; missing unit weight; a unit weight outside 10–30 kN/m³; a parameter
with no unit; an unused material; an unrecognised constitutive kind.

`gm commit` refuses to record a revision with errors in it. A history full of
invalid states is worse than no history, because it removes the one thing a
consumer could rely on.

## Syncing

Two copies of a file may exchange history when they share a `file_id`. That id
names the project, not the copy: `gm clone` preserves it, and files with
different ids refuse to sync, which stops one project being grafted onto
another.

The protocol is one rule: **send the objects the other side does not have.**
Because a commit is a blob whose hash is the hash of its manifest, walking a
commit's ancestry and copying missing blobs transfers history, models and
materials through one path. Objects move as raw bytes, and a receiver rejects
anything that does not hash to its own name, so corruption cannot propagate.

Divergence is resolved by a three-way merge over documents keyed by
`(kind, key)`, using the standard rule: a side that did not change from the
merge base loses to the side that did; both sides changing to the same value is
agreement; both sides changing to different values is a conflict. Conflicts
write nothing.

A merge commit records both revisions as parents. History is therefore a DAG,
and ordering it by `committed_at` would be wrong — see below.

### Bundles

Objects move between copies as a **bundle**: a bag of objects, with no ordering
guarantees, no deltas and no compression.

```
gm-bundle/1\n
sha256-<hex> <byte length>\n
<that many bytes>
sha256-<hex> <byte length>\n
<that many bytes>
...
```

Lengths are byte counts, so payloads need no escaping and a decoder never has to
guess where an object ends. Every object carries its own hash, so a receiver
verifies each one independently and a truncated or reordered transfer is caught
rather than half-applied.

The absence of delta encoding is deliberate. Deduplication has already done that
work: a revision of a six-model route in which one layer boundary moved is two
objects — the changed model document and the new manifest — regardless of how
large the rest of the file is.

### Over HTTP

`gm serve` exposes four endpoints. There is no session state; each request
stands alone.

| | |
|---|---|
| `GET /sync/info` | JSON: protocol version, `fileId`, `schemaVersion`, `head`, `name`, `acceptsPush` |
| `GET /sync/commits` | every commit hash this copy holds, one per line |
| `POST /sync/bundle` | body: the caller's commit hashes, one per line. Returns a bundle of everything reachable from this copy's head that the caller lacks |
| `POST /sync/push` | body: a bundle. Header `X-GM-Head`: the new head. Applies it if that is a fast-forward |

A peer holding a commit is proof that it also holds every document that commit
references, so `POST /sync/bundle` excludes those blobs exactly rather than
re-sending them. That is what keeps a routine pull down to what actually
changed.

`POST /sync/push` refuses (409) anything that is not a fast-forward, and refuses
to write over a remote whose own working tree is dirty. It never forces: only
the pusher can see both sides of a divergence, so merging is their job.

Serving a bundle discloses nothing the UI pages do not already show, so it needs
no permission beyond being reachable. Accepting a push is a write to someone
else's file, so it requires `--allow-push`.

**Transport security is out of scope.** This is plain HTTP: `--token` sets a
shared secret checked as `Authorization: Bearer <token>`, but nothing is
encrypted, and `https://` is refused rather than silently downgraded. It is
built for a network you already trust — a LAN, a VPN, or a tunnel you put in
front of it.

## History ordering

`gm log` orders commits **topologically**: descendants before ancestors, from
the parent graph. Commit time breaks ties only between commits the graph leaves
genuinely unordered, such as the two sides of a merge.

Timestamps cannot be the primary ordering. Two commits made in the same second
would order arbitrarily, and once files sync between machines a skewed clock
could place a child before its own parent. The parent graph is the only thing
that actually knows what came first.

## Presentation

Not part of the format, but shared by the tools that render it, and worth
recording because two implementations of the same rules drift.

**Material colour** is derived from the material key so that a stratum keeps the
same colour on every page and in every tool: FNV-1a over the key's UTF-8 bytes,
multiplied by 137 (a golden-angle step, so adjacent keys land far apart on the
wheel), taken modulo 360, then clamped to at least 1.

```
hue(key) = max(1, (fnv1a(key) * 137 mod 2^32) mod 360)
```

The order matters: clamping before the modulo leaves 0 reachable and disagrees
with this rule for roughly one key in 360.

`assets/conformance.json` pins the agreed hues and the exact set of validation
issues a known-bad document must produce. Both the Rust core and the browser
editor read it in their test suites.

## Identifying a file

- `PRAGMA application_id` is `0x474D444C` (ASCII `GMDL`).
- `PRAGMA user_version` is the physical schema version, currently `1`.
- `file_metadata.schema_version` carries the format version string.

A file whose `user_version` is higher than the reading build understands is
rejected rather than misread.
