//! The 1D ground-model domain types.
//!
//! These are the *versioned* documents: what gets canonicalised, hashed and
//! stored as an object. The SQL tables in [`crate::schema`] are a materialised
//! view of whichever commit is checked out, not the authority.
//!
//! Two things are deliberately absent from every document here:
//!
//! * **Surrogate ids.** A model is addressed by `model_key`, a material by
//!   `material_key`. The UUID-ish `id` columns in the SQL view are derived from
//!   content hashes so that the same model materialises to the same row
//!   everywhere. See `docs/format.md`.
//! * **`created_at` / `updated_at`.** Timestamps live in commits. Embedding
//!   them in the document would change its hash on every save even when nothing
//!   about the ground actually changed, which would defeat deduplication and
//!   make diffs unreadable.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A quantity with an optional credible range, and optionally a variation with
/// depth. This is the single numeric primitive of the format: a ground model is
/// an interpretation, and a bare `f64` cannot say how well constrained it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Bounded {
    /// Best estimate / characteristic value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    /// Lower credible bound (often the cautious estimate for strength).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lower: Option<f64>,
    /// Upper credible bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upper: Option<f64>,
    /// Unit string, e.g. `kPa`, `kN/m3`, `deg`, `m/s`. Free text by design:
    /// pinning a unit ontology is a bigger fight than this format should pick.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Variation with depth. When present, `value`/`lower`/`upper` are a
    /// representative summary and the profile is authoritative.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Profile>,
}

impl Bounded {
    pub fn scalar(value: f64, unit: &str) -> Self {
        Self {
            value: Some(value),
            unit: Some(unit.to_string()),
            ..Default::default()
        }
    }

    pub fn ranged(value: f64, lower: f64, upper: f64, unit: &str) -> Self {
        Self {
            value: Some(value),
            lower: Some(lower),
            upper: Some(upper),
            unit: Some(unit.to_string()),
            profile: None,
        }
    }
}

/// A parameter that varies with depth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    pub interpolation: Interpolation,
    /// What `ProfilePoint::depth` is measured from.
    #[serde(default)]
    pub datum: ProfileDatum,
    pub points: Vec<ProfilePoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Interpolation {
    /// Linear between points; constant beyond the first and last point.
    Linear,
    /// Value holds from each point down to the next.
    Step,
}

/// Materials are reusable across every model in a file, so a profile expressed
/// as "depth below ground level" would mean something different in each model
/// that uses it. Making the datum explicit is the only way a shared material can
/// carry a depth-varying parameter honestly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileDatum {
    /// Depth below the top of this material's occurrence in whichever model is
    /// being evaluated. The only datum that travels correctly with a shared
    /// material, and therefore the default.
    #[default]
    LayerTop,
    /// Depth below the model's `surface_level`. Only meaningful when the
    /// material is used by a single model, or where all models share a surface.
    GroundLevel,
    /// Absolute level (mAOD or whatever `vertical_datum` says). `depth` is read
    /// as a level, increasing upwards, not a depth.
    Level,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfilePoint {
    /// Depth (or level, when `datum` is `level`) at which the values apply.
    pub depth: f64,
    pub value: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lower: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upper: Option<f64>,
}

/// Pore pressure regime for a model.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Groundwater {
    /// No pore water considered.
    Dry,
    /// Hydrostatic below a water table at `depth` below `surface_level`.
    #[serde(rename_all = "camelCase")]
    Hydrostatic { depth: f64 },
    /// An explicit pore pressure profile, in kPa.
    #[serde(rename_all = "camelCase")]
    Piezometric { profile: Profile },
    /// Genuinely not known. Distinct from `dry`, which is an assertion.
    #[default]
    Unknown,
}

/// A constitutive model attached to a material.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstitutiveModel {
    /// Stable key, unique within the material. Lets a design refer to "the
    /// drained set" across revisions.
    pub id: String,
    /// e.g. `mohr-coulomb`, `undrained-tresca`, `hardening-soil`. Open set:
    /// unknown kinds are carried through untouched and warned about, never
    /// rejected, so a file from a newer tool still round-trips.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drainage: Option<Drainage>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parameters: BTreeMap<String, Bounded>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Drainage {
    Drained,
    Undrained,
}

/// A reusable material. File-scoped: many models may reference the same one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Material {
    pub material_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub soil_class: Option<String>,
    /// General properties: `unitWeight`, `permeability`, and so on.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, Bounded>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constitutive_models: Vec<ConstitutiveModel>,
    /// How this material was arrived at: measured, assessed, assumed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// One stratum in a model. Layers do not exist independently of their model, so
/// they are stored inside the model document rather than as their own object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layer {
    /// Level of the top of this stratum, increasing upwards, in the file's
    /// `vertical_datum`. The base is the top of the next layer, and the base of
    /// the last layer is the model's `base_level`.
    pub top_level: f64,
    /// References [`Material::material_key`], not a surrogate id.
    pub material_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Evidence for this boundary: which log, which interpretation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<serde_json::Value>,
    /// True when the layer was generated by subdividing a parameter profile
    /// rather than picked from evidence.
    #[serde(default, skip_serializing_if = "is_false")]
    pub generated_from_profile: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Build settings that affect how the model is interpreted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[derive(Default)]
pub struct Settings {
    /// Whether a vertical gap between layers is permitted. Off by default: a
    /// gap is almost always a data-entry error rather than an intent.
    pub allow_gaps: bool,
    /// Thickness used when flattening a depth-varying parameter into discrete
    /// sublayers for a consumer that cannot handle profiles.
    pub profile_sublayer_thickness: Option<f64>,
}

/// A single 1D ground model: one vertical succession at one location.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroundModel {
    /// Human-readable identifier, unique within the file. This is the identity
    /// the object store tracks across revisions.
    pub model_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `1d` today. Present so a future 2D/3D model can coexist without the
    /// identity of existing models changing.
    #[serde(default = "default_model_type")]
    pub model_type: String,
    /// Level of the ground surface. Should equal the first layer's `top_level`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface_level: Option<f64>,
    /// Level of the base of the model. Without this the deepest layer has no
    /// bottom and the model has no vertical extent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_level: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    /// Unit weight of water, in kN/m3.
    #[serde(default = "default_gamma_w")]
    pub gamma_w: f64,
    #[serde(default)]
    pub groundwater: Groundwater,
    #[serde(default)]
    pub settings: Settings,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    /// Ordered from the top down. Array position *is* the layer order; there is
    /// no separate ordinal to fall out of step with the levels.
    #[serde(default)]
    pub layers: Vec<Layer>,
}

fn default_model_type() -> String {
    "1d".to_string()
}

fn default_gamma_w() -> f64 {
    9.81
}

impl GroundModel {
    pub fn new(model_key: impl Into<String>) -> Self {
        Self {
            model_key: model_key.into(),
            name: None,
            description: None,
            model_type: default_model_type(),
            surface_level: None,
            base_level: None,
            x: None,
            y: None,
            gamma_w: default_gamma_w(),
            groundwater: Groundwater::default(),
            settings: Settings::default(),
            metadata: None,
            layers: Vec::new(),
        }
    }

    /// Base level of the layer at `index`: the top of the next layer, or the
    /// model's `base_level` for the deepest one.
    pub fn layer_base(&self, index: usize) -> Option<f64> {
        match self.layers.get(index + 1) {
            Some(next) => Some(next.top_level),
            None => self.base_level,
        }
    }
}

/// File-level metadata. Exactly one of these per file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMetadata {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Horizontal CRS, e.g. `EPSG:27700`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crs: Option<String>,
    /// e.g. `Ordnance Datum Newlyn`. Every level in the file is in this datum.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertical_datum: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl FileMetadata {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            crs: None,
            vertical_datum: None,
            metadata: None,
        }
    }
}
