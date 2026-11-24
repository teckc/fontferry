#![forbid(unsafe_code)]

mod candidate;
pub mod catalog;
pub mod engine;
pub mod error;
mod operation;
pub mod ports;
pub mod version;

pub use candidate::{InstallCandidate, select_release_assets, select_variant_assets};
pub use catalog::*;
pub use engine::*;
pub use error::*;
pub use operation::OperationGuard;
pub use ports::*;
pub use version::*;
