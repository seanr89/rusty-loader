use anyhow::Result;
use clap::Parser;
use rusty_loader::cli::Cli;
use rusty_loader::config::BenchmarkConfig;
use rusty_loader::engine::TestController;
use rusty_loader::os::check_and_warn_os_limits;
use rusty_loader::reporter::{report_results, spawn_live_progress_bar};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = BenchmarkConfig::from_cli(cli)?;

    check_and_warn_os_limits();

    let controller = TestController::from_config(config.clone())?;
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

    Ok(())
}
