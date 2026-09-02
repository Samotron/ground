//! The built-in web UI: `gm ui`.
//!
//! Read-only, bound to the loopback interface, and served entirely from the
//! binary. It reopens the file for every request rather than holding it, so a
//! commit made in another terminal — or an edit made with `sqlite3` — shows up
//! on the next refresh instead of going stale behind a cached handle.

mod page;
mod section;

use crate::render as text;
use anyhow::{Context, Result, anyhow};
use gm_core::exchange::Exchange;
use gm_core::model::{Bounded, ConstitutiveModel, Groundwater};
use gm_core::store::{Repository, State, short_hash};
use gm_core::validate;
use page::{escape, num, render};
use std::fmt::Write;
use std::path::Path;
use tiny_http::{Header, Response, Server};

pub fn serve(path: &Path, port: u16) -> Result<()> {
    // Loopback only. A ground model is usually commercially confidential and
    // often personally identifiable by site address; binding to 0.0.0.0 would
    // publish it to the whole network, which is never what someone typing
    // `gm ui` meant to do.
    let address = format!("127.0.0.1:{port}");
    let server =
        Server::http(&address).map_err(|e| anyhow!("could not listen on {address}: {e}"))?;

    let path = path.to_path_buf();
    let name = Repository::open(&path)?.working()?.file_metadata.name;

    println!("gm ui: {name}");
    println!("       http://{address}/");
    println!("       read-only; press Ctrl-C to stop");

    for request in server.incoming_requests() {
        let url = request.url().to_string();
        let method = request.method().as_str().to_string();

        let response = if method != "GET" {
            // The UI never writes. Editing goes through the CLI, where it is
            // validated and attributed to a named author.
            Reply::text(405, "gm ui is read-only")
        } else {
            match route(&path, &url) {
                Ok(reply) => reply,
                Err(err) => Reply::html(
                    500,
                    &render(
                        &name,
                        "Error",
                        "",
                        &format!(
                            "<h2>Something went wrong</h2><p class=\"note\">{}</p>",
                            escape(&format!("{err:#}"))
                        ),
                    ),
                ),
            }
        };

        let header = Header::from_bytes(&b"Content-Type"[..], response.content_type.as_bytes())
            .expect("content types are valid header values");
        let _ = request.respond(
            Response::from_string(response.body)
                .with_status_code(response.status)
                .with_header(header),
        );
    }
    Ok(())
}

struct Reply {
    status: u16,
    content_type: &'static str,
    body: String,
}

impl Reply {
    fn html(status: u16, body: &str) -> Self {
        Self {
            status,
            content_type: "text/html; charset=utf-8",
            body: body.to_string(),
        }
    }
    fn json(body: String) -> Self {
        Self {
            status: 200,
            content_type: "application/json; charset=utf-8",
            body,
        }
    }
    fn text(status: u16, body: &str) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8",
            body: body.to_string(),
        }
    }
}

fn route(path: &Path, url: &str) -> Result<Reply> {
    let repo = Repository::open(path).context("reopening the ground-model file")?;
    let state = repo.working()?;
    let name = state.file_metadata.name.clone();

    // Strip any query string; nothing here uses one.
    let route = url.split('?').next().unwrap_or("/");
    let segments: Vec<String> = route
        .split('/')
        .filter(|s| !s.is_empty())
        .map(percent_decode)
        .collect();

    let body = match segments.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => index(&repo, &state)?,
        ["materials"] => materials_page(&state),
        ["history"] => history_page(&repo)?,
        ["validate"] => validate_page(&state),
        ["model", key] => match state.models.get(key) {
            Some(model) => model_page(&repo, &state, model)?,
            None => return Ok(not_found(&name, "model", key)),
        },
        ["material", key] => match state.materials.get(key) {
            Some(material) => material_page(&state, material),
            None => return Ok(not_found(&name, "material", key)),
        },
        ["commit", rev] => {
            let hash = repo.resolve(rev)?;
            commit_page(&repo, &hash)?
        }
        ["api", "export"] => {
            let doc = Exchange::from_state(&state, repo.head()?);
            return Ok(Reply::json(doc.to_json_pretty()?));
        }
        ["api", "models"] => {
            return Ok(Reply::json(serde_json::to_string_pretty(
                &state.models.values().collect::<Vec<_>>(),
            )?));
        }
        ["api", "validate"] => {
            return Ok(Reply::json(serde_json::to_string_pretty(
                &validate::validate_state(&state),
            )?));
        }
        _ => return Ok(not_found(&name, "page", route)),
    };

    let (title, active) = title_for(&segments);
    Ok(Reply::html(200, &render(&name, &title, active, &body)))
}

fn title_for(segments: &[String]) -> (String, &'static str) {
    match segments.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => ("Models".into(), "models"),
        ["materials"] => ("Materials".into(), "materials"),
        ["history"] => ("History".into(), "history"),
        ["validate"] => ("Validation".into(), "validate"),
        ["model", key] => (key.to_string(), "models"),
        ["material", key] => (key.to_string(), "materials"),
        ["commit", rev] => (format!("Commit {rev}"), "history"),
        _ => ("Not found".into(), ""),
    }
}

fn not_found(file_name: &str, kind: &str, key: &str) -> Reply {
    Reply::html(
        404,
        &render(
            file_name,
            "Not found",
            "",
            &format!(
                "<h2>Not found</h2><p class=\"note\">No {} called <code>{}</code>.</p>",
                escape(kind),
                escape(key)
            ),
        ),
    )
}

/// Minimal percent-decoding for path segments. Model keys are usually plain
/// ASCII, but a chainage written `CH 100+250` would otherwise break the link.
fn percent_decode(segment: &str) -> String {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&segment[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// -- pages ------------------------------------------------------------------

fn index(repo: &Repository, state: &State) -> Result<String> {
    let mut out = String::new();
    let meta = &state.file_metadata;
    let (blobs, commits, _) = repo.object_stats()?;
    let issues = validate::validate_state(state);
    let (errors, warnings) = validate::count(&issues);

    out.push_str("<div class=\"panel\"><dl class=\"facts\">");
    let head = repo.head()?.map(|h| short_hash(&h)).unwrap_or_default();
    for (label, value) in [
        ("Name", escape(&meta.name)),
        (
            "Description",
            meta.description.as_deref().map(escape).unwrap_or_default(),
        ),
        (
            "CRS",
            meta.crs
                .as_deref()
                .map(escape)
                .unwrap_or_else(|| "<span class=\"empty\">not set</span>".into()),
        ),
        (
            "Vertical datum",
            meta.vertical_datum
                .as_deref()
                .map(escape)
                .unwrap_or_else(|| "<span class=\"empty\">not set</span>".into()),
        ),
        (
            "File id",
            format!("<span class=\"mono\">{}</span>", escape(repo.file_id())),
        ),
        (
            "Revision",
            format!("<a class=\"hash\" href=\"/commit/{head}\">{head}</a>"),
        ),
        ("History", format!("{commits} commits, {blobs} objects")),
        (
            "Validation",
            format!(
                "<span class=\"sev-error\">{errors} error{}</span>, \
                 <span class=\"sev-warning\">{warnings} warning{}</span> \
                 &nbsp;<a href=\"/validate\">details</a>",
                if errors == 1 { "" } else { "s" },
                if warnings == 1 { "" } else { "s" }
            ),
        ),
    ] {
        if value.is_empty() {
            continue;
        }
        let _ = write!(out, "<dt>{label}</dt><dd>{value}</dd>");
    }
    out.push_str("</dl></div>");

    out.push_str("<h2>Models</h2>");
    if state.models.is_empty() {
        out.push_str("<p class=\"empty\">No models yet.</p>");
    } else {
        out.push_str(
            "<table><tr><th>Key</th><th>Name</th><th class=\"num\">Surface</th>\
             <th class=\"num\">Base</th><th class=\"num\">Layers</th>\
             <th>Groundwater</th><th>Strata</th></tr>",
        );
        for model in state.models.values() {
            let strata: String = model
                .layers
                .iter()
                .map(|l| {
                    format!(
                        "<span class=\"swatch\" style=\"background:hsl({} 34% 66%)\" title=\"{}\"></span>",
                        section_hue(&l.material_key),
                        escape(&l.material_key)
                    )
                })
                .collect();
            let _ = write!(
                out,
                "<tr><td><a href=\"/model/{}\">{}</a></td><td>{}</td>\
                 <td class=\"num\">{}</td><td class=\"num\">{}</td>\
                 <td class=\"num\">{}</td><td>{}</td><td>{strata}</td></tr>",
                escape(&model.model_key),
                escape(&model.model_key),
                escape(model.name.as_deref().unwrap_or("")),
                num(model.surface_level, 2),
                num(model.base_level, 2),
                model.layers.len(),
                escape(&text::groundwater(&model.groundwater)),
            );
        }
        out.push_str("</table>");
    }
    Ok(out)
}

/// Mirror of the section drawing's colour rule, so a swatch in a table matches
/// the stratum in the drawing.
fn section_hue(key: &str) -> u32 {
    let mut hash: u32 = 2_166_136_261;
    for byte in key.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    (hash.wrapping_mul(137) % 360).max(1)
}

fn model_page(repo: &Repository, state: &State, model: &gm_core::GroundModel) -> Result<String> {
    let mut out = String::new();
    let _ = write!(
        out,
        "<h2>{} {}</h2>",
        escape(&model.model_key),
        model
            .name
            .as_deref()
            .map(|n| format!("<span class=\"note\">&mdash; {}</span>", escape(n)))
            .unwrap_or_default()
    );

    out.push_str("<div class=\"cols\"><div>");
    out.push_str(&section::draw(model, &state.materials));
    out.push_str("</div><div class=\"grow\">");

    out.push_str("<div class=\"panel\"><dl class=\"facts\">");
    let coords = match (model.x, model.y) {
        (Some(x), Some(y)) => format!("{x:.1} E, {y:.1} N"),
        _ => "<span class=\"empty\">not set</span>".into(),
    };
    for (label, value) in [
        ("Location", coords),
        ("Surface level", num(model.surface_level, 2)),
        ("Base level", num(model.base_level, 2)),
        (
            "Groundwater",
            escape(&text::groundwater(&model.groundwater)),
        ),
        ("Water unit weight", format!("{} kN/m3", model.gamma_w)),
        (
            "Description",
            model.description.as_deref().map(escape).unwrap_or_default(),
        ),
    ] {
        if value.is_empty() {
            continue;
        }
        let _ = write!(out, "<dt>{label}</dt><dd>{value}</dd>");
    }
    out.push_str("</dl></div>");

    // Any validation issue that concerns this model, shown where it matters
    // rather than only on a separate page.
    let issues: Vec<_> = validate::validate_state(state)
        .into_iter()
        .filter(|i| i.model_key.as_deref() == Some(model.model_key.as_str()))
        .collect();
    if !issues.is_empty() {
        out.push_str("<h2>Issues</h2><table>");
        for issue in &issues {
            let _ = write!(
                out,
                "<tr><td><span class=\"sev-{}\">{}</span></td><td><code>{}</code><br>{}</td></tr>",
                issue.severity.as_str(),
                issue.severity.as_str(),
                escape(&issue.field_path),
                escape(&issue.message)
            );
        }
        out.push_str("</table>");
    }
    out.push_str("</div></div>");

    out.push_str(
        "<h2>Layers</h2><table><tr><th class=\"num\">#</th><th>Material</th>\
                  <th class=\"num\">Top</th><th class=\"num\">Base</th>\
                  <th class=\"num\">Top depth</th><th class=\"num\">Thickness</th>\
                  <th>Description</th></tr>",
    );
    for (i, layer) in model.layers.iter().enumerate() {
        let base = model.layer_base(i);
        let _ = write!(
            out,
            "<tr><td class=\"num\">{}</td>\
             <td><span class=\"swatch\" style=\"background:hsl({} 34% 66%)\"></span>\
             <a href=\"/material/{}\">{}</a></td>\
             <td class=\"num\">{:.2}</td><td class=\"num\">{}</td>\
             <td class=\"num\">{}</td><td class=\"num\">{}</td><td>{}</td></tr>",
            i + 1,
            section_hue(&layer.material_key),
            escape(&layer.material_key),
            escape(&layer.material_key),
            layer.top_level,
            num(base, 2),
            num(model.surface_level.map(|s| s - layer.top_level), 2),
            num(base.map(|b| layer.top_level - b), 2),
            escape(layer.description.as_deref().unwrap_or("")),
        );
    }
    out.push_str("</table>");

    if let Groundwater::Piezometric { profile } = &model.groundwater {
        out.push_str(
            "<h2>Pore pressure profile</h2><table><tr><th class=\"num\">Depth</th>\
                      <th class=\"num\">Value</th></tr>",
        );
        for point in &profile.points {
            let _ = write!(
                out,
                "<tr><td class=\"num\">{:.2}</td><td class=\"num\">{:.1}</td></tr>",
                point.depth, point.value
            );
        }
        out.push_str("</table>");
    }

    // Which revisions touched this model.
    let mut rows = String::new();
    let mut last: Option<String> = None;
    for info in repo.log(None)?.into_iter().rev() {
        let blob: Option<String> = repo
            .connection()
            .query_row(
                "SELECT blob_hash FROM gm_entry
                 WHERE commit_hash = ?1 AND kind = 'ground_model' AND key = ?2",
                rusqlite::params![info.hash, model.model_key],
                |r| r.get(0),
            )
            .ok();
        if blob.is_some() && blob != last {
            let _ = write!(
                rows,
                "<tr><td><a class=\"hash\" href=\"/commit/{}\">{}</a></td>\
                 <td>{}</td><td>{}</td></tr>",
                short_hash(&info.hash),
                short_hash(&info.hash),
                escape(&info.committed_at),
                escape(&info.message)
            );
            last = blob;
        }
    }
    if !rows.is_empty() {
        out.push_str(
            "<h2>Revisions that changed this model</h2>\
                      <table><tr><th>Commit</th><th>When</th><th>Message</th></tr>",
        );
        out.push_str(&rows);
        out.push_str("</table>");
    }

    Ok(out)
}

fn materials_page(state: &State) -> String {
    let mut out = String::from("<h2>Materials</h2>");
    if state.materials.is_empty() {
        return out + "<p class=\"empty\">No materials yet.</p>";
    }
    out.push_str(
        "<table><tr><th>Key</th><th>Name</th><th>Class</th>\
         <th>Unit weight</th><th>Constitutive models</th><th class=\"num\">Used by</th></tr>",
    );
    for material in state.materials.values() {
        let used = state
            .models
            .values()
            .filter(|m| {
                m.layers
                    .iter()
                    .any(|l| l.material_key == material.material_key)
            })
            .count();
        let _ = write!(
            out,
            "<tr><td><span class=\"swatch\" style=\"background:hsl({} 34% 66%)\"></span>\
             <a href=\"/material/{}\">{}</a></td><td>{}</td><td>{}</td><td>{}</td>\
             <td>{}</td><td class=\"num\">{}</td></tr>",
            section_hue(&material.material_key),
            escape(&material.material_key),
            escape(&material.material_key),
            escape(material.name.as_deref().unwrap_or("")),
            escape(material.soil_class.as_deref().unwrap_or("")),
            material
                .properties
                .get("unitWeight")
                .map(|b| escape(&text::bounded(b)))
                .unwrap_or_else(|| "<span class=\"empty\">not set</span>".into()),
            material
                .constitutive_models
                .iter()
                .map(|c| format!("<span class=\"tag\">{}</span>", escape(&c.kind)))
                .collect::<Vec<_>>()
                .join(" "),
            used
        );
    }
    out.push_str("</table>");
    out
}

fn material_page(state: &State, material: &gm_core::Material) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "<h2><span class=\"swatch\" style=\"background:hsl({} 34% 66%)\"></span>{} {}</h2>",
        section_hue(&material.material_key),
        escape(&material.material_key),
        material
            .name
            .as_deref()
            .map(|n| format!("<span class=\"note\">&mdash; {}</span>", escape(n)))
            .unwrap_or_default()
    );

    if let Some(class) = &material.soil_class {
        let _ = write!(out, "<p class=\"note\">{}</p>", escape(class));
    }

    if !material.properties.is_empty() {
        out.push_str("<h2>Properties</h2>");
        out.push_str(&bounded_table(material.properties.iter()));
    }

    for cm in &material.constitutive_models {
        let drainage = cm
            .drainage
            .map(|d| {
                format!(
                    " <span class=\"tag\">{}</span>",
                    format!("{d:?}").to_lowercase()
                )
            })
            .unwrap_or_default();
        let _ = write!(
            out,
            "<h2>{} <span class=\"hash\">[{}]</span>{drainage}</h2>",
            escape(&cm.kind),
            escape(&cm.id)
        );
        out.push_str(&bounded_table(cm.parameters.iter()));
        out.push_str(&profiles(cm));
    }

    out.push_str("<h2>Used by</h2>");
    let users: Vec<String> = state
        .models
        .values()
        .filter(|m| {
            m.layers
                .iter()
                .any(|l| l.material_key == material.material_key)
        })
        .map(|m| {
            format!(
                "<a href=\"/model/{}\">{}</a>",
                escape(&m.model_key),
                escape(&m.model_key)
            )
        })
        .collect();
    if users.is_empty() {
        out.push_str("<p class=\"empty\">No model uses this material.</p>");
    } else {
        let _ = write!(out, "<p>{}</p>", users.join(", "));
    }
    out
}

fn bounded_table<'a>(params: impl Iterator<Item = (&'a String, &'a Bounded)>) -> String {
    let mut out = String::from(
        "<table><tr><th>Parameter</th><th class=\"num\">Value</th>\
         <th class=\"num\">Lower</th><th class=\"num\">Upper</th><th>Unit</th></tr>",
    );
    for (name, b) in params {
        let _ = write!(
            &mut out,
            "<tr><td>{}{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td>\
             <td class=\"num\">{}</td><td>{}</td></tr>",
            escape(name),
            if b.profile.is_some() {
                " <span class=\"tag\">varies with depth</span>"
            } else {
                ""
            },
            b.value.map(|v| v.to_string()).unwrap_or("&mdash;".into()),
            b.lower.map(|v| v.to_string()).unwrap_or("&mdash;".into()),
            b.upper.map(|v| v.to_string()).unwrap_or("&mdash;".into()),
            escape(b.unit.as_deref().unwrap_or("")),
        );
    }
    out.push_str("</table>");
    out
}

fn profiles(cm: &ConstitutiveModel) -> String {
    let mut out = String::new();
    for (name, param) in &cm.parameters {
        let Some(profile) = &param.profile else {
            continue;
        };
        let _ = write!(
            &mut out,
            "<p class=\"note\">{} varies with depth ({:?} interpolation, measured from {:?}).</p>\
             <table><tr><th class=\"num\">Depth</th><th class=\"num\">Value</th>\
             <th class=\"num\">Lower</th><th class=\"num\">Upper</th></tr>",
            escape(name),
            profile.interpolation,
            profile.datum
        );
        for point in &profile.points {
            let _ = write!(
                &mut out,
                "<tr><td class=\"num\">{}</td><td class=\"num\">{}</td>\
                 <td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
                point.depth,
                point.value,
                point
                    .lower
                    .map(|v| v.to_string())
                    .unwrap_or("&mdash;".into()),
                point
                    .upper
                    .map(|v| v.to_string())
                    .unwrap_or("&mdash;".into()),
            );
        }
        out.push_str("</table>");
    }
    out
}

fn history_page(repo: &Repository) -> Result<String> {
    let head = repo.head()?;
    let mut out = String::from(
        "<h2>History</h2><table><tr><th>Commit</th><th>When</th>\
         <th>Author</th><th>Message</th></tr>",
    );
    for info in repo.log(None)? {
        let is_head = Some(&info.hash) == head.as_ref();
        let _ = write!(
            out,
            "<tr><td><a class=\"hash\" href=\"/commit/{}\">{}</a>{}</td>\
             <td class=\"mono\">{}</td><td>{}</td><td>{}{}</td></tr>",
            short_hash(&info.hash),
            short_hash(&info.hash),
            if is_head {
                " <span class=\"tag\">current</span>"
            } else {
                ""
            },
            escape(&info.committed_at),
            escape(&info.author),
            escape(&info.message),
            if info.parents.len() > 1 {
                " <span class=\"tag\">merge</span>"
            } else {
                ""
            },
        );
    }
    out.push_str("</table>");
    Ok(out)
}

fn commit_page(repo: &Repository, hash: &str) -> Result<String> {
    let info = repo.commit_info(hash)?;
    let mut out = String::new();
    let _ = write!(
        out,
        "<h2>Commit <span class=\"hash\">{}</span></h2><div class=\"panel\"><dl class=\"facts\">\
         <dt>Message</dt><dd>{}</dd><dt>Author</dt><dd>{}</dd><dt>When</dt><dd>{}</dd>",
        short_hash(hash),
        escape(&info.message),
        escape(&info.author),
        escape(&info.committed_at)
    );
    if info.parents.is_empty() {
        out.push_str("<dt>Parent</dt><dd class=\"empty\">none (root commit)</dd>");
    } else {
        let links: Vec<String> = info
            .parents
            .iter()
            .map(|p| {
                format!(
                    "<a class=\"hash\" href=\"/commit/{0}\">{0}</a>",
                    short_hash(p)
                )
            })
            .collect();
        let _ = write!(
            out,
            "<dt>Parent{}</dt><dd>{}</dd>",
            if links.len() > 1 { "s" } else { "" },
            links.join(", ")
        );
    }
    out.push_str("</dl></div>");

    // What this revision changed, against its first parent.
    let new_state = repo.state_at(hash)?;
    if let Some(parent) = info.parents.first() {
        let old_state = repo.state_at(parent)?;
        let diff = text::diff_states(&old_state, &new_state);
        let _ = write!(
            out,
            "<h2>Changes</h2><pre class=\"panel mono\">{}</pre>",
            escape(diff.trim())
        );
    }

    out.push_str(
        "<h2>Contents at this revision</h2><table><tr><th>Model</th>\
                  <th class=\"num\">Layers</th><th class=\"num\">Surface</th>\
                  <th class=\"num\">Base</th></tr>",
    );
    for model in new_state.models.values() {
        let _ = write!(
            out,
            "<tr><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td>\
             <td class=\"num\">{}</td></tr>",
            escape(&model.model_key),
            model.layers.len(),
            num(model.surface_level, 2),
            num(model.base_level, 2),
        );
    }
    out.push_str("</table>");
    Ok(out)
}

fn validate_page(state: &State) -> String {
    let issues = validate::validate_state(state);
    let (errors, warnings) = validate::count(&issues);

    let mut out = format!(
        "<h2>Validation</h2><p><span class=\"sev-error\">{errors} error{}</span> and \
         <span class=\"sev-warning\">{warnings} warning{}</span>.</p>",
        if errors == 1 { "" } else { "s" },
        if warnings == 1 { "" } else { "s" }
    );
    if issues.is_empty() {
        return out + "<p class=\"note\">Nothing to report. Every model is coherent.</p>";
    }

    let mut sorted = issues;
    sorted.sort_by(|a, b| {
        (a.severity, &a.model_key, &a.field_path).cmp(&(b.severity, &b.model_key, &b.field_path))
    });

    out.push_str("<table><tr><th>Severity</th><th>Model</th><th>Field</th><th>Message</th></tr>");
    for issue in &sorted {
        let model = match &issue.model_key {
            Some(key) => format!("<a href=\"/model/{0}\">{0}</a>", escape(key)),
            None => "<span class=\"empty\">file</span>".into(),
        };
        let _ = write!(
            out,
            "<tr><td><span class=\"sev-{0}\">{0}</span></td><td>{model}</td>\
             <td><code>{1}</code></td><td>{2}</td></tr>",
            issue.severity.as_str(),
            escape(&issue.field_path),
            escape(&issue.message)
        );
    }
    out.push_str("</table>");

    if errors > 0 {
        out.push_str(
            "<p class=\"note\">Errors block <code>gm commit</code>: a history of invalid \
             states would remove the one thing a consumer could rely on.</p>",
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encoded_path_segments_decode() {
        assert_eq!(percent_decode("CH-100"), "CH-100");
        assert_eq!(percent_decode("CH%20100%2B250"), "CH 100+250");
        assert_eq!(percent_decode("a+b"), "a b");
    }

    #[test]
    fn a_truncated_escape_is_left_alone_rather_than_panicking() {
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("100%2"), "100%2");
        assert_eq!(percent_decode("%zz"), "%zz");
    }
}
