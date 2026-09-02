//! `gm` — a self-contained tool for 1D ground models.

mod render;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use gm_core::exchange::{Exchange, merge_into};
use gm_core::store::{Repository, State, now_rfc3339, short_hash};
use gm_core::validate;
use gm_core::{FileMetadata, schema};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "gm",
    version,
    about = "Transfer, storage and creation of 1D ground models.",
    long_about = "A ground-model file is one SQLite database holding the full revision \
history of every model it contains, plus a plain SQL view of the current \
revision that any tool can read."
)]
struct Cli {
    /// The ground-model file to work on. Defaults to $GM_FILE, or the single
    /// *.gm file in the current directory.
    #[arg(short, long, global = true, env = "GM_FILE")]
    file: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new ground-model file.
    Init {
        path: PathBuf,
        /// Human-readable name for the file.
        #[arg(short, long)]
        name: Option<String>,
        /// Horizontal CRS, e.g. EPSG:27700.
        #[arg(long)]
        crs: Option<String>,
        /// Vertical datum, e.g. "Ordnance Datum Newlyn".
        #[arg(long)]
        datum: Option<String>,
        #[arg(long, env = "GM_AUTHOR")]
        author: Option<String>,
    },

    /// Show what the file is and how big its history is.
    Info,

    /// Show uncommitted changes in the working tree.
    Status,

    /// Show the revision history.
    Log {
        #[arg(short = 'n', long)]
        limit: Option<usize>,
    },

    /// List the models in the file.
    Models,

    /// List the materials in the file.
    Materials,

    /// Draw a section through one model, or show one material.
    Show {
        /// A model key, or a material key.
        key: String,
        /// Read from this revision instead of the working tree.
        #[arg(long)]
        rev: Option<String>,
    },

    /// Print the raw versioned document for a model or material.
    Cat {
        key: String,
        #[arg(long)]
        rev: Option<String>,
    },

    /// Record the working tree as a new revision.
    Commit {
        #[arg(short, long)]
        message: String,
        #[arg(long, env = "GM_AUTHOR")]
        author: Option<String>,
    },

    /// Replace the working tree with an earlier revision.
    Checkout {
        rev: String,
        /// Discard uncommitted changes.
        #[arg(long)]
        force: bool,
    },

    /// Compare two revisions, or the working tree against one.
    Diff {
        /// Defaults to HEAD.
        from: Option<String>,
        /// Defaults to the working tree.
        to: Option<String>,
    },

    /// Check the models for errors and suspicious values.
    Validate {
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
        /// Also write results into the model_issues table.
        #[arg(long)]
        store: bool,
    },

    /// Re-hash every object and check every reference resolves.
    Verify,

    /// Read a flat JSON interchange document into the working tree.
    Import {
        path: PathBuf,
        /// Discard the current working tree instead of merging into it.
        #[arg(long)]
        replace: bool,
    },

    /// Write the current revision as a flat JSON interchange document.
    Export {
        /// Output path; defaults to stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        rev: Option<String>,
    },

    /// Run a read-only SQL query against the materialised tables.
    Sql { query: String },
}

fn main() {
    if let Err(err) = run() {
        eprintln!("gm: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    // `init` is the one command that must not resolve an existing file.
    if let Command::Init {
        path,
        name,
        crs,
        datum,
        author,
    } = &cli.command
    {
        return init(
            path,
            name.as_deref(),
            crs.as_deref(),
            datum.as_deref(),
            author.as_deref(),
        );
    }

    let path = resolve_file(cli.file.as_deref())?;
    let mut repo =
        Repository::open(&path).with_context(|| format!("opening {}", path.display()))?;

    match cli.command {
        Command::Init { .. } => unreachable!("handled above"),

        Command::Info => info(&repo, &path),
        Command::Status => {
            print!("{}", render::changes(&repo.status()?));
            Ok(())
        }
        Command::Log { limit } => {
            for entry in repo.log(limit)? {
                println!("{}", render::commit_line(&entry));
            }
            Ok(())
        }
        Command::Models => {
            print!("{}", render::model_list(&repo.working()?));
            Ok(())
        }
        Command::Materials => {
            print!("{}", render::material_list(&repo.working()?));
            Ok(())
        }
        Command::Show { key, rev } => show(&repo, &key, rev.as_deref()),
        Command::Cat { key, rev } => cat(&repo, &key, rev.as_deref()),
        Command::Commit { message, author } => commit(&mut repo, &message, author.as_deref()),
        Command::Checkout { rev, force } => {
            let hash = repo.resolve(&rev)?;
            repo.checkout(&hash, force)?;
            println!("checked out {}", short_hash(&hash));
            Ok(())
        }
        Command::Diff { from, to } => diff(&repo, from.as_deref(), to.as_deref()),
        Command::Validate { json, store } => run_validate(&repo, json, store),
        Command::Verify => verify(&repo),
        Command::Import { path, replace } => import(&mut repo, &path, replace),
        Command::Export { output, rev } => export(&repo, output.as_deref(), rev.as_deref()),
        Command::Sql { query } => sql(&repo, &query),
    }
}

/// Find the file to operate on. Explicit flag wins; otherwise a single `*.gm`
/// in the working directory is unambiguous enough to use without being asked.
fn resolve_file(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(".")
        .context("reading the current directory")?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "gm"))
        .collect();
    candidates.sort();

    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        0 => bail!("no ground-model file found; pass --file or run `gm init`"),
        n => bail!(
            "{n} ground-model files here; pass --file to say which ({})",
            candidates
                .iter()
                .filter_map(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Author for a commit: the flag, then $GM_AUTHOR, then the file's recorded
/// default. Commits are attribution, so we refuse to invent one.
fn resolve_author(repo: &Repository, explicit: Option<&str>) -> Result<String> {
    if let Some(author) = explicit {
        return Ok(author.to_string());
    }
    repo.default_author()?
        .ok_or_else(|| anyhow!("no author; pass --author or set GM_AUTHOR"))
}

fn init(
    path: &Path,
    name: Option<&str>,
    crs: Option<&str>,
    datum: Option<&str>,
    author: Option<&str>,
) -> Result<()> {
    let author = author
        .map(str::to_string)
        .or_else(|| std::env::var("GM_AUTHOR").ok())
        .ok_or_else(|| anyhow!("no author; pass --author or set GM_AUTHOR"))?;

    let name = name
        .map(str::to_string)
        .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "Ground model file".to_string());

    let mut meta = FileMetadata::new(name);
    meta.crs = crs.map(str::to_string);
    meta.vertical_datum = datum.map(str::to_string);

    let repo = Repository::create(path, meta, &author)?;
    println!(
        "initialised {} (file id {})",
        path.display(),
        repo.file_id()
    );
    if crs.is_none() || datum.is_none() {
        println!(
            "note: set --crs and --datum, or every level and coordinate in this file is ambiguous"
        );
    }
    Ok(())
}

fn info(repo: &Repository, path: &Path) -> Result<()> {
    let state = repo.working()?;
    let (blobs, commits, bytes) = repo.object_stats()?;
    let head = repo.head()?;

    println!("file       {}", path.display());
    println!("name       {}", state.file_metadata.name);
    println!("file id    {}", repo.file_id());
    println!("schema     {}", schema::SCHEMA_VERSION);
    println!(
        "crs        {}",
        state.file_metadata.crs.as_deref().unwrap_or("(none)")
    );
    println!(
        "datum      {}",
        state
            .file_metadata
            .vertical_datum
            .as_deref()
            .unwrap_or("(none)")
    );
    println!("models     {}", state.models.len());
    println!("materials  {}", state.materials.len());
    println!(
        "head       {}",
        head.as_deref()
            .map(short_hash)
            .unwrap_or_else(|| "(none)".into())
    );
    println!("history    {commits} commits, {blobs} objects, {bytes} bytes of documents");

    let changes = repo.status()?;
    if !changes.is_empty() {
        println!("status     {} uncommitted change(s)", changes.len());
    }
    Ok(())
}

fn state_for(repo: &Repository, rev: Option<&str>) -> Result<State> {
    match rev {
        Some(rev) => {
            let hash = repo.resolve(rev)?;
            Ok(repo.state_at(&hash)?)
        }
        None => Ok(repo.working()?),
    }
}

fn show(repo: &Repository, key: &str, rev: Option<&str>) -> Result<()> {
    let state = state_for(repo, rev)?;
    if let Some(model) = state.models.get(key) {
        print!("{}", render::section(model, &state.materials));
        return Ok(());
    }
    if let Some(material) = state.materials.get(key) {
        print!("{}", render::material_detail(material));
        return Ok(());
    }
    bail!("no model or material called '{key}'")
}

fn cat(repo: &Repository, key: &str, rev: Option<&str>) -> Result<()> {
    let state = state_for(repo, rev)?;
    if let Some(model) = state.models.get(key) {
        println!("{}", serde_json::to_string_pretty(model)?);
        return Ok(());
    }
    if let Some(material) = state.materials.get(key) {
        println!("{}", serde_json::to_string_pretty(material)?);
        return Ok(());
    }
    bail!("no model or material called '{key}'")
}

fn commit(repo: &mut Repository, message: &str, author: Option<&str>) -> Result<()> {
    let author = resolve_author(repo, author)?;

    // Refuse to record a revision that is known to be broken. A history full of
    // invalid states is worse than no history, because it removes the one thing
    // a consumer could rely on.
    let issues = validate::validate_state(&repo.working()?);
    let (errors, _) = validate::count(&issues);
    if errors > 0 {
        print!("{}", render::issues(&issues));
        bail!("{errors} validation error(s); fix them or run `gm validate` for detail");
    }

    match repo.commit(&author, message)? {
        Some(hash) => {
            println!("committed {}", short_hash(&hash));
            Ok(())
        }
        None => {
            println!("nothing to commit");
            Ok(())
        }
    }
}

fn diff(repo: &Repository, from: Option<&str>, to: Option<&str>) -> Result<()> {
    let from_rev = from.unwrap_or("HEAD");
    let old = state_for(repo, Some(from_rev))?;
    let new = state_for(repo, to)?;
    print!("{}", render::diff_states(&old, &new));
    Ok(())
}

fn run_validate(repo: &Repository, json: bool, store: bool) -> Result<()> {
    let state = repo.working()?;
    let issues = validate::validate_state(&state);
    let (errors, warnings) = validate::count(&issues);

    if store {
        let conn = repo.connection();
        conn.execute("DELETE FROM model_issues", [])?;
        let now = now_rfc3339();
        let mut stmt = conn.prepare(
            "INSERT INTO model_issues
                (id, ground_model_id, model_key, severity, field_path, message, created_at)
             VALUES (?1,
                     (SELECT id FROM ground_models WHERE model_key = ?2),
                     ?2, ?3, ?4, ?5, ?6)",
        )?;
        for (i, issue) in issues.iter().enumerate() {
            stmt.execute(rusqlite::params![
                format!("issue-{i:06}"),
                issue.model_key,
                issue.severity.as_str(),
                issue.field_path,
                issue.message,
                now,
            ])?;
        }
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&issues)?);
    } else {
        print!("{}", render::issues(&issues));
        println!("\n{errors} error(s), {warnings} warning(s)");
    }

    if errors > 0 {
        std::process::exit(2);
    }
    Ok(())
}

fn verify(repo: &Repository) -> Result<()> {
    let problems = repo.verify()?;
    if problems.is_empty() {
        let (blobs, commits, _) = repo.object_stats()?;
        println!("ok: {blobs} objects and {commits} commits verified");
        return Ok(());
    }
    for problem in &problems {
        println!("{problem}");
    }
    bail!("{} integrity problem(s)", problems.len())
}

fn import(repo: &mut Repository, path: &Path, replace: bool) -> Result<()> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let incoming = Exchange::from_json(&text)?.into_state();

    let state = if replace {
        incoming
    } else {
        let mut base = repo.working()?;
        let report = merge_into(&mut base, incoming, false);
        println!(
            "models: {} added, {} replaced, {} unchanged",
            report.models.added, report.models.replaced, report.models.unchanged
        );
        println!(
            "materials: {} added, {} replaced, {} unchanged",
            report.materials.added, report.materials.replaced, report.materials.unchanged
        );
        base
    };

    repo.write_working(&state)?;
    println!("imported into the working tree; run `gm commit` to record it");
    Ok(())
}

fn export(repo: &Repository, output: Option<&Path>, rev: Option<&str>) -> Result<()> {
    let source_commit = match rev {
        Some(rev) => Some(repo.resolve(rev)?),
        None => repo.head()?,
    };
    let state = state_for(repo, rev)?;
    let doc = Exchange::from_state(&state, source_commit);
    let json = doc.to_json_pretty()?;

    match output {
        Some(path) => {
            std::fs::write(path, format!("{json}\n"))
                .with_context(|| format!("writing {}", path.display()))?;
            println!("wrote {}", path.display());
        }
        None => println!("{json}"),
    }
    Ok(())
}

fn sql(repo: &Repository, query: &str) -> Result<()> {
    let conn = repo.connection();
    let mut stmt = conn.prepare(query).context("preparing the query")?;
    let columns: Vec<String> = stmt
        .column_names()
        .into_iter()
        .map(str::to_string)
        .collect();
    println!("{}", columns.join("\t"));

    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let cells: Vec<String> = (0..columns.len())
            .map(|i| match row.get_ref(i) {
                Ok(rusqlite::types::ValueRef::Null) => String::new(),
                Ok(rusqlite::types::ValueRef::Integer(v)) => v.to_string(),
                Ok(rusqlite::types::ValueRef::Real(v)) => v.to_string(),
                Ok(rusqlite::types::ValueRef::Text(v)) => String::from_utf8_lossy(v).into_owned(),
                Ok(rusqlite::types::ValueRef::Blob(v)) => format!("<{} bytes>", v.len()),
                Err(e) => format!("<{e}>"),
            })
            .collect();
        println!("{}", cells.join("\t"));
    }
    Ok(())
}
