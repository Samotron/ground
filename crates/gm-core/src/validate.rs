//! Validation.
//!
//! Split into errors and warnings on one principle: an **error** means the file
//! does not describe a coherent ground model and a consumer would be wrong to
//! use it; a **warning** means the model is coherent but suspicious. Geotechnics
//! is full of legitimately odd ground, so the bar for an error is high — a
//! validator that refuses plausible ground is one that gets switched off.

use crate::model::{
    Bounded, ConstitutiveModel, GroundModel, Groundwater, Interpolation, Material, Profile,
    ProfileDatum,
};
use crate::store::State;
use crate::vocabulary;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub severity: Severity,
    /// The model this concerns, when it concerns one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_key: Option<String>,
    /// A path into the interchange document, e.g. `layers[2].topLevel` or
    /// `materials.LONDON_CLAY.properties.unitWeight`. Model-scoped paths are
    /// relative to the model named by `model_key`. camelCase throughout,
    /// because that is what the document itself uses.
    pub field_path: String,
    pub message: String,
}

impl Issue {
    fn error(model: Option<&str>, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            model_key: model.map(str::to_string),
            field_path: path.into(),
            message: message.into(),
        }
    }

    fn warn(model: Option<&str>, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            model_key: model.map(str::to_string),
            field_path: path.into(),
            message: message.into(),
        }
    }
}

/// Validate a whole file.
pub fn validate_state(state: &State) -> Vec<Issue> {
    let mut issues = Vec::new();
    let known_materials: BTreeSet<&str> = state.materials.keys().map(String::as_str).collect();

    if state.file_metadata.vertical_datum.is_none() {
        issues.push(Issue::warn(
            None,
            "file.verticalDatum",
            "no vertical datum declared; every level in this file is therefore ambiguous",
        ));
    }
    if state.file_metadata.crs.is_none()
        && state
            .models
            .values()
            .any(|m| m.x.is_some() || m.y.is_some())
    {
        issues.push(Issue::warn(
            None,
            "file.crs",
            "models carry coordinates but the file declares no CRS",
        ));
    }

    for (key, material) in &state.materials {
        validate_material(key, material, &mut issues);
    }

    let mut used: BTreeSet<&str> = BTreeSet::new();
    for model in state.models.values() {
        for layer in &model.layers {
            used.insert(layer.material_key.as_str());
        }
        validate_model(model, &known_materials, &mut issues);
    }

    for unused in known_materials.difference(&used) {
        issues.push(Issue::warn(
            None,
            format!("materials.{unused}"),
            format!("material '{unused}' is not used by any model"),
        ));
    }

    issues
}

pub fn validate_model(
    model: &GroundModel,
    known_materials: &BTreeSet<&str>,
    issues: &mut Vec<Issue>,
) {
    let key = model.model_key.as_str();
    let allow_gaps = model.settings.allow_gaps;

    if model.model_key.trim().is_empty() {
        issues.push(Issue::error(Some(key), "modelKey", "model key is empty"));
    }
    if !(9.0..=11.0).contains(&model.gamma_w) {
        issues.push(Issue::warn(
            Some(key),
            "gammaW",
            format!(
                "unit weight of water is {} kN/m3, which is outside the plausible 9-11 range",
                model.gamma_w
            ),
        ));
    }

    if model.layers.is_empty() {
        issues.push(Issue::warn(Some(key), "layers", "model has no layers"));
        return;
    }

    // Strictly decreasing tops: a 1D succession reads downwards, and equal tops
    // would mean a zero-thickness stratum.
    for window in model.layers.windows(2).enumerate() {
        let (i, pair) = window;
        let (upper, lower) = (&pair[0], &pair[1]);
        if lower.top_level > upper.top_level {
            issues.push(Issue::error(
                Some(key),
                format!("layers[{}].topLevel", i + 1),
                format!(
                    "layer {} starts at {} which is above layer {} at {}; layers must be ordered downwards",
                    i + 2,
                    lower.top_level,
                    i + 1,
                    upper.top_level
                ),
            ));
        } else if lower.top_level == upper.top_level {
            issues.push(Issue::error(
                Some(key),
                format!("layers[{}].topLevel", i + 1),
                format!(
                    "layers {} and {} both start at {}, giving a zero-thickness stratum",
                    i + 1,
                    i + 2,
                    upper.top_level
                ),
            ));
        }
    }

    let first_top = model.layers[0].top_level;
    let last_top = model.layers[model.layers.len() - 1].top_level;

    match model.surface_level {
        None => issues.push(Issue::warn(
            Some(key),
            "surfaceLevel",
            "no surface level; depths below ground cannot be computed for this model",
        )),
        Some(surface) if surface != first_top => {
            let message = format!(
                "surface level {surface} does not meet the top of the first layer at {first_top}"
            );
            issues.push(if allow_gaps {
                Issue::warn(Some(key), "surfaceLevel", message)
            } else {
                Issue::error(Some(key), "surfaceLevel", message)
            });
        }
        Some(_) => {}
    }

    match model.base_level {
        None => issues.push(Issue::warn(
            Some(key),
            "baseLevel",
            "no base level; the deepest layer has no bottom and the model has no vertical extent",
        )),
        Some(base) if base >= last_top => issues.push(Issue::error(
            Some(key),
            "baseLevel",
            format!("base level {base} is not below the top of the deepest layer at {last_top}"),
        )),
        Some(_) => {}
    }

    for (i, layer) in model.layers.iter().enumerate() {
        let path = format!("layers[{i}]");
        if !known_materials.contains(layer.material_key.as_str()) {
            issues.push(Issue::error(
                Some(key),
                format!("{path}.materialKey"),
                format!(
                    "references material '{}', which the file does not define",
                    layer.material_key
                ),
            ));
        }
        if !layer.top_level.is_finite() {
            issues.push(Issue::error(
                Some(key),
                format!("{path}.topLevel"),
                "top level is not a finite number",
            ));
        }
    }

    validate_groundwater(model, issues);
}

fn validate_groundwater(model: &GroundModel, issues: &mut Vec<Issue>) {
    let key = model.model_key.as_str();
    match &model.groundwater {
        Groundwater::Unknown => issues.push(Issue::warn(
            Some(key),
            "groundwater",
            "groundwater regime is unknown; consumers cannot compute effective stress",
        )),
        Groundwater::Hydrostatic { depth } => {
            if *depth < 0.0 {
                issues.push(Issue::warn(
                    Some(key),
                    "groundwater.depth",
                    format!("water table is {} m above ground level", -depth),
                ));
            }
            if let (Some(surface), Some(base)) = (model.surface_level, model.base_level) {
                if surface - depth < base {
                    issues.push(Issue::warn(
                        Some(key),
                        "groundwater.depth",
                        "water table is below the base of the model, so the model is entirely dry",
                    ));
                }
            }
        }
        Groundwater::Piezometric { profile } => {
            validate_profile(Some(key), "groundwater.profile", profile, issues);
        }
        Groundwater::Dry => {}
    }
}

pub fn validate_material(key: &str, material: &Material, issues: &mut Vec<Issue>) {
    let path = |suffix: &str| format!("materials.{key}.{suffix}");

    for (name, bounded) in &material.properties {
        validate_bounded(&path(&format!("properties.{name}")), bounded, issues);
    }

    if material.constitutive_models.is_empty() {
        issues.push(Issue::warn(
            None,
            path("constitutiveModels"),
            format!("material '{key}' has no constitutive model, so it cannot be analysed"),
        ));
    }

    let mut seen_ids = BTreeSet::new();
    for cm in &material.constitutive_models {
        if !seen_ids.insert(cm.id.as_str()) {
            issues.push(Issue::error(
                None,
                path(&format!("constitutiveModels.{}", cm.id)),
                format!(
                    "duplicate constitutive model id '{}' in material '{key}'",
                    cm.id
                ),
            ));
        }
        validate_constitutive(key, cm, issues);
    }

    // Unit weight is the one property nearly every calculation needs.
    if !material.properties.contains_key("unitWeight") {
        issues.push(Issue::warn(
            None,
            path("properties.unitWeight"),
            format!("material '{key}' has no unit weight; self-weight stress cannot be computed"),
        ));
    } else if let Some(uw) = material.properties.get("unitWeight").and_then(|b| b.value) {
        if !(10.0..=30.0).contains(&uw) {
            issues.push(Issue::warn(
                None,
                path("properties.unitWeight"),
                format!(
                    "unit weight of {uw} kN/m3 is outside the usual 10-30 range for soil and rock"
                ),
            ));
        }
    }
}

fn validate_constitutive(material_key: &str, cm: &ConstitutiveModel, issues: &mut Vec<Issue>) {
    let path = |suffix: &str| {
        format!(
            "materials.{material_key}.constitutiveModels.{}.{suffix}",
            cm.id
        )
    };

    if !vocabulary::knows_constitutive_kind(&cm.kind) {
        issues.push(Issue::warn(
            None,
            path("kind"),
            format!(
                "constitutive model kind '{}' is not one this build knows; its parameters are preserved but unchecked",
                cm.kind
            ),
        ));
    }

    for (name, bounded) in &cm.parameters {
        validate_bounded(&path(&format!("parameters.{name}")), bounded, issues);
    }

    // Kind-specific sanity, only for the kinds we claim to understand.
    match cm.kind.as_str() {
        "mohr-coulomb" => {
            if let Some(phi) = cm.parameters.get("frictionAngleDeg").and_then(|b| b.value) {
                if !(0.0..90.0).contains(&phi) {
                    issues.push(Issue::error(
                        None,
                        path("parameters.frictionAngleDeg"),
                        format!("friction angle of {phi} degrees is not physically meaningful"),
                    ));
                }
            } else {
                issues.push(Issue::warn(
                    None,
                    path("parameters.frictionAngleDeg"),
                    "Mohr-Coulomb model has no friction angle",
                ));
            }
            if !cm.parameters.contains_key("cohesion") {
                issues.push(Issue::warn(
                    None,
                    path("parameters.cohesion"),
                    "Mohr-Coulomb model has no cohesion; assumed zero by most consumers",
                ));
            }
        }
        "undrained-tresca" => {
            if !cm.parameters.contains_key("undrainedShearStrength") {
                issues.push(Issue::warn(
                    None,
                    path("parameters.undrainedShearStrength"),
                    "undrained model has no undrained shear strength",
                ));
            }
            if cm.drainage == Some(crate::model::Drainage::Drained) {
                issues.push(Issue::warn(
                    None,
                    path("drainage"),
                    "an undrained Tresca model is marked as drained",
                ));
            }
        }
        _ => {}
    }
}

pub fn validate_bounded(path: &str, bounded: &Bounded, issues: &mut Vec<Issue>) {
    for (name, v) in [
        ("value", bounded.value),
        ("lower", bounded.lower),
        ("upper", bounded.upper),
    ] {
        if let Some(v) = v {
            if !v.is_finite() {
                issues.push(Issue::error(
                    None,
                    format!("{path}.{name}"),
                    "not a finite number",
                ));
            }
        }
    }

    if let (Some(lower), Some(upper)) = (bounded.lower, bounded.upper) {
        if lower > upper {
            issues.push(Issue::error(
                None,
                format!("{path}.lower"),
                format!("lower bound {lower} exceeds upper bound {upper}"),
            ));
        }
    }
    if let (Some(value), Some(lower)) = (bounded.value, bounded.lower) {
        if value < lower {
            issues.push(Issue::error(
                None,
                format!("{path}.value"),
                format!("value {value} is below its own lower bound {lower}"),
            ));
        }
    }
    if let (Some(value), Some(upper)) = (bounded.value, bounded.upper) {
        if value > upper {
            issues.push(Issue::error(
                None,
                format!("{path}.value"),
                format!("value {value} is above its own upper bound {upper}"),
            ));
        }
    }
    if bounded.value.is_none() && bounded.profile.is_none() {
        issues.push(Issue::warn(
            None,
            path.to_string(),
            "parameter has neither a value nor a depth profile",
        ));
    }
    if bounded.unit.is_none() {
        issues.push(Issue::warn(
            None,
            format!("{path}.unit"),
            "no unit given, so the number cannot be interpreted safely",
        ));
    }

    if let Some(profile) = &bounded.profile {
        validate_profile(None, path, profile, issues);
    }
}

pub fn validate_profile(
    model_key: Option<&str>,
    path: &str,
    profile: &Profile,
    issues: &mut Vec<Issue>,
) {
    if profile.points.is_empty() {
        issues.push(Issue::error(
            model_key,
            format!("{path}.points"),
            "profile has no points",
        ));
        return;
    }
    if profile.points.len() == 1 && profile.interpolation == Interpolation::Linear {
        issues.push(Issue::warn(
            model_key,
            format!("{path}.points"),
            "a linear profile with one point is just a constant; consider a plain value",
        ));
    }

    // Points read downwards for depth-based data and upwards for levels, so the
    // required ordering flips with the datum.
    let descending_levels = profile.datum == ProfileDatum::Level;
    for (i, pair) in profile.points.windows(2).enumerate() {
        let (a, b) = (&pair[0], &pair[1]);
        let out_of_order = if descending_levels {
            b.depth >= a.depth
        } else {
            b.depth <= a.depth
        };
        if out_of_order {
            let axis = if descending_levels {
                "levels must decrease"
            } else {
                "depths must increase"
            };
            issues.push(Issue::error(
                model_key,
                format!("{path}.points[{}]", i + 1),
                format!("profile points are out of order: {axis} down the profile"),
            ));
        }
    }

    for (i, point) in profile.points.iter().enumerate() {
        if !point.depth.is_finite() || !point.value.is_finite() {
            issues.push(Issue::error(
                model_key,
                format!("{path}.points[{i}]"),
                "profile point has a non-finite depth or value",
            ));
        }
        if let (Some(lower), Some(upper)) = (point.lower, point.upper) {
            if lower > upper {
                issues.push(Issue::error(
                    model_key,
                    format!("{path}.points[{i}].lower"),
                    format!("lower bound {lower} exceeds upper bound {upper}"),
                ));
            }
        }
    }
}

/// True when nothing in `issues` would stop a consumer using the file.
pub fn is_usable(issues: &[Issue]) -> bool {
    !issues.iter().any(|i| i.severity == Severity::Error)
}

pub fn count(issues: &[Issue]) -> (usize, usize) {
    let errors = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .count();
    (errors, issues.len() - errors)
}
