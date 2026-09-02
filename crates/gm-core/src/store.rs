//! The repository: object store, history, and the materialised working tree.

use crate::canon;
use crate::commit::{Manifest, ManifestEntry};
use crate::error::{Error, Result, invalid};
use crate::model::{FileMetadata, GroundModel, Groundwater, Layer, Material, Settings};
use crate::schema::{
    self, KIND_FILE_META, KIND_MATERIAL, KIND_MODEL, MATERIALISED_DDL, OBJECT_STORE_DDL,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::Path;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// The single reserved key of the file-metadata document. There is exactly one
/// per file, but it is versioned like everything else so that renaming a
/// project or correcting a datum shows up in the history.
const FILE_META_KEY: &str = "file";

/// Current UTC time, RFC 3339, truncated to whole seconds. Sub-second precision
/// buys nothing here and makes commit ids noisier to read.
pub fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .expect("zero is always a valid nanosecond")
        .format(&Rfc3339)
        .expect("UTC always formats as RFC 3339")
}

/// A complete revision: everything the file contains at one point in history.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub file_metadata: FileMetadata,
    pub materials: BTreeMap<String, Material>,
    pub models: BTreeMap<String, GroundModel>,
}

impl State {
    pub fn empty(file_metadata: FileMetadata) -> Self {
        Self {
            file_metadata,
            materials: BTreeMap::new(),
            models: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

impl ChangeKind {
    pub fn marker(self) -> char {
        match self {
            ChangeKind::Added => 'A',
            ChangeKind::Modified => 'M',
            ChangeKind::Deleted => 'D',
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// One of the `KIND_*` constants.
    pub kind: String,
    pub key: String,
    pub change: ChangeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    pub hash: String,
    pub author: String,
    pub committed_at: String,
    pub message: String,
    pub parents: Vec<String>,
}

impl CommitInfo {
    /// Enough of the hash to be unambiguous in practice, for display and for
    /// accepting abbreviated ids on the command line.
    pub fn short(&self) -> String {
        short_hash(&self.hash)
    }
}

pub fn short_hash(hash: &str) -> String {
    let hex = hash.strip_prefix("sha256-").unwrap_or(hash);
    hex.chars().take(12).collect()
}

#[derive(Debug)]
pub struct Repository {
    conn: Connection,
    file_id: String,
}

impl Repository {
    /// Create a new ground-model file and record the root commit.
    pub fn create(path: impl AsRef<Path>, meta: FileMetadata, author: &str) -> Result<Self> {
        let created = now_rfc3339();
        let mut repo = Self::create_empty(path, &ulid::Ulid::new().to_string(), author)?;
        // Start with a root commit so that HEAD is never null and every later
        // operation has a parent to diff against.
        let state = State::empty(meta);
        repo.materialise(&state, &HashMap::new(), &created)?;
        repo.commit(author, "Initialise ground-model file")?;
        Ok(repo)
    }

    /// Create a file with the schema in place but no history at all.
    ///
    /// `file_id` identifies the *project*, not the copy, so a clone keeps the
    /// id of the file it came from. That is what lets two copies recognise each
    /// other as the same thing when they sync.
    pub fn create_empty(path: impl AsRef<Path>, file_id: &str, author: &str) -> Result<Self> {
        let path = path.as_ref();
        if path.exists() {
            return Err(invalid(format!("{} already exists", path.display())));
        }
        let conn = Connection::open(path)?;
        Self::init_connection(&conn)?;

        conn.execute(
            "INSERT INTO gm_config (id, file_id, schema_version, created_at, default_author)
             VALUES (1, ?1, ?2, ?3, ?4)",
            params![file_id, schema::SCHEMA_VERSION, now_rfc3339(), author],
        )?;
        conn.execute(
            "INSERT INTO gm_ref (name, kind, commit_hash) VALUES ('HEAD', 'head', NULL)",
            [],
        )?;

        Ok(Repository {
            conn,
            file_id: file_id.to_string(),
        })
    }

    /// Open an existing ground-model file.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(Error::NotARepository(path.display().to_string()));
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "foreign_keys", true)?;

        let app_id: i32 = conn.pragma_query_value(None, "application_id", |r| r.get(0))?;
        if app_id != schema::APPLICATION_ID {
            return Err(Error::NotARepository(path.display().to_string()));
        }
        let user_version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if user_version > schema::USER_VERSION {
            return Err(Error::SchemaVersion {
                found: user_version.to_string(),
                supported: schema::USER_VERSION.to_string(),
            });
        }

        let file_id: String =
            conn.query_row("SELECT file_id FROM gm_config WHERE id = 1", [], |r| {
                r.get(0)
            })?;
        Ok(Repository { conn, file_id })
    }

    fn init_connection(conn: &Connection) -> Result<()> {
        // WAL keeps readers (a running `gm ui`, a DuckDB ATTACH) from blocking
        // a commit. `synchronous = NORMAL` is the standard WAL pairing.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "application_id", schema::APPLICATION_ID)?;
        conn.pragma_update(None, "user_version", schema::USER_VERSION)?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.execute_batch(OBJECT_STORE_DDL)?;
        conn.execute_batch(MATERIALISED_DDL)?;
        Ok(())
    }

    pub fn file_id(&self) -> &str {
        &self.file_id
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub fn default_author(&self) -> Result<Option<String>> {
        Ok(self.conn.query_row(
            "SELECT default_author FROM gm_config WHERE id = 1",
            [],
            |r| r.get(0),
        )?)
    }

    pub fn set_default_author(&self, author: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE gm_config SET default_author = ?1 WHERE id = 1",
            params![author],
        )?;
        Ok(())
    }

    // -- object store -------------------------------------------------------

    /// Store a document, returning its hash. Storing the same content twice is
    /// a no-op, which is what keeps a long history cheap.
    pub fn put_blob(&self, value: &Value) -> Result<String> {
        let canonical = canon::canonicalize(value)?;
        let hash = canon::hash_bytes(canonical.as_bytes());
        self.conn.execute(
            "INSERT OR IGNORE INTO gm_blob (hash, size, content) VALUES (?1, ?2, ?3)",
            params![hash, canonical.len() as i64, canonical.as_bytes()],
        )?;
        Ok(hash)
    }

    /// Store a blob by its raw bytes, verifying that they hash to `hash`.
    ///
    /// Sync uses this rather than re-serialising the document: copying the
    /// exact bytes keeps the object's hash valid even if a future build changed
    /// its serialiser, and it means a corrupt object cannot be laundered into a
    /// clone by being re-written on the way through.
    pub fn put_blob_raw(&self, hash: &str, bytes: &[u8]) -> Result<()> {
        let actual = canon::hash_bytes(bytes);
        if actual != hash {
            return Err(Error::CorruptObject {
                hash: hash.to_string(),
                actual,
            });
        }
        self.conn.execute(
            "INSERT OR IGNORE INTO gm_blob (hash, size, content) VALUES (?1, ?2, ?3)",
            params![hash, bytes.len() as i64, bytes],
        )?;
        Ok(())
    }

    pub fn has_commit(&self, hash: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM gm_commit WHERE hash = ?1",
                params![hash],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn has_blob(&self, hash: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM gm_blob WHERE hash = ?1",
                params![hash],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn get_blob_bytes(&self, hash: &str) -> Result<Vec<u8>> {
        self.conn
            .query_row(
                "SELECT content FROM gm_blob WHERE hash = ?1",
                params![hash],
                |r| stored_bytes(r.get_ref(0)?),
            )
            .optional()?
            .ok_or_else(|| Error::MissingObject(hash.to_string()))
    }

    pub fn get_blob(&self, hash: &str) -> Result<Value> {
        let bytes = self.get_blob_bytes(hash)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub fn manifest(&self, commit_hash: &str) -> Result<Manifest> {
        Ok(serde_json::from_value(self.get_blob(commit_hash)?)?)
    }

    // -- refs ---------------------------------------------------------------

    pub fn head(&self) -> Result<Option<String>> {
        Ok(self.conn.query_row(
            "SELECT commit_hash FROM gm_ref WHERE name = 'HEAD'",
            [],
            |r| r.get(0),
        )?)
    }

    /// Move HEAD without touching the working tree. Sync uses this; ordinary
    /// callers want [`Repository::checkout`], which also rebuilds the tables.
    pub fn set_head(&self, commit_hash: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE gm_ref SET commit_hash = ?1 WHERE name = 'HEAD'",
            params![commit_hash],
        )?;
        Ok(())
    }

    pub fn remotes(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, url FROM gm_remote ORDER BY name")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn remote_url(&self, name: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT url FROM gm_remote WHERE name = ?1",
                params![name],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn set_remote(&self, name: &str, url: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO gm_remote (name, url) VALUES (?1, ?2)
             ON CONFLICT(name) DO UPDATE SET url = excluded.url",
            params![name, url],
        )?;
        Ok(())
    }

    pub fn remove_remote(&self, name: &str) -> Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM gm_remote WHERE name = ?1", params![name])?
            > 0)
    }

    /// Resolve an abbreviated commit id, or `HEAD`, to a full hash.
    pub fn resolve(&self, rev: &str) -> Result<String> {
        if rev.eq_ignore_ascii_case("head") {
            return self
                .head()?
                .ok_or_else(|| Error::NotFound("commit", rev.to_string()));
        }
        if let Some(hash) = self
            .conn
            .query_row(
                "SELECT commit_hash FROM gm_ref WHERE name = ?1",
                params![rev],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten()
        {
            return Ok(hash);
        }

        let needle = rev.strip_prefix("sha256-").unwrap_or(rev);
        let mut stmt = self
            .conn
            .prepare("SELECT hash FROM gm_commit WHERE hash LIKE 'sha256-' || ?1 || '%'")?;
        let matches: Vec<String> = stmt
            .query_map(params![needle], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        match matches.len() {
            0 => Err(Error::NotFound("commit", rev.to_string())),
            1 => Ok(matches.into_iter().next().expect("length checked")),
            n => Err(invalid(format!("{rev} is ambiguous: {n} commits match"))),
        }
    }

    // -- history ------------------------------------------------------------

    pub fn commit_info(&self, hash: &str) -> Result<CommitInfo> {
        let (author, committed_at, message) = self
            .conn
            .query_row(
                "SELECT author, committed_at, message FROM gm_commit WHERE hash = ?1",
                params![hash],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound("commit", hash.to_string()))?;

        let mut stmt = self.conn.prepare(
            "SELECT parent_hash FROM gm_commit_parent WHERE commit_hash = ?1 ORDER BY ord",
        )?;
        let parents: Vec<String> = stmt
            .query_map(params![hash], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;

        Ok(CommitInfo {
            hash: hash.to_string(),
            author,
            committed_at,
            message,
            parents,
        })
    }

    /// Ancestry of `start`, descendants before ancestors.
    ///
    /// The order is topological, not chronological. Sorting by `committed_at`
    /// would be wrong twice over: two commits made inside the same second would
    /// order arbitrarily, and once files sync between machines a skewed clock
    /// could place a child before its own parent. The parent graph is the only
    /// thing that actually knows what came first, so it decides, and commit time
    /// is used only to break ties between commits that the graph leaves
    /// genuinely unordered — the two sides of a merge, say.
    pub fn ancestry(&self, start: &str) -> Result<Vec<CommitInfo>> {
        // Collect the reachable set first.
        let mut reachable: HashMap<String, CommitInfo> = HashMap::new();
        let mut queue = VecDeque::from([start.to_string()]);
        while let Some(hash) = queue.pop_front() {
            if reachable.contains_key(&hash) {
                continue;
            }
            let info = self.commit_info(&hash)?;
            queue.extend(info.parents.iter().cloned());
            reachable.insert(hash, info);
        }

        // A commit is ready to emit once every commit that lists it as a parent
        // has already been emitted.
        let mut pending_children: HashMap<&str, usize> =
            reachable.keys().map(|h| (h.as_str(), 0usize)).collect();
        for info in reachable.values() {
            for parent in &info.parents {
                if let Some(count) = pending_children.get_mut(parent.as_str()) {
                    *count += 1;
                }
            }
        }

        // Max-heap on (committed_at, hash): among commits the graph leaves
        // unordered, prefer the most recent, and fall back to the hash so the
        // result is deterministic.
        let mut ready: std::collections::BinaryHeap<(String, String)> = reachable
            .values()
            .filter(|i| pending_children[i.hash.as_str()] == 0)
            .map(|i| (i.committed_at.clone(), i.hash.clone()))
            .collect();

        let mut out = Vec::with_capacity(reachable.len());
        while let Some((_, hash)) = ready.pop() {
            let info = &reachable[&hash];
            for parent in &info.parents {
                if let Some(count) = pending_children.get_mut(parent.as_str()) {
                    *count -= 1;
                    if *count == 0 {
                        let parent_info = &reachable[parent.as_str()];
                        ready.push((parent_info.committed_at.clone(), parent_info.hash.clone()));
                    }
                }
            }
            out.push(info.clone());
        }

        debug_assert_eq!(
            out.len(),
            reachable.len(),
            "commit graph contains a cycle, which content addressing should make impossible"
        );
        Ok(out)
    }

    pub fn log(&self, limit: Option<usize>) -> Result<Vec<CommitInfo>> {
        let Some(head) = self.head()? else {
            return Ok(Vec::new());
        };
        let mut all = self.ancestry(&head)?;
        if let Some(limit) = limit {
            all.truncate(limit);
        }
        Ok(all)
    }

    /// When each document first appeared and when it last changed, derived from
    /// history. These are the `created_at` / `updated_at` of the materialised
    /// tables: timestamps belong to commits, not to the documents themselves,
    /// so that re-saving an unchanged model does not fabricate an edit.
    fn history_timestamps(
        &self,
        head: Option<&str>,
    ) -> Result<HashMap<(String, String), (String, String)>> {
        let mut out: HashMap<(String, String), (String, String)> = HashMap::new();
        let Some(head) = head else { return Ok(out) };

        let mut chain = self.ancestry(head)?;
        chain.reverse(); // oldest first
        let mut last_blob: HashMap<(String, String), String> = HashMap::new();

        for info in chain {
            let mut stmt = self
                .conn
                .prepare("SELECT kind, key, blob_hash FROM gm_entry WHERE commit_hash = ?1")?;
            let rows: Vec<(String, String, String)> = stmt
                .query_map(params![info.hash], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?
                .collect::<rusqlite::Result<_>>()?;

            for (kind, key, blob) in rows {
                let id = (kind, key);
                match last_blob.get(&id) {
                    None => {
                        out.insert(
                            id.clone(),
                            (info.committed_at.clone(), info.committed_at.clone()),
                        );
                        last_blob.insert(id, blob);
                    }
                    Some(prev) if *prev != blob => {
                        if let Some(entry) = out.get_mut(&id) {
                            entry.1 = info.committed_at.clone();
                        }
                        last_blob.insert(id, blob);
                    }
                    Some(_) => {}
                }
            }
        }
        Ok(out)
    }

    // -- reading a revision -------------------------------------------------

    pub fn state_at(&self, commit_hash: &str) -> Result<State> {
        let manifest = self.manifest(commit_hash)?;
        let mut file_metadata = None;
        let mut materials = BTreeMap::new();
        let mut models = BTreeMap::new();

        for entry in &manifest.entries {
            let doc = self.get_blob(&entry.blob)?;
            match entry.kind.as_str() {
                KIND_FILE_META => file_metadata = Some(serde_json::from_value(doc)?),
                KIND_MATERIAL => {
                    materials.insert(entry.key.clone(), serde_json::from_value(doc)?);
                }
                KIND_MODEL => {
                    models.insert(entry.key.clone(), serde_json::from_value(doc)?);
                }
                other => {
                    // Forward compatibility: a newer tool may record kinds we
                    // do not model. Ignore them for reading rather than refuse
                    // to open the file.
                    let _ = other;
                }
            }
        }

        Ok(State {
            file_metadata: file_metadata
                .ok_or_else(|| invalid(format!("commit {commit_hash} has no file metadata")))?,
            materials,
            models,
        })
    }

    // -- the working tree ---------------------------------------------------

    /// Read the materialised tables back into documents. This is the inverse of
    /// [`Repository::materialise`], and it is what makes "edit the SQLite file
    /// with whatever tool you like, then commit" work.
    pub fn working(&self) -> Result<State> {
        let file_metadata: FileMetadata = self.conn.query_row(
            "SELECT name, description, crs, vertical_datum, metadata
             FROM file_metadata WHERE id = 1",
            [],
            |r| {
                Ok(FileMetadata {
                    name: r.get(0)?,
                    description: r.get(1)?,
                    crs: r.get(2)?,
                    vertical_datum: r.get(3)?,
                    metadata: parse_opt_json(r.get::<_, Option<String>>(4)?),
                })
            },
        )?;

        let mut materials = BTreeMap::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT material_key, name, description, soil_class, properties,
                        constitutive_models, provenance, metadata
                 FROM materials ORDER BY material_key",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<String>>(7)?,
                ))
            })?;
            for row in rows {
                let (key, name, description, soil_class, props, cms, provenance, metadata) = row?;
                materials.insert(
                    key.clone(),
                    Material {
                        material_key: key,
                        name,
                        description,
                        soil_class,
                        properties: serde_json::from_str(&props)?,
                        constitutive_models: serde_json::from_str(&cms)?,
                        provenance: parse_opt_json(provenance),
                        metadata: parse_opt_json(metadata),
                    },
                );
            }
        }

        let mut models: BTreeMap<String, GroundModel> = BTreeMap::new();
        let mut model_ids: Vec<(String, String)> = Vec::new(); // (id, model_key)
        {
            let mut stmt = self.conn.prepare(
                "SELECT id, model_key, name, description, model_type, surface_level,
                        base_level, x, y, gamma_w, groundwater, settings, metadata
                 FROM ground_models ORDER BY model_key",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<f64>>(5)?,
                    r.get::<_, Option<f64>>(6)?,
                    r.get::<_, Option<f64>>(7)?,
                    r.get::<_, Option<f64>>(8)?,
                    r.get::<_, f64>(9)?,
                    r.get::<_, String>(10)?,
                    r.get::<_, Option<String>>(11)?,
                    r.get::<_, Option<String>>(12)?,
                ))
            })?;
            for row in rows {
                let (
                    id,
                    key,
                    name,
                    description,
                    model_type,
                    surface,
                    base,
                    x,
                    y,
                    gw,
                    water,
                    settings,
                    metadata,
                ) = row?;
                let groundwater: Groundwater = serde_json::from_str(&water)?;
                let settings: Settings = match settings {
                    Some(s) => serde_json::from_str(&s)?,
                    None => Settings::default(),
                };
                model_ids.push((id, key.clone()));
                models.insert(
                    key.clone(),
                    GroundModel {
                        model_key: key,
                        name,
                        description,
                        model_type,
                        surface_level: surface,
                        base_level: base,
                        x,
                        y,
                        gamma_w: gw,
                        groundwater,
                        settings,
                        metadata: parse_opt_json(metadata),
                        layers: Vec::new(),
                    },
                );
            }
        }

        {
            let mut stmt = self.conn.prepare(
                "SELECT top_level, material_key, description, source,
                        generated_from_profile, metadata
                 FROM ground_layers WHERE ground_model_id = ?1 ORDER BY layer_order",
            )?;
            for (id, key) in &model_ids {
                let rows = stmt.query_map(params![id], |r| {
                    Ok((
                        r.get::<_, f64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, Option<String>>(5)?,
                    ))
                })?;
                let model = models
                    .get_mut(key)
                    .expect("id list came from the same query");
                for row in rows {
                    let (top_level, material_key, description, source, generated, metadata) = row?;
                    model.layers.push(Layer {
                        top_level,
                        material_key,
                        description,
                        source: parse_opt_json(source),
                        generated_from_profile: generated != 0,
                        metadata: parse_opt_json(metadata),
                    });
                }
            }
        }

        Ok(State {
            file_metadata,
            materials,
            models,
        })
    }

    /// Rewrite the materialised tables from `state`.
    fn materialise(
        &mut self,
        state: &State,
        timestamps: &HashMap<(String, String), (String, String)>,
        fallback_time: &str,
    ) -> Result<()> {
        let file_id = self.file_id.clone();
        let created_at: String = self
            .conn
            .query_row("SELECT created_at FROM gm_config WHERE id = 1", [], |r| {
                r.get(0)
            })
            .optional()?
            .unwrap_or_else(|| fallback_time.to_string());

        let tx = self.conn.transaction()?;
        tx.execute_batch(MATERIALISED_DDL)?;

        let (meta_created, meta_updated) = timestamps
            .get(&(KIND_FILE_META.to_string(), FILE_META_KEY.to_string()))
            .cloned()
            .unwrap_or_else(|| (created_at.clone(), fallback_time.to_string()));

        tx.execute(
            "INSERT INTO file_metadata
                (id, file_id, name, description, schema_version, created_at,
                 updated_at, crs, vertical_datum, metadata)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                file_id,
                state.file_metadata.name,
                state.file_metadata.description,
                schema::SCHEMA_VERSION,
                meta_created,
                meta_updated,
                state.file_metadata.crs,
                state.file_metadata.vertical_datum,
                to_opt_json(&state.file_metadata.metadata)?,
            ],
        )?;

        let mut material_ids: HashMap<String, String> = HashMap::new();
        for (key, material) in &state.materials {
            let id = canon::hash_value(&serde_json::to_value(material)?)?;
            let (created, updated) = timestamps
                .get(&(KIND_MATERIAL.to_string(), key.clone()))
                .cloned()
                .unwrap_or_else(|| (fallback_time.to_string(), fallback_time.to_string()));
            tx.execute(
                "INSERT INTO materials
                    (id, file_id, material_key, name, description, soil_class,
                     properties, constitutive_models, provenance, metadata,
                     created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    id,
                    file_id,
                    material.material_key,
                    material.name,
                    material.description,
                    material.soil_class,
                    serde_json::to_string(&material.properties)?,
                    serde_json::to_string(&material.constitutive_models)?,
                    to_opt_json(&material.provenance)?,
                    to_opt_json(&material.metadata)?,
                    created,
                    updated,
                ],
            )?;
            material_ids.insert(key.clone(), id);
        }

        for (key, model) in &state.models {
            let model_id = canon::hash_value(&serde_json::to_value(model)?)?;
            let (created, updated) = timestamps
                .get(&(KIND_MODEL.to_string(), key.clone()))
                .cloned()
                .unwrap_or_else(|| (fallback_time.to_string(), fallback_time.to_string()));
            tx.execute(
                "INSERT INTO ground_models
                    (id, file_id, model_key, name, description, model_type,
                     surface_level, base_level, x, y, gamma_w, groundwater,
                     settings, metadata, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                params![
                    model_id,
                    file_id,
                    model.model_key,
                    model.name,
                    model.description,
                    model.model_type,
                    model.surface_level,
                    model.base_level,
                    model.x,
                    model.y,
                    model.gamma_w,
                    serde_json::to_string(&model.groundwater)?,
                    serde_json::to_string(&model.settings)?,
                    to_opt_json(&model.metadata)?,
                    created,
                    updated,
                ],
            )?;

            for (index, layer) in model.layers.iter().enumerate() {
                // A layer referencing an unknown material would violate the
                // foreign key anyway; failing here gives a message that names
                // the model and the material instead of a constraint number.
                let material_id = material_ids.get(&layer.material_key).ok_or_else(|| {
                    invalid(format!(
                        "model '{}' layer {} references unknown material '{}'",
                        key,
                        index + 1,
                        layer.material_key
                    ))
                })?;
                tx.execute(
                    "INSERT INTO ground_layers
                        (id, ground_model_id, layer_order, top_level, base_level,
                         material_id, material_key, description, source,
                         generated_from_profile, metadata)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        format!("{model_id}:{:03}", index + 1),
                        model_id,
                        (index + 1) as i64,
                        layer.top_level,
                        model.layer_base(index),
                        material_id,
                        layer.material_key,
                        layer.description,
                        to_opt_json(&layer.source)?,
                        layer.generated_from_profile as i64,
                        to_opt_json(&layer.metadata)?,
                    ],
                )?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// Replace the working tree with the contents of `state`, deriving
    /// timestamps from history where it knows them.
    pub fn write_working(&mut self, state: &State) -> Result<()> {
        let head = self.head()?;
        let timestamps = self.history_timestamps(head.as_deref())?;
        self.materialise(state, &timestamps, &now_rfc3339())
    }

    // -- status, commit, checkout -------------------------------------------

    /// Documents that differ between the working tree and HEAD.
    pub fn status(&self) -> Result<Vec<Change>> {
        let working = self.working()?;
        let head_state = match self.head()? {
            Some(head) => Some(self.state_at(&head)?),
            None => None,
        };

        let mut changes = Vec::new();
        let (old_meta, old_materials, old_models) = match &head_state {
            Some(s) => (
                Some(&s.file_metadata),
                s.materials.clone(),
                s.models.clone(),
            ),
            None => (None, BTreeMap::new(), BTreeMap::new()),
        };

        match old_meta {
            Some(old) if *old != working.file_metadata => changes.push(Change {
                kind: KIND_FILE_META.into(),
                key: FILE_META_KEY.into(),
                change: ChangeKind::Modified,
            }),
            None => changes.push(Change {
                kind: KIND_FILE_META.into(),
                key: FILE_META_KEY.into(),
                change: ChangeKind::Added,
            }),
            Some(_) => {}
        }

        diff_maps(
            &old_materials,
            &working.materials,
            KIND_MATERIAL,
            &mut changes,
        );
        diff_maps(&old_models, &working.models, KIND_MODEL, &mut changes);
        Ok(changes)
    }

    /// Write the working tree as a new commit. Returns `None` when nothing
    /// changed, so committing twice in a row is harmless rather than producing
    /// an empty revision.
    pub fn commit(&mut self, author: &str, message: &str) -> Result<Option<String>> {
        let parent = self.head()?;
        if parent.is_some() && self.status()?.is_empty() {
            return Ok(None);
        }
        let parents = parent.into_iter().collect();
        Ok(Some(self.commit_with_parents(parents, author, message)?))
    }

    /// Record the working tree with explicit parents, unconditionally.
    ///
    /// Merges need this: a merge commit has two parents, and it must be
    /// recorded even when the merged result happens to equal one of the sides,
    /// because the point of the commit is to record that the two histories are
    /// now joined.
    pub fn commit_with_parents(
        &mut self,
        parents: Vec<String>,
        author: &str,
        message: &str,
    ) -> Result<String> {
        let state = self.working()?;
        let mut entries = vec![ManifestEntry {
            kind: KIND_FILE_META.into(),
            key: FILE_META_KEY.into(),
            blob: self.put_blob(&serde_json::to_value(&state.file_metadata)?)?,
        }];
        for (key, material) in &state.materials {
            entries.push(ManifestEntry {
                kind: KIND_MATERIAL.into(),
                key: key.clone(),
                blob: self.put_blob(&serde_json::to_value(material)?)?,
            });
        }
        for (key, model) in &state.models {
            entries.push(ManifestEntry {
                kind: KIND_MODEL.into(),
                key: key.clone(),
                blob: self.put_blob(&serde_json::to_value(model)?)?,
            });
        }

        let manifest = Manifest::new(parents, author, now_rfc3339(), message, entries);
        let hash = self.record_commit(&manifest)?;
        self.set_head(&hash)?;

        // Re-materialise so that created_at / updated_at reflect the commit
        // that just happened rather than the pre-commit fallback.
        let state = self.state_at(&hash)?;
        self.write_working(&state)?;
        Ok(hash)
    }

    /// Store a manifest and index it. Shared by `commit` and by sync, which
    /// receives manifests it did not build.
    pub fn record_commit(&self, manifest: &Manifest) -> Result<String> {
        let value = manifest.to_value()?;
        let hash = self.put_blob(&value)?;

        self.conn.execute(
            "INSERT OR IGNORE INTO gm_commit (hash, author, committed_at, message)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                hash,
                manifest.author,
                manifest.committed_at,
                manifest.message
            ],
        )?;
        for (ord, parent) in manifest.parents.iter().enumerate() {
            self.conn.execute(
                "INSERT OR IGNORE INTO gm_commit_parent (commit_hash, parent_hash, ord)
                 VALUES (?1, ?2, ?3)",
                params![hash, parent, ord as i64],
            )?;
        }
        for entry in &manifest.entries {
            self.conn.execute(
                "INSERT OR IGNORE INTO gm_entry (commit_hash, kind, key, blob_hash)
                 VALUES (?1, ?2, ?3, ?4)",
                params![hash, entry.kind, entry.key, entry.blob],
            )?;
        }
        Ok(hash)
    }

    /// Point HEAD at `commit_hash` and rebuild the working tree from it.
    pub fn checkout(&mut self, commit_hash: &str, force: bool) -> Result<()> {
        if !force && !self.status()?.is_empty() {
            return Err(Error::DirtyWorkingTree);
        }
        let state = self.state_at(commit_hash)?;
        self.set_head(commit_hash)?;
        self.write_working(&state)
    }

    // -- integrity ----------------------------------------------------------

    /// Re-hash every stored object and check that every reference resolves.
    /// Cheap insurance for a format meant to be archived and passed around.
    pub fn verify(&self) -> Result<Vec<String>> {
        let mut problems = Vec::new();

        let mut stmt = self.conn.prepare("SELECT hash, content FROM gm_blob")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, stored_bytes(r.get_ref(1)?)?))
        })?;
        for row in rows {
            let (hash, content) = row?;
            let actual = canon::hash_bytes(&content);
            if actual != hash {
                problems.push(format!("blob {hash} hashes to {actual}"));
            }
        }

        let mut stmt = self.conn.prepare(
            "SELECT commit_hash, blob_hash FROM gm_entry
             WHERE blob_hash NOT IN (SELECT hash FROM gm_blob)",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (commit, blob) = row?;
            problems.push(format!(
                "commit {} references missing blob {blob}",
                short_hash(&commit)
            ));
        }

        let mut stmt = self.conn.prepare(
            "SELECT commit_hash, parent_hash FROM gm_commit_parent
             WHERE parent_hash NOT IN (SELECT hash FROM gm_commit)",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (commit, parent) = row?;
            problems.push(format!(
                "commit {} references missing parent {}",
                short_hash(&commit),
                short_hash(&parent)
            ));
        }

        Ok(problems)
    }

    /// Object counts and total stored size, for `gm info`.
    pub fn object_stats(&self) -> Result<(i64, i64, i64)> {
        let (blobs, bytes): (i64, i64) = self.conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM gm_blob",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let commits: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM gm_commit", [], |r| r.get(0))?;
        Ok((blobs, commits, bytes))
    }
}

fn diff_maps<T: PartialEq + serde::Serialize>(
    old: &BTreeMap<String, T>,
    new: &BTreeMap<String, T>,
    kind: &str,
    changes: &mut Vec<Change>,
) {
    for (key, value) in new {
        match old.get(key) {
            None => changes.push(Change {
                kind: kind.into(),
                key: key.clone(),
                change: ChangeKind::Added,
            }),
            Some(prev) if prev != value => changes.push(Change {
                kind: kind.into(),
                key: key.clone(),
                change: ChangeKind::Modified,
            }),
            Some(_) => {}
        }
    }
    for key in old.keys() {
        if !new.contains_key(key) {
            changes.push(Change {
                kind: kind.into(),
                key: key.clone(),
                change: ChangeKind::Deleted,
            });
        }
    }
}

fn parse_opt_json(text: Option<String>) -> Option<Value> {
    text.and_then(|t| serde_json::from_str(&t).ok())
}

fn to_opt_json(value: &Option<Value>) -> Result<Option<String>> {
    match value {
        Some(v) => Ok(Some(serde_json::to_string(v)?)),
        None => Ok(None),
    }
}

/// Read a stored object's bytes, accepting either storage class.
///
/// `gm_blob.content` is declared `BLOB`, but SQLite types values, not columns:
/// a client that wrote the canonical JSON as TEXT produces a perfectly valid
/// row that a strict `Vec<u8>` read rejects with a type error. Since the whole
/// premise is that this file can be edited with any SQLite client, that is a
/// realistic way for a file to arrive damaged — and it must surface as "this
/// object does not match its hash", diagnosed by [`Repository::verify`], rather
/// than as an opaque column-type error that tells the reader nothing.
fn stored_bytes(value: rusqlite::types::ValueRef<'_>) -> rusqlite::Result<Vec<u8>> {
    use rusqlite::types::ValueRef;
    match value {
        ValueRef::Blob(bytes) => Ok(bytes.to_vec()),
        ValueRef::Text(bytes) => Ok(bytes.to_vec()),
        other => Err(rusqlite::Error::InvalidColumnType(
            0,
            "content".to_string(),
            other.data_type(),
        )),
    }
}
