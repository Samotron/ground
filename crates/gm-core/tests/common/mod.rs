//! Shared fixtures: a small but realistic two-chainage London Clay site.

use gm_core::FileMetadata;
use gm_core::model::{
    Bounded, ConstitutiveModel, Drainage, GroundModel, Groundwater, Layer, Material,
};
use gm_core::store::{Repository, State};
use std::collections::BTreeMap;
use std::path::Path;
use tempfile::TempDir;

pub const AUTHOR: &str = "test@example.com";

pub struct TestRepo {
    /// Held so the directory outlives the repository.
    pub _dir: TempDir,
    pub repo: Repository,
}

pub fn repo_at(path: &Path) -> Repository {
    let mut meta = FileMetadata::new("Test file");
    meta.crs = Some("EPSG:27700".into());
    meta.vertical_datum = Some("Ordnance Datum Newlyn".into());
    Repository::create(path, meta, AUTHOR).expect("create")
}

pub fn temp_repo() -> TestRepo {
    let dir = TempDir::new().expect("temp dir");
    let repo = repo_at(&dir.path().join("test.gm"));
    TestRepo { _dir: dir, repo }
}

pub fn made_ground() -> Material {
    Material {
        material_key: "MADE_GROUND".into(),
        name: Some("Made Ground".into()),
        description: None,
        soil_class: Some("anthropogenic".into()),
        properties: BTreeMap::from([(
            "unitWeight".to_string(),
            Bounded::ranged(19.0, 17.0, 21.0, "kN/m3"),
        )]),
        constitutive_models: vec![ConstitutiveModel {
            id: "mc-01".into(),
            kind: "mohr-coulomb".into(),
            drainage: Some(Drainage::Drained),
            parameters: BTreeMap::from([
                (
                    "frictionAngleDeg".to_string(),
                    Bounded::ranged(30.0, 27.0, 33.0, "deg"),
                ),
                ("cohesion".to_string(), Bounded::scalar(2.0, "kPa")),
            ]),
            metadata: None,
        }],
        provenance: None,
        metadata: None,
    }
}

pub fn london_clay() -> Material {
    Material {
        material_key: "LONDON_CLAY".into(),
        name: Some("London Clay".into()),
        description: None,
        soil_class: Some("clay".into()),
        properties: BTreeMap::from([(
            "unitWeight".to_string(),
            Bounded::ranged(20.0, 19.0, 21.0, "kN/m3"),
        )]),
        constitutive_models: vec![ConstitutiveModel {
            id: "ut-01".into(),
            kind: "undrained-tresca".into(),
            drainage: Some(Drainage::Undrained),
            parameters: BTreeMap::from([(
                "undrainedShearStrength".to_string(),
                Bounded::ranged(75.0, 60.0, 90.0, "kPa"),
            )]),
            metadata: None,
        }],
        provenance: None,
        metadata: None,
    }
}

/// Made Ground over London Clay, 20 m deep, water table at 2.5 m.
pub fn model(key: &str, surface: f64, clay_top: f64) -> GroundModel {
    let mut m = GroundModel::new(key);
    m.name = Some(format!("Model {key}"));
    m.surface_level = Some(surface);
    m.base_level = Some(surface - 20.0);
    m.x = Some(384100.0);
    m.y = Some(397200.0);
    m.groundwater = Groundwater::Hydrostatic { depth: 2.5 };
    m.layers = vec![
        Layer {
            top_level: surface,
            material_key: "MADE_GROUND".into(),
            description: None,
            source: None,
            generated_from_profile: false,
            metadata: None,
        },
        Layer {
            top_level: clay_top,
            material_key: "LONDON_CLAY".into(),
            description: None,
            source: None,
            generated_from_profile: false,
            metadata: None,
        },
    ];
    m
}

/// Fill the working tree with two chainages and the two materials they use.
pub fn populated(repo: &mut Repository) -> State {
    let mut state = repo.working().expect("working");
    state.materials.insert("MADE_GROUND".into(), made_ground());
    state.materials.insert("LONDON_CLAY".into(), london_clay());
    state
        .models
        .insert("CH-100".into(), model("CH-100", 82.5, 79.5));
    state
        .models
        .insert("CH-125".into(), model("CH-125", 81.9, 79.9));
    repo.write_working(&state).expect("write working");
    state
}
