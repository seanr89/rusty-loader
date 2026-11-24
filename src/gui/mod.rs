//! Embedded real-time web monitoring dashboard (`--gui`) and interactive desktop workbench.

pub mod api;
pub mod interactive;
pub mod manager;
pub mod server;

pub use api::{
    build_router, is_sensitive_header, meta_from_config, redact_headers, sanitize_url, AppState,
    GuiMode, MetaResponse,
};
pub use interactive::{
    build_interactive_router, generate_session_token, DialogDispatcher, InteractiveState,
    NativeDialogRequest,
};
pub use manager::{
    CompletedRunRecord, CompletedRunSummary, RunLifecycleEvent, RunManager, RunManagerError,
};
pub use server::{
    open_browser_url, start_gui_server, start_interactive_gui_server, GuiServerHandle,
    InteractiveServerHandle,
};
