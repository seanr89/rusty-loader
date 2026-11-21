#[cfg(feature = "gui")]
use std::sync::Arc;
#[cfg(feature = "gui")]
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use rusty_loader::changelog::full_changelog;
use rusty_loader::cli::Cli;
use rusty_loader::config::{BenchmarkConfig, GuiConfig};
use rusty_loader::engine::TestController;
#[cfg(feature = "gui")]
use rusty_loader::gui::{open_browser_url, start_gui_server};
#[cfg(feature = "gui")]
use rusty_loader::metrics::TelemetryHub;
use rusty_loader::os::check_and_warn_os_limits;
use rusty_loader::reporter::{report_results, spawn_live_progress_bar};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.changelog {
        print!("{}", full_changelog());
        return Ok(());
    }
    let gui_config = GuiConfig::from_cli(&cli)?;
    let config = BenchmarkConfig::from_cli(cli)?;

    check_and_warn_os_limits();

    #[cfg(not(feature = "gui"))]
    if gui_config.enabled {
        anyhow::bail!("--gui requested, but this binary was compiled without the 'gui' feature");
    }

    #[cfg(feature = "gui")]
    let mut controller = TestController::from_config(config.clone())?;
    #[cfg(not(feature = "gui"))]
    let controller = TestController::from_config(config.clone())?;

    #[cfg(feature = "gui")]
    let gui_handle = if gui_config.enabled {
        let hub = Arc::new(TelemetryHub::new(config.duration, config.ramp_up));
        controller = controller.with_telemetry_hub(Arc::clone(&hub));

        let handle = start_gui_server(
            Arc::clone(controller.config()),
            &gui_config,
            hub,
            controller.counters(),
            controller.cancel_token(),
        )
        .await?;

        eprintln!("Live GUI dashboard listening at {}", handle.url());

        if gui_config.open_browser {
            let open_url = if handle.local_addr().ip().is_unspecified() {
                let port = handle.local_addr().port();
                if handle.local_addr().is_ipv6() {
                    format!("http://[::1]:{port}")
                } else {
                    format!("http://127.0.0.1:{port}")
                }
            } else {
                handle.url()
            };
            open_browser_url(&open_url);
        }

        Some(handle)
    } else {
        None
    };

    let progress_handle = spawn_live_progress_bar(
        config.duration,
        controller.counters(),
        controller.cancel_token(),
    );

    let stats = controller.run().await?;

    // Ensure the progress bar task is cancelled and cleared before printing tables.
    controller.cancel_token().cancel();
    let _ = progress_handle.await;

    report_results(&config, &stats)?;

    #[cfg(feature = "gui")]
    if let Some(handle) = gui_handle {
        if gui_config.keep_alive {
            eprintln!(
                "GUI keep-alive enabled at {} (press Ctrl+C to exit)...",
                handle.url()
            );
            let _ = tokio::signal::ctrl_c().await;
            handle.shutdown().await;
        } else {
            // Allow active SSE listeners a short window to consume the final `complete` event.
            tokio::time::sleep(Duration::from_millis(150)).await;
            handle.shutdown().await;
        }
    }

    Ok(())
}
