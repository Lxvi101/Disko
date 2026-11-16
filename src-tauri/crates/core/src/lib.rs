//! Disko's shared vocabulary: what a piece of clutter is, how the disk tree is described to the UI
//! and to agents, and the two ways Disko removes things (Trash, or delete for rebuildable data).
//!
//! Every other crate depends on this one and nothing else from the workspace, so they can be
//! built and tested independently. Keep it small and dependency-light.

pub mod ops;
pub mod paths;
pub mod types;

pub use types::*;
