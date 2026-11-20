//! Embedded real-time web monitoring dashboard (`--gui`).

pub mod api;
pub mod server;

pub use api::{build_router, is_sensitive_header, redact_headers, sanitize_url, AppState, MetaResponse};
pub use server::{open_browser_url, start_gui_server, GuiServerHandle};
