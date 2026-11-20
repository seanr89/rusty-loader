//! Embedded Axum HTTP server lifecycle and browser launch helper.

use std::net::SocketAddr;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::config::{BenchmarkConfig, GuiConfig};
use crate::gui::api::{build_router, AppState};
use crate::metrics::snapshot::TelemetryHub;
use crate::metrics::stats::LiveCounters;

const SERVER_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(500);

/// Handle to a running embedded GUI HTTP server.
pub struct GuiServerHandle {
    local_addr: SocketAddr,
    shutdown_token: CancellationToken,
    join_handle: JoinHandle<()>,
}

impl GuiServerHandle {
    /// Returns the bound [`SocketAddr`] of the GUI HTTP server.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Returns the HTTP URL where the dashboard is reachable (e.g. `http://127.0.0.1:8080`).
    pub fn url(&self) -> String {
        format!("http://{}", self.local_addr)
    }

    /// Gracefully shuts down the embedded HTTP server, aborting lingering keep-alive sockets
    /// if they remain open past the shutdown grace window.
    pub async fn shutdown(self) {
        self.shutdown_token.cancel();
        let mut join_handle = self.join_handle;
        if tokio::time::timeout(SERVER_SHUTDOWN_TIMEOUT, &mut join_handle)
            .await
            .is_err()
        {
            join_handle.abort();
            let _ = join_handle.await;
        }
    }
}

/// Starts the embedded Axum web dashboard server bound to `gui_config.socket_addr()`.
///
/// The HTTP server uses its own independent shutdown token so cancelling `benchmark_cancel_token`
/// (e.g. via `POST /api/stop` or `Ctrl+C`) stops the load test while keeping the server alive
/// long enough to deliver the final `complete` SSE event and JSON exports.
pub async fn start_gui_server(
    config: Arc<BenchmarkConfig>,
    gui_config: &GuiConfig,
    hub: Arc<TelemetryHub>,
    counters: Arc<LiveCounters>,
    benchmark_cancel_token: CancellationToken,
) -> Result<GuiServerHandle> {
    let bind_addr = gui_config.socket_addr();
    let listener = TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("failed to bind GUI server to {bind_addr}"))?;
    let local_addr = listener
        .local_addr()
        .context("failed to read local socket address for GUI server")?;

    let state = AppState {
        config,
        hub,
        counters,
        cancel_token: benchmark_cancel_token,
        started_at: Instant::now(),
    };
    let router = build_router(state);

    let shutdown_token = CancellationToken::new();
    let server_shutdown = shutdown_token.clone();

    let join_handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(server_shutdown.cancelled_owned())
            .await;
    });

    Ok(GuiServerHandle {
        local_addr,
        shutdown_token,
        join_handle,
    })
}

/// Best-effort non-blocking launch of the system default browser for `url`.
///
/// Failures (for example on headless Linux servers without a desktop environment) are ignored.
pub fn open_browser_url(url: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }

    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("xdg-open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }

    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("cmd")
            .args(["/C", "start", "", url])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}
