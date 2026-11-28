use std::sync::Arc;
#[cfg(feature = "gui")]
use std::time::Duration;

use anyhow::{Context, Result};
use clap::parser::ValueSource;
use clap::{ArgMatches, CommandFactory, FromArgMatches};
use rusty_loader::changelog::full_changelog;
use rusty_loader::cli::{parse_header, Cli};
use rusty_loader::config::{
    validate_input_with_review, BenchmarkConfig, BenchmarkConfigInput, BodyMode, GuiConfig,
    HeaderEntry,
};
use rusty_loader::engine::TestController;
#[cfg(feature = "gui")]
use rusty_loader::gui::{open_browser_url, start_gui_server, start_interactive_gui_server, RunManager};
use rusty_loader::metrics::snapshot::{TelemetryHub, TelemetrySnapshot};
use rusty_loader::os::{check_and_warn_os_limits, OsLimits};
#[cfg(feature = "gui")]
use rusty_loader::os::raise_nofile_soft_limit;
#[cfg(feature = "gui")]
use rusty_loader::presets::PresetStore;
use rusty_loader::reporter::{report_results_with_snapshots, spawn_live_progress_bar};
use rusty_loader::scenario::{
    execute_dry_run, load_scenario_file, render_dry_run_report, render_review_report, RequestMode,
};

fn main() -> Result<()> {
    let matches = Cli::command().get_matches();
    let cli = Cli::from_arg_matches(&matches)?;

    if cli.changelog {
        print!("{}", full_changelog());
        return Ok(());
    }

    if cli.desktop {
        let initial = initial_input_from_cli(&cli, &matches)?;
        #[cfg(all(feature = "desktop", target_os = "macos"))]
        {
            return rusty_loader::desktop::run_desktop_app(initial);
        }
        #[cfg(not(all(feature = "desktop", target_os = "macos")))]
        {
            let _ = initial;
            anyhow::bail!(
                "--desktop is only supported on macOS with the 'desktop' feature enabled; use --ui-serve to open the interactive web UI in a browser"
            );
        }
    }

    if cli.ui_serve {
        #[cfg(not(feature = "gui"))]
        anyhow::bail!("--ui-serve requested, but this binary was compiled without the 'gui' feature");

        #[cfg(feature = "gui")]
        {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .context("failed to initialize Tokio runtime")?;
            return rt.block_on(run_ui_serve(cli, matches));
        }
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to initialize Tokio runtime")?;
    rt.block_on(run_cli_benchmark(cli, matches))
}

fn is_cli_flag_explicit(matches: &ArgMatches, id: &str) -> bool {
    matches!(matches.value_source(id), Some(ValueSource::CommandLine))
}

fn load_scenario_with_cli_overrides(
    cli: &Cli,
    matches: &ArgMatches,
    scenario_path: &std::path::Path,
) -> Result<BenchmarkConfigInput> {
    let mut input = load_scenario_file(scenario_path)?;
    if is_cli_flag_explicit(matches, "concurrency") {
        input.concurrency = cli.concurrency;
    }
    if is_cli_flag_explicit(matches, "duration") {
        input.duration = format!("{}ms", cli.duration.as_millis());
    }
    if is_cli_flag_explicit(matches, "ramp_up") {
        input.ramp_up = format!("{}ms", cli.ramp_up.as_millis());
    }
    if is_cli_flag_explicit(matches, "timeout") {
        input.timeout = format!("{}ms", cli.timeout.as_millis());
    }
    if let Some(ref p) = cli.output_json {
        input.output_json = Some(p.clone());
    }
    if let Some(ref p) = cli.output_csv {
        input.output_csv = Some(p.clone());
    }
    if let Some(ref p) = cli.output_pdf {
        input.output_pdf = Some(p.clone());
    }
    Ok(input)
}

fn single_input_from_cli(cli: &Cli) -> BenchmarkConfigInput {
    let headers = cli
        .headers
        .iter()
        .filter_map(|raw| {
            parse_header(raw).ok().map(|(name, value)| HeaderEntry {
                name: name.as_str().to_string(),
                value: value.to_str().unwrap_or("").to_string(),
                enabled: true,
            })
        })
        .collect();

    let (body_mode, body, body_file) = match cli.body.as_deref() {
        Some(raw) if raw.starts_with('@') => {
            let p = raw.trim_start_matches('@').trim();
            if p.is_empty() {
                (BodyMode::None, None, None)
            } else {
                (BodyMode::File, None, Some(p.into()))
            }
        }
        Some(raw) if !raw.is_empty() => (BodyMode::Raw, Some(raw.to_string()), None),
        _ => (BodyMode::None, None, None),
    };

    BenchmarkConfigInput {
        mode: RequestMode::Single,
        url: cli.url.clone(),
        method: cli.method,
        concurrency: cli.concurrency,
        duration: format!("{}ms", cli.duration.as_millis()),
        ramp_up: format!("{}ms", cli.ramp_up.as_millis()),
        timeout: format!("{}ms", cli.timeout.as_millis()),
        headers,
        body_mode,
        body,
        body_file,
        output_json: cli.output_json.clone(),
        output_csv: cli.output_csv.clone(),
        output_pdf: cli.output_pdf.clone(),
        scenario: None,
    }
}

fn initial_input_from_cli(
    cli: &Cli,
    matches: &ArgMatches,
) -> Result<Option<BenchmarkConfigInput>> {
    if let Some(ref scenario_path) = cli.scenario {
        return Ok(Some(load_scenario_with_cli_overrides(
            cli,
            matches,
            scenario_path,
        )?));
    }

    if cli.url.trim().is_empty() {
        return Ok(None);
    }

    Ok(Some(single_input_from_cli(cli)))
}

#[cfg(feature = "gui")]
async fn run_ui_serve(cli: Cli, matches: ArgMatches) -> Result<()> {
    let gui_config = GuiConfig::from_cli(&cli)?;
    let _ = raise_nofile_soft_limit(65_536);
    check_and_warn_os_limits();

    let initial = initial_input_from_cli(&cli, &matches)?;
    let manager = Arc::new(RunManager::new());
    let presets = Arc::new(PresetStore::load_default()?);

    let handle = start_interactive_gui_server(
        gui_config.socket_addr(),
        Arc::clone(&manager),
        presets,
        None,
        initial,
    )
    .await?;

    let tokenized_url = handle.tokenized_url();
    eprintln!("Interactive UI workbench listening at {tokenized_url}");
    eprintln!("Press Ctrl+C to shut down.");

    if cli.gui_open {
        open_browser_url(&tokenized_url);
    }

    let _ = tokio::signal::ctrl_c().await;
    manager.shutdown().await;
    handle.shutdown().await;
    Ok(())
}

async fn run_cli_benchmark(cli: Cli, matches: ArgMatches) -> Result<()> {
    let gui_config = GuiConfig::from_cli(&cli)?;

    if cli.review {
        let input = match cli.scenario.as_deref() {
            Some(path) => load_scenario_with_cli_overrides(&cli, &matches, path)?,
            None => single_input_from_cli(&cli),
        };
        let (report, review_opt) = validate_input_with_review(&input, Some(OsLimits::detect()));
        if let Some(review) = review_opt {
            print!("{}", render_review_report(&review));
        } else {
            println!(
                "Single-Request Review: {} {} (concurrency: {}, duration: {}, timeout: {})",
                input.method, input.url, input.concurrency, input.duration, input.timeout
            );
            for warn in &report.warnings {
                println!("Warning: {warn}");
            }
            for (field, err) in &report.field_errors {
                println!("Error [{field}]: {err}");
            }
        }
        if !report.valid {
            anyhow::bail!("configuration validation failed");
        }
        return Ok(());
    }

    let config = if let Some(ref scenario_path) = cli.scenario {
        let input = load_scenario_with_cli_overrides(&cli, &matches, scenario_path)?;
        BenchmarkConfig::from_input(input)?
    } else {
        BenchmarkConfig::from_cli(cli.clone())?
    };

    if cli.dry_run {
        let trace = execute_dry_run(&config).await?;
        print!("{}", render_dry_run_report(&trace));
        if !trace.completed {
            anyhow::bail!("dry-run iteration failed or aborted");
        }
        return Ok(());
    }

    check_and_warn_os_limits();

    #[cfg(not(feature = "gui"))]
    if gui_config.enabled {
        anyhow::bail!("--gui requested, but this binary was compiled without the 'gui' feature");
    }

    let needs_hub = {
        #[cfg(feature = "gui")]
        let gui_enabled = gui_config.enabled;
        #[cfg(not(feature = "gui"))]
        let gui_enabled = false;
        gui_enabled || config.output_csv.is_some() || config.output_pdf.is_some()
    };

    let telemetry_hub =
        needs_hub.then(|| Arc::new(TelemetryHub::new(config.duration, config.ramp_up)));

    let mut controller = TestController::from_config(config.clone())?;
    if let Some(ref hub) = telemetry_hub {
        controller = controller.with_telemetry_hub(Arc::clone(hub));
    }

    #[cfg(feature = "gui")]
    let gui_handle = if gui_config.enabled {
        let hub = Arc::clone(
            telemetry_hub
                .as_ref()
                .expect("telemetry hub initialized when --gui is enabled"),
        );

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

    let snapshots: Vec<TelemetrySnapshot> = telemetry_hub
        .as_ref()
        .map(|hub| hub.history().into_iter().map(|s| (*s).clone()).collect())
        .unwrap_or_default();

    report_results_with_snapshots(&config, &stats, &snapshots)?;

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
