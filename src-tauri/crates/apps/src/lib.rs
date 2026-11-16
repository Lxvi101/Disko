//! Disko's apps crate: installed apps with everything that belongs to them (and uninstalling
//! them to the Trash), and coding-agent sessions per tool → project → session.
//!
//! Public API:
//! - [`apps()`] → `Vec<AppInfo>` (largest first)
//! - [`uninstall()`] → [`Uninstall`] (a `CleanResult` plus admin-deleted paths and notes)
//! - [`sessions()`] → `Vec<AgentTool>` (largest first)
//! - [`size::invalidate`] after removals made elsewhere (sizes are cached for 5 minutes)

pub mod apps;
pub mod sessions;
pub mod size;
pub mod types;
pub mod uninstall;
pub(crate) mod util;

pub use apps::apps;
pub use sessions::sessions;
pub use types::*;
pub use uninstall::uninstall;
