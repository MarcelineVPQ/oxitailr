//! Application state modules.

pub mod session;

// Session persistence is wired into the TUI in a later phase.
#[allow(unused_imports)]
pub use session::{
    load_local_sources, load_session, save_local_sources, save_session, SavedLocalSource,
    WindowState,
};
