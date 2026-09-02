//! Core of the ground-model file format.
//!
//! A ground-model file is one SQLite database holding two things: a
//! content-addressed object store with the full revision history of every 1D
//! model and material, and a materialised SQL view of the currently checked-out
//! revision that any tool can read without knowing this crate exists.
//!
//! ```text
//!  gm_blob / gm_commit / gm_entry / gm_ref     authoritative, versioned
//!                  |
//!                  v  materialise
//!  file_metadata / materials / ground_models / ground_layers
//!                                              queryable, replaceable
//! ```

pub mod canon;
pub mod commit;
pub mod error;
pub mod exchange;
pub mod model;
pub mod schema;
pub mod store;
pub mod validate;

pub use error::{Error, Result};
pub use exchange::Exchange;
pub use model::{
    Bounded, ConstitutiveModel, Drainage, FileMetadata, GroundModel, Groundwater, Interpolation,
    Layer, Material, Profile, ProfileDatum, ProfilePoint, Settings,
};
pub use store::{Change, ChangeKind, CommitInfo, Repository, State};
pub use validate::{Issue, Severity};
