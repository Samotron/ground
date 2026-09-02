//! `gm` — a self-contained tool for 1D ground models.

mod render;
mod web;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use gm_core::exchange::{Exchange, merge_into};
use gm_core::store::{Repository, State, now_rfc3339, short_hash};
use gm_core::sync;
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

    /// Copy a ground-model file, history and all.
    Clone {
        /// Path of the file to copy.
        source: PathBuf,
        /// Path to create. Defaults to the source's file name here.
        dest: Option<PathBuf>,
        #[arg(long, env = "GM_AUTHOR")]
        author: Option<String>,
    },

    /// Fetch from another copy and fast-forward if possible.
    Pull {
        /// A remote name, or a path to another ground-model file.
        remote: Option<String>,
    },

    /// Send local commits to another copy.
    Push {
        /// A remote name, or a path to another ground-model file.
        remote: Option<String>,
    },

    /// Merge a diverged revision into the current one.
    Merge {
        rev: String,
        #[arg(long, env = "GM_AUTHOR")]
        author: Option<String>,
    },

    /// Serve a local read-only web UI for this file.
    Ui {
        #[arg(short, long, default_value_t = 8765)]
        port: u16,
    },

    /// Manage the named copies this file syncs with.
    Remote {
        #[command(subcommand)]
        action: RemoteAction,
    },
}

#[derive(Subcommand)]
enum RemoteAction {
    /// List the configured remotes.
    List,
    /// Add or update one.
    Add { name: String, url: String },
    /// Forget one.
    Remove { name: String },
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

    // Like `init`, `clone` creates its target rather than opening one.
    if let Command::Clone {
        source,
        dest,
        author,
    } = &cli.command
    {
        return clone(source, dest.as_deref(), author.as_deref());
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
        Command::Clone { .. } => unreachable!("handled above"),
        Command::Pull { remote } => pull(&mut repo, remote.as_deref()),
        Command::Push { remote } => push(&repo, remote.as_deref()),
        Command::Merge { rev, author } => merge(&mut repo, &rev, author.as_deref()),
        Command::Ui { port } => {
            // Drop our handle first: the server reopens the file per request.
            drop(repo);
            web::serve(&path, port)
        }
        Command::Remote { action } => remote(&repo, action),
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

// -- sync -------------------------------------------------------------------

/// Resolve a remote argument to a path. A bare name is looked up in the file's
/// remote list; anything else is taken as a path.
fn remote_path(repo: &Repository, arg: Option<&str>) -> Result<PathBuf> {
    let name = arg.unwrap_or("origin");
    if let Some(url) = repo.remote_url(name)? {
        return Ok(PathBuf::from(url));
    }
    match arg {
        Some(arg) => {
            let path = PathBuf::from(arg);
            if path.exists() {
                Ok(path)
            } else {
                bail!("no remote called '{arg}', and no file at that path")
            }
        }
        None => bail!("no remote given and none called 'origin' is configured"),
    }
}

fn clone(source: &Path, dest: Option<&Path>, author: Option<&str>) -> Result<()> {
    let author = author
        .map(str::to_string)
        .or_else(|| std::env::var("GM_AUTHOR").ok())
        .ok_or_else(|| anyhow!("no author; pass --author or set GM_AUTHOR"))?;

    let dest = match dest {
        Some(dest) => dest.to_path_buf(),
        None => PathBuf::from(
            source
                .file_name()
                .ok_or_else(|| anyhow!("{} has no file name", source.display()))?,
        ),
    };

    let src = Repository::open(source).with_context(|| format!("opening {}", source.display()))?;
    let cloned = sync::clone_to(&src, &dest, &author)?;

    // Point the clone back at where it came from, so a bare `gm pull` works.
    let origin = std::fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
    cloned.set_remote("origin", &origin.to_string_lossy())?;

    let (blobs, commits, _) = cloned.object_stats()?;
    println!(
        "cloned {} into {} ({commits} commits, {blobs} objects)",
        source.display(),
        dest.display()
    );
    Ok(())
}

fn pull(repo: &mut Repository, remote_arg: Option<&str>) -> Result<()> {
    let path = remote_path(repo, remote_arg)?;
    let remote = Repository::open(&path).with_context(|| format!("opening {}", path.display()))?;

    match sync::pull(repo, &remote)? {
        sync::PullOutcome::UpToDate => println!("already up to date"),
        sync::PullOutcome::FastForward { to, transferred } => {
            println!(
                "fast-forwarded to {} ({} commits, {} objects)",
                short_hash(&to),
                transferred.commits,
                transferred.objects
            );
        }
        sync::PullOutcome::Diverged {
            ours,
            theirs,
            base,
            transferred,
        } => {
            println!(
                "fetched {} commits, {} objects",
                transferred.commits, transferred.objects
            );
            println!("histories have diverged:");
            println!("  yours   {}", short_hash(&ours));
            println!("  theirs  {}", short_hash(&theirs));
            match base {
                Some(base) => println!("  common  {}", short_hash(&base)),
                None => println!("  common  (none: unrelated histories)"),
            }
            println!("\nrun `gm merge {}` to combine them", short_hash(&theirs));
        }
    }
    Ok(())
}

fn push(repo: &Repository, remote_arg: Option<&str>) -> Result<()> {
    let path = remote_path(repo, remote_arg)?;
    let mut remote =
        Repository::open(&path).with_context(|| format!("opening {}", path.display()))?;

    match sync::push(repo, &mut remote)? {
        sync::PushOutcome::UpToDate => println!("already up to date"),
        sync::PushOutcome::FastForward { to, transferred } => {
            println!(
                "pushed to {}: now at {} ({} commits, {} objects)",
                path.display(),
                short_hash(&to),
                transferred.commits,
                transferred.objects
            );
        }
    }
    Ok(())
}

fn merge(repo: &mut Repository, rev: &str, author: Option<&str>) -> Result<()> {
    let author = resolve_author(repo, author)?;
    let theirs = repo.resolve(rev)?;
    let ours = repo
        .head()?
        .ok_or_else(|| anyhow!("this file has no commits to merge into"))?;

    match sync::merge(repo, &ours, &theirs, &author)? {
        sync::MergeOutcome::AlreadyUpToDate => {
            println!(
                "{} is already in this history; nothing to merge",
                short_hash(&theirs)
            );
            Ok(())
        }
        sync::MergeOutcome::FastForward { to } => {
            println!("fast-forwarded to {}", short_hash(&to));
            Ok(())
        }
        sync::MergeOutcome::Merged { hash, result } => {
            println!("merged {} into {}", short_hash(&theirs), short_hash(&ours));
            for key in &result.took_theirs {
                println!("  took theirs  {key}");
            }
            for key in &result.kept_ours {
                println!("  kept yours   {key}");
            }
            println!("committed {}", short_hash(&hash));
            Ok(())
        }
        sync::MergeOutcome::Conflicts(result) => {
            println!("conflicts in {} document(s):", result.conflicts.len());
            for conflict in &result.conflicts {
                println!("  {:<14} {}", conflict.kind, conflict.key);
            }
            println!(
                "\nBoth sides changed these. Nothing has been written; pick a version \
                 with `gm checkout`, or edit and commit the one you want to keep."
            );
            bail!("merge stopped on conflicts")
        }
    }
}

fn remote(repo: &Repository, action: RemoteAction) -> Result<()> {
    match action {
        RemoteAction::List => {
            let remotes = repo.remotes()?;
            if remotes.is_empty() {
                println!("no remotes");
            }
            for (name, url) in remotes {
                println!("{name:<12} {url}");
            }
        }
        RemoteAction::Add { name, url } => {
            repo.set_remote(&name, &url)?;
            println!("remote '{name}' -> {url}");
        }
        RemoteAction::Remove { name } => {
            if repo.remove_remote(&name)? {
                println!("removed remote '{name}'");
            } else {
                bail!("no remote called '{name}'");
            }
        }
    }
    Ok(())
}
