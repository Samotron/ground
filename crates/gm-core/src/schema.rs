//! The on-disk SQLite schema.
//!
//! A ground-model file has two layers, and the distinction matters:
//!
//! 1. **The object store** (`gm_*` tables). Content-addressed, append-only,
//!    and authoritative. Every historical revision of every model lives here.
//!    This is what clone/push/pull exchange.
//! 2. **The materialised view** (unprefixed tables, matching `schema.dbml`).
//!    A projection of whichever commit is currently checked out. Rebuilt from
//!    the object store on demand, and safe to drop entirely. This is the layer
//!    other tools read: plain SQL, or `ATTACH ... (TYPE sqlite)` from DuckDB.
//!
//! The materialised tables double as the working tree: edit them with `gm`, or
//! with any SQLite client you like, then `gm commit` reads them back, validates
//! them and writes new objects. `gm status` is a diff of those tables against
//! the checked-out commit.

/// `PRAGMA application_id`, so `file(1)` and any other tool can identify the
/// format without opening it. ASCII "GMDL".
pub const APPLICATION_ID: i32 = 0x474D_444C;

/// `PRAGMA user_version`. Bumped only for incompatible physical changes.
pub const USER_VERSION: i32 = 1;

/// The format version recorded in `file_metadata.schema_version`.
pub const SCHEMA_VERSION: &str = "0.1.0";

/// Object kinds tracked in `gm_entry`.
pub const KIND_MODEL: &str = "ground_model";
pub const KIND_MATERIAL: &str = "material";
pub const KIND_FILE_META: &str = "file_metadata";

pub const OBJECT_STORE_DDL: &str = r#"
-- ---------------------------------------------------------------------------
-- Object store: content-addressed and append-only.
-- ---------------------------------------------------------------------------

-- Every versioned document, keyed by the SHA-256 of its canonical JSON form.
-- Identical content stored twice occupies one row, so a file holding fifty
-- revisions of a model that only moved one layer boundary stays small.
CREATE TABLE IF NOT EXISTS gm_blob (
    hash    TEXT NOT NULL PRIMARY KEY,
    size    INTEGER NOT NULL,
    content BLOB NOT NULL
) WITHOUT ROWID;

-- Commits are themselves blobs: a commit's hash IS the hash of its manifest.
-- That is what lets sync be "exchange the blobs you are missing" and nothing
-- more. This table is a queryable index over those manifest blobs.
CREATE TABLE IF NOT EXISTS gm_commit (
    hash         TEXT NOT NULL PRIMARY KEY REFERENCES gm_blob(hash),
    author       TEXT NOT NULL,
    committed_at TEXT NOT NULL,   -- RFC 3339, always UTC
    message      TEXT NOT NULL
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS gm_commit_parent (
    commit_hash TEXT NOT NULL REFERENCES gm_commit(hash),
    parent_hash TEXT NOT NULL,
    ord         INTEGER NOT NULL,  -- 0 is the first parent
    PRIMARY KEY (commit_hash, ord)
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS gm_commit_parent_by_parent
    ON gm_commit_parent(parent_hash);

-- What each commit contains: one row per document, addressed by kind and key.
-- Keys, not surrogate ids, so a model keeps its identity across revisions.
CREATE TABLE IF NOT EXISTS gm_entry (
    commit_hash TEXT NOT NULL REFERENCES gm_commit(hash),
    kind        TEXT NOT NULL,
    key         TEXT NOT NULL,
    blob_hash   TEXT NOT NULL REFERENCES gm_blob(hash),
    PRIMARY KEY (commit_hash, kind, key)
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS gm_entry_by_key ON gm_entry(kind, key);

-- Named pointers into history. 'HEAD' is what the working tree came from.
CREATE TABLE IF NOT EXISTS gm_ref (
    name        TEXT NOT NULL PRIMARY KEY,
    kind        TEXT NOT NULL,     -- 'head' | 'branch' | 'tag'
    commit_hash TEXT
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS gm_remote (
    name TEXT NOT NULL PRIMARY KEY,
    url  TEXT NOT NULL
) WITHOUT ROWID;

-- Identity and settings for this file. Single row, id = 1.
CREATE TABLE IF NOT EXISTS gm_config (
    id             INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
    file_id        TEXT NOT NULL,
    schema_version TEXT NOT NULL,
    created_at     TEXT NOT NULL,
    default_author TEXT
);
"#;

/// The materialised view. Column names and types follow `schema.dbml`; the
/// deviations are documented in `docs/format.md` and summarised here:
///
/// * `ground_models.base_level` is new. Without it the deepest layer has no
///   base and the model has no vertical extent.
/// * `id` columns hold content hashes rather than random UUIDs, so the same
///   model materialises to the same row in every copy of the file. Random ids
///   would make two byte-identical models diff as different.
/// * `ground_layers.layer_order` is derived from array position in the model
///   document rather than stored independently, so it cannot contradict
///   `top_level`.
/// * `created_at` / `updated_at` are computed from commit history rather than
///   being editable fields.
pub const MATERIALISED_DDL: &str = r#"
DROP VIEW  IF EXISTS layer_intervals;
DROP TABLE IF EXISTS model_issues;
DROP TABLE IF EXISTS ground_layers;
DROP TABLE IF EXISTS ground_models;
DROP TABLE IF EXISTS materials;
DROP TABLE IF EXISTS file_metadata;

CREATE TABLE file_metadata (
    id              INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
    file_id         TEXT NOT NULL UNIQUE,
    name            TEXT NOT NULL,
    description     TEXT,
    schema_version  TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    crs             TEXT,
    vertical_datum  TEXT,
    metadata        TEXT
);

CREATE TABLE materials (
    id                  TEXT NOT NULL PRIMARY KEY,
    file_id             TEXT NOT NULL REFERENCES file_metadata(file_id),
    material_key        TEXT NOT NULL,
    name                TEXT,
    description         TEXT,
    soil_class          TEXT,
    properties          TEXT NOT NULL,
    constitutive_models TEXT NOT NULL,
    provenance          TEXT,
    metadata            TEXT,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);

CREATE UNIQUE INDEX materials_file_key ON materials(file_id, material_key);

CREATE TABLE ground_models (
    id            TEXT NOT NULL PRIMARY KEY,
    file_id       TEXT NOT NULL REFERENCES file_metadata(file_id),
    model_key     TEXT NOT NULL,
    name          TEXT,
    description   TEXT,
    model_type    TEXT NOT NULL DEFAULT '1d',
    surface_level REAL,
    base_level    REAL,
    x             REAL,
    y             REAL,
    gamma_w       REAL NOT NULL DEFAULT 9.81,
    groundwater   TEXT NOT NULL,
    settings      TEXT,
    metadata      TEXT,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

CREATE UNIQUE INDEX ground_models_file_key ON ground_models(file_id, model_key);
CREATE INDEX ground_models_location ON ground_models(file_id, x, y);

CREATE TABLE ground_layers (
    id                     TEXT NOT NULL PRIMARY KEY,
    ground_model_id        TEXT NOT NULL REFERENCES ground_models(id),
    layer_order            INTEGER NOT NULL,
    top_level              REAL NOT NULL,
    base_level             REAL,
    material_id            TEXT NOT NULL REFERENCES materials(id),
    material_key           TEXT NOT NULL,
    description            TEXT,
    source                 TEXT,
    generated_from_profile INTEGER NOT NULL DEFAULT 0,
    metadata               TEXT
);

CREATE UNIQUE INDEX ground_layers_order ON ground_layers(ground_model_id, layer_order);
CREATE UNIQUE INDEX ground_layers_top   ON ground_layers(ground_model_id, top_level);
CREATE INDEX ground_layers_material     ON ground_layers(material_id);

-- Validation output. Operational data, regenerated on demand, and explicitly
-- not part of the ground model itself.
CREATE TABLE model_issues (
    id              TEXT NOT NULL PRIMARY KEY,
    ground_model_id TEXT REFERENCES ground_models(id),
    model_key       TEXT,
    severity        TEXT NOT NULL CHECK (severity IN ('error', 'warning')),
    field_path      TEXT NOT NULL,
    message         TEXT NOT NULL,
    created_at      TEXT NOT NULL
);

CREATE INDEX model_issues_by_model ON model_issues(ground_model_id);

-- Convenience for consumers that want depth intervals rather than levels,
-- which is most of them. Kept as a view so it cannot drift from the tables.
CREATE VIEW layer_intervals AS
SELECT
    l.ground_model_id,
    m.model_key,
    l.layer_order,
    l.material_key,
    l.top_level,
    l.base_level,
    m.surface_level - l.top_level  AS top_depth,
    m.surface_level - l.base_level AS base_depth,
    l.top_level - l.base_level     AS thickness
FROM ground_layers l
JOIN ground_models m ON m.id = l.ground_model_id
ORDER BY m.model_key, l.layer_order;
"#;
