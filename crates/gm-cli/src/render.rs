//! Human-readable output.
//!
//! A ground model is a picture in an engineer's head before it is a row in a
//! table, so `gm show` draws the succession rather than dumping the document.

use gm_core::model::{Bounded, GroundModel, Groundwater, Material};
use gm_core::store::{Change, State, short_hash};
use gm_core::validate::{Issue, Severity};
use std::collections::BTreeMap;
use std::fmt::Write;

pub fn level(v: Option<f64>) -> String {
    v.map(|v| format!("{v:.2}")).unwrap_or_else(|| "--".into())
}

/// A one-line section through a model: levels, thicknesses and materials.
pub fn section(model: &GroundModel, materials: &BTreeMap<String, Material>) -> String {
    let mut out = String::new();
    let name = model.name.as_deref().unwrap_or("");
    let _ = writeln!(out, "{}  {}", model.model_key, name);

    if let (Some(x), Some(y)) = (model.x, model.y) {
        let _ = writeln!(out, "  location   {x:.1} E, {y:.1} N");
    }
    let _ = writeln!(
        out,
        "  surface    {}    base {}",
        level(model.surface_level),
        level(model.base_level)
    );
    let _ = writeln!(out, "  water      {}", groundwater(&model.groundwater));
    if let Some(desc) = &model.description {
        let _ = writeln!(out, "  note       {desc}");
    }
    out.push('\n');

    if model.layers.is_empty() {
        out.push_str("  (no layers)\n");
        return out;
    }

    let _ = writeln!(out, "     Level    Depth  Thickness   Material",);
    let _ = writeln!(out, "  {}", "-".repeat(66));

    for (i, layer) in model.layers.iter().enumerate() {
        let base = model.layer_base(i);
        let depth = model.surface_level.map(|s| s - layer.top_level);
        let thickness = base.map(|b| layer.top_level - b);
        let mat_name = materials
            .get(&layer.material_key)
            .and_then(|m| m.name.clone())
            .unwrap_or_default();

        let _ = writeln!(
            out,
            "  {:>8}  {:>7}  {:>9}   {:<14} {}",
            format!("{:.2}", layer.top_level),
            depth
                .map(|d| format!("{d:.2}"))
                .unwrap_or_else(|| "--".into()),
            thickness
                .map(|t| format!("{t:.2}"))
                .unwrap_or_else(|| "--".into()),
            layer.material_key,
            mat_name,
        );
    }

    // Close the section off at the base so the last layer visibly has a bottom.
    if let Some(base) = model.base_level {
        let depth = model.surface_level.map(|s| s - base);
        let _ = writeln!(
            out,
            "  {:>8}  {:>7}              (base of model)",
            format!("{base:.2}"),
            depth
                .map(|d| format!("{d:.2}"))
                .unwrap_or_else(|| "--".into()),
        );
    }
    out
}

pub fn groundwater(gw: &Groundwater) -> String {
    match gw {
        Groundwater::Dry => "dry".into(),
        Groundwater::Unknown => "unknown".into(),
        Groundwater::Hydrostatic { depth } => {
            format!("hydrostatic, water table {depth:.2} m below ground level")
        }
        Groundwater::Piezometric { profile } => {
            format!("piezometric profile, {} points", profile.points.len())
        }
    }
}

/// Compact rendering of a bounded quantity: `19 (17-21) kN/m3`.
pub fn bounded(b: &Bounded) -> String {
    let mut s = match b.value {
        Some(v) => format!("{v}"),
        None => "-".to_string(),
    };
    if let (Some(lo), Some(hi)) = (b.lower, b.upper) {
        let _ = write!(s, " ({lo}-{hi})");
    }
    if let Some(unit) = &b.unit {
        let _ = write!(s, " {unit}");
    }
    if let Some(p) = &b.profile {
        let _ = write!(s, " [profile, {} points]", p.points.len());
    }
    s
}

pub fn model_list(state: &State) -> String {
    if state.models.is_empty() {
        return "no models\n".into();
    }
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<16} {:>9} {:>9} {:>7}  NAME",
        "KEY", "SURFACE", "BASE", "LAYERS"
    );
    for model in state.models.values() {
        let _ = writeln!(
            out,
            "{:<16} {:>9} {:>9} {:>7}  {}",
            model.model_key,
            level(model.surface_level),
            level(model.base_level),
            model.layers.len(),
            model.name.as_deref().unwrap_or("")
        );
    }
    out
}

pub fn material_list(state: &State) -> String {
    if state.materials.is_empty() {
        return "no materials\n".into();
    }
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<16} {:<14} {:<20} MODELS",
        "KEY", "CLASS", "UNIT WEIGHT"
    );
    for material in state.materials.values() {
        let uw = material
            .properties
            .get("unitWeight")
            .map(bounded)
            .unwrap_or_else(|| "-".into());
        let kinds: Vec<&str> = material
            .constitutive_models
            .iter()
            .map(|c| c.kind.as_str())
            .collect();
        let _ = writeln!(
            out,
            "{:<16} {:<14} {:<20} {}",
            material.material_key,
            material.soil_class.as_deref().unwrap_or("-"),
            uw,
            kinds.join(", ")
        );
    }
    out
}

pub fn material_detail(material: &Material) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{}  {}",
        material.material_key,
        material.name.as_deref().unwrap_or("")
    );
    if let Some(class) = &material.soil_class {
        let _ = writeln!(out, "  class      {class}");
    }
    if let Some(desc) = &material.description {
        let _ = writeln!(out, "  note       {desc}");
    }

    if !material.properties.is_empty() {
        let _ = writeln!(out, "\n  Properties");
        for (name, value) in &material.properties {
            let _ = writeln!(out, "    {:<24} {}", name, bounded(value));
        }
    }

    for cm in &material.constitutive_models {
        let drainage = cm
            .drainage
            .map(|d| format!(" ({d:?})").to_lowercase())
            .unwrap_or_default();
        let _ = writeln!(out, "\n  {} [{}]{}", cm.kind, cm.id, drainage);
        for (name, value) in &cm.parameters {
            let _ = writeln!(out, "    {:<24} {}", name, bounded(value));
        }
        if let Some(profiles) = profile_detail(cm) {
            out.push_str(&profiles);
        }
    }
    out
}

/// Expand any depth-varying parameters, which the one-line form only counts.
fn profile_detail(cm: &gm_core::model::ConstitutiveModel) -> Option<String> {
    let mut out = String::new();
    for (name, param) in &cm.parameters {
        let Some(profile) = &param.profile else {
            continue;
        };
        let _ = writeln!(
            out,
            "    {name} profile ({:?}, from {:?}):",
            profile.interpolation, profile.datum
        );
        for point in &profile.points {
            let bounds = match (point.lower, point.upper) {
                (Some(lo), Some(hi)) => format!(" ({lo}-{hi})"),
                _ => String::new(),
            };
            let _ = writeln!(out, "      {:>8}  {}{}", point.depth, point.value, bounds);
        }
    }
    (!out.is_empty()).then_some(out)
}

pub fn issues(list: &[Issue]) -> String {
    if list.is_empty() {
        return "no issues\n".into();
    }
    let mut out = String::new();
    // Errors first: they are the ones that block a consumer.
    let mut sorted: Vec<&Issue> = list.iter().collect();
    sorted.sort_by_key(|i| (i.severity, i.model_key.clone(), i.field_path.clone()));

    for issue in sorted {
        let scope = issue
            .model_key
            .as_deref()
            .map(|m| format!("{m}: "))
            .unwrap_or_default();
        let marker = match issue.severity {
            Severity::Error => "error  ",
            Severity::Warning => "warning",
        };
        let _ = writeln!(
            out,
            "{marker}  {scope}{}\n           {}",
            issue.field_path, issue.message
        );
    }
    out
}

pub fn changes(list: &[Change]) -> String {
    if list.is_empty() {
        return "working tree matches the last commit\n".into();
    }
    let mut out = String::new();
    for change in list {
        let _ = writeln!(
            out,
            "  {}  {:<14} {}",
            change.change.marker(),
            change.kind,
            change.key
        );
    }
    out
}

/// A field-level diff between two revisions. Layer boundaries get individual
/// attention because "the clay came up 400 mm" is the change an engineer
/// actually needs to see.
pub fn diff_states(old: &State, new: &State) -> String {
    let mut out = String::new();

    if old.file_metadata != new.file_metadata {
        let _ = writeln!(out, "file metadata changed");
    }

    for (key, model) in &new.models {
        match old.models.get(key) {
            None => {
                let _ = writeln!(out, "\n+ model {key} ({} layers)", model.layers.len());
            }
            Some(prev) if prev != model => {
                let _ = writeln!(out, "\n~ model {key}");
                diff_model(prev, model, &mut out);
            }
            Some(_) => {}
        }
    }
    for key in old.models.keys() {
        if !new.models.contains_key(key) {
            let _ = writeln!(out, "\n- model {key}");
        }
    }

    for (key, material) in &new.materials {
        match old.materials.get(key) {
            None => {
                let _ = writeln!(out, "\n+ material {key}");
            }
            Some(prev) if prev != material => {
                let _ = writeln!(out, "\n~ material {key}");
                diff_material(prev, material, &mut out);
            }
            Some(_) => {}
        }
    }
    for key in old.materials.keys() {
        if !new.materials.contains_key(key) {
            let _ = writeln!(out, "\n- material {key}");
        }
    }

    if out.is_empty() {
        out.push_str("no differences\n");
    }
    out
}

fn diff_model(old: &GroundModel, new: &GroundModel, out: &mut String) {
    scalar(out, "name", &old.name, &new.name);
    scalar(out, "description", &old.description, &new.description);
    scalar(out, "surfaceLevel", &old.surface_level, &new.surface_level);
    scalar(out, "baseLevel", &old.base_level, &new.base_level);
    scalar(out, "x", &old.x, &new.x);
    scalar(out, "y", &old.y, &new.y);
    if old.gamma_w != new.gamma_w {
        let _ = writeln!(out, "    gammaW  {} -> {}", old.gamma_w, new.gamma_w);
    }
    if old.groundwater != new.groundwater {
        let _ = writeln!(
            out,
            "    groundwater  {} -> {}",
            groundwater(&old.groundwater),
            groundwater(&new.groundwater)
        );
    }

    let n = old.layers.len().max(new.layers.len());
    for i in 0..n {
        match (old.layers.get(i), new.layers.get(i)) {
            (Some(a), Some(b)) if a == b => {}
            (Some(a), Some(b)) => {
                if a.material_key != b.material_key {
                    let _ = writeln!(
                        out,
                        "    layer {}  material  {} -> {}",
                        i + 1,
                        a.material_key,
                        b.material_key
                    );
                }
                if a.top_level != b.top_level {
                    let shift = b.top_level - a.top_level;
                    let _ = writeln!(
                        out,
                        "    layer {}  top  {:.2} -> {:.2}  ({}{:.2} m)",
                        i + 1,
                        a.top_level,
                        b.top_level,
                        if shift >= 0.0 { "+" } else { "" },
                        shift
                    );
                }
                if a.description != b.description || a.source != b.source {
                    let _ = writeln!(out, "    layer {}  annotation changed", i + 1);
                }
            }
            (None, Some(b)) => {
                let _ = writeln!(
                    out,
                    "  + layer {}  {} at {:.2}",
                    i + 1,
                    b.material_key,
                    b.top_level
                );
            }
            (Some(a), None) => {
                let _ = writeln!(
                    out,
                    "  - layer {}  {} at {:.2}",
                    i + 1,
                    a.material_key,
                    a.top_level
                );
            }
            (None, None) => {}
        }
    }
}

fn diff_material(old: &Material, new: &Material, out: &mut String) {
    scalar(out, "name", &old.name, &new.name);
    scalar(out, "soilClass", &old.soil_class, &new.soil_class);

    for (name, value) in &new.properties {
        match old.properties.get(name) {
            None => {
                let _ = writeln!(out, "  + property {name}  {}", bounded(value));
            }
            Some(prev) if prev != value => {
                let _ = writeln!(
                    out,
                    "    property {name}  {} -> {}",
                    bounded(prev),
                    bounded(value)
                );
            }
            Some(_) => {}
        }
    }
    for name in old.properties.keys() {
        if !new.properties.contains_key(name) {
            let _ = writeln!(out, "  - property {name}");
        }
    }

    let old_kinds: Vec<&str> = old
        .constitutive_models
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    let new_kinds: Vec<&str> = new
        .constitutive_models
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    if old_kinds != new_kinds {
        let _ = writeln!(
            out,
            "    constitutive models  [{}] -> [{}]",
            old_kinds.join(", "),
            new_kinds.join(", ")
        );
    } else {
        for (a, b) in old.constitutive_models.iter().zip(&new.constitutive_models) {
            if a != b {
                let _ = writeln!(out, "    constitutive model {} changed", a.id);
                for (name, value) in &b.parameters {
                    match a.parameters.get(name) {
                        Some(prev) if prev != value => {
                            let _ = writeln!(
                                out,
                                "      {name}  {} -> {}",
                                bounded(prev),
                                bounded(value)
                            );
                        }
                        None => {
                            let _ = writeln!(out, "    + {name}  {}", bounded(value));
                        }
                        Some(_) => {}
                    }
                }
            }
        }
    }
}

fn scalar<T: PartialEq + std::fmt::Debug>(out: &mut String, name: &str, old: &T, new: &T) {
    if old != new {
        let _ = writeln!(out, "    {name}  {old:?} -> {new:?}");
    }
}

pub fn commit_line(info: &gm_core::store::CommitInfo) -> String {
    format!(
        "{}  {}  {:<24}  {}",
        short_hash(&info.hash),
        info.committed_at,
        info.author,
        info.message
    )
}
