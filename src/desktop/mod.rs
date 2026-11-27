//! Native macOS desktop window (`tao` + `wry` WKWebView), native menus (`muda`),
//! and native Open/Save dialogs (`rfd`) hosting the interactive loopback workbench.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use anyhow::{Context, Result};
use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

use crate::changelog::APP_VERSION;
use crate::config::BenchmarkConfigInput;
use crate::gui::{
    start_interactive_gui_server, DialogDispatcher, NativeDialogRequest, RunManager,
};
use crate::os::raise_nofile_soft_limit;
use crate::presets::PresetStore;

const QUIT_MENU_ID: &str = "app.quit";

enum DesktopEvent {
    Dialog(NativeDialogRequest),
    QuitRequested,
}

/// Launches the native macOS desktop application window on the process main thread.
pub fn run_desktop_app(initial_config: Option<BenchmarkConfigInput>) -> Result<()> {
    // LaunchServices commonly starts GUI bundles with a 256 soft FD limit; raise best-effort early.
    let _ = raise_nofile_soft_limit(65_536);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to initialize Tokio runtime for desktop UI")?;

    let event_loop = EventLoopBuilder::<DesktopEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let dispatcher: DialogDispatcher = Arc::new(move |req| {
        proxy.send_event(DesktopEvent::Dialog(req)).is_ok()
    });

    let manager = Arc::new(RunManager::new());
    let presets = Arc::new(PresetStore::load_default()?);
    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);

    let server_handle = rt.block_on(start_interactive_gui_server(
        bind_addr,
        Arc::clone(&manager),
        Arc::clone(&presets),
        Some(dispatcher),
        initial_config,
    ))?;

    let menu = build_macos_menu()?;
    menu.init_for_nsapp();

    let menu_proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id().as_ref() == QUIT_MENU_ID {
            let _ = menu_proxy.send_event(DesktopEvent::QuitRequested);
        }
    }));

    let window = WindowBuilder::new()
        .with_title("rusty-loader")
        .with_inner_size(LogicalSize::new(1360.0, 880.0))
        .with_min_inner_size(LogicalSize::new(1024.0, 680.0))
        .build(&event_loop)
        .context("failed to create macOS application window")?;

    let token_literal = serde_json::to_string(server_handle.session_token())
        .context("failed to serialize desktop session token")?;
    let init_script = format!("window.__RL_SESSION_TOKEN__ = {token_literal};");

    let webview = WebViewBuilder::new()
        .with_url(server_handle.tokenized_url())
        .with_initialization_script(init_script)
        .with_devtools(cfg!(debug_assertions))
        .build(&window)
        .context("failed to initialize WKWebView")?;

    let mut server_opt = Some(server_handle);
    let mut shut_down = false;

    let mut shutdown_all = move || {
        if shut_down {
            return;
        }
        shut_down = true;
        if let Some(handle) = server_opt.take() {
            rt.block_on(async {
                manager.shutdown().await;
                handle.shutdown().await;
            });
        }
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        let _keep_webview_alive = &webview;
        let _keep_window_alive = &window;
        let _keep_menu_alive = &menu;

        match event {
            Event::UserEvent(DesktopEvent::Dialog(req)) => match req {
                NativeDialogRequest::OpenBodyFile { resp } => {
                    let chosen = rfd::FileDialog::new()
                        .set_title("Select Request Payload File")
                        .pick_file();
                    let _ = resp.send(chosen);
                }
                NativeDialogRequest::SaveReportFile {
                    format,
                    default_name,
                    resp,
                } => {
                    let (filter_label, extensions) = format.dialog_filter();
                    let title = format!("Export Benchmark {}", filter_label);
                    let chosen = rfd::FileDialog::new()
                        .set_title(&title)
                        .set_file_name(&default_name)
                        .add_filter(filter_label, extensions)
                        .save_file();
                    let _ = resp.send(chosen);
                }
                NativeDialogRequest::OpenScenarioFile { resp } => {
                    let chosen = rfd::FileDialog::new()
                        .set_title("Import API Sequence JSON")
                        .add_filter("Scenario JSON", &["json"])
                        .pick_file();
                    let _ = resp.send(chosen);
                }
                NativeDialogRequest::SaveScenarioFile { default_name, resp } => {
                    let chosen = rfd::FileDialog::new()
                        .set_title("Export API Sequence JSON")
                        .set_file_name(&default_name)
                        .add_filter("Scenario JSON", &["json"])
                        .save_file();
                    let _ = resp.send(chosen);
                }
            },
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            }
            | Event::UserEvent(DesktopEvent::QuitRequested) => {
                shutdown_all();
                *control_flow = ControlFlow::Exit;
            }
            Event::LoopDestroyed => {
                shutdown_all();
            }
            _ => {}
        }
    });
}

fn build_macos_menu() -> Result<Menu> {
    let quit_accel = Accelerator::new(Modifiers::META, Code::KeyQ);
    let quit_item = MenuItem::with_id(QUIT_MENU_ID, "Quit rusty-loader", true, Some(quit_accel));

    let about_meta = AboutMetadata {
        name: Some("rusty-loader".to_string()),
        version: Some(APP_VERSION.to_string()),
        ..Default::default()
    };

    let app_menu = Submenu::with_items(
        "rusty-loader",
        true,
        &[
            &PredefinedMenuItem::about(Some("About rusty-loader"), Some(about_meta)),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &quit_item,
        ],
    )?;

    let edit_menu = Submenu::with_items(
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(None),
            &PredefinedMenuItem::redo(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::select_all(None),
        ],
    )?;

    let window_menu = Submenu::with_items(
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::maximize(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::close_window(None),
        ],
    )?;

    let menu = Menu::with_items(&[&app_menu, &edit_menu, &window_menu])?;
    Ok(menu)
}
