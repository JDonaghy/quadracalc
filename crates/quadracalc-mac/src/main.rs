//! quadracalc in a native macOS window.
//!
//! `quadracalc-mac [--mcp] [path]` opens `path` (xlsx/ods/csv/…) through
//! GridCraft's `file.open`; with no path argument it keeps the built-in
//! sample sheet. `--mcp` additionally starts a `quadraui-mcp` server
//! alongside the window — see `quadracalc_core::agent`'s module doc for
//! the three app commands it registers and the transport it binds.

/// Dock/app-switcher icon, rendered from `assets/app-icon/quadracalc.svg`
/// onto Apple's 824/1024 icon grid — see that directory's README. Embedded
/// rather than loaded from disk so the icon shows up even when the binary
/// is run from outside the repo (the common case once this is a packaged
/// `.app`), mirroring `RunConfig::with_app_icon`'s own `ImageSource::Bytes`
/// contract.
const APP_ICON_PNG: &[u8] = include_bytes!("../../../assets/app-icon/quadracalc-macos-512.png");

#[cfg(target_os = "macos")]
fn run_config() -> quadraui::macos::RunConfig {
    quadraui::macos::RunConfig::new("quadracalc")
        .with_app_icon(quadraui::ImageSource::Bytes(APP_ICON_PNG.to_vec()))
}

#[cfg(target_os = "macos")]
fn main() -> std::process::ExitCode {
    let args = match quadracalc_core::agent::parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("quadracalc: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    let app = match quadracalc_core::CalcApp::new_for_path(args.path.as_deref()) {
        Ok(app) => app,
        Err(e) => {
            eprintln!("quadracalc: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    if !args.mcp {
        return match quadraui::macos::run_with(app, run_config()) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("quadracalc: {e}");
                std::process::ExitCode::FAILURE
            }
        };
    }

    // Manual runner loop instead of `quadraui::macos::run_with` — only
    // this path needs the `ControlHandle` `MacRunner::attach_control`
    // hands back, the same pattern the TUI binary follows (see
    // `quadracalc`'s own `main.rs`) and `quadraui`'s `tui_mcp_demo`
    // example.
    let transport = quadracalc_core::agent::mcp_transport();
    eprintln!("quadraui-mcp listening on {transport:?}");
    let mut runner = quadraui::macos::MacRunner::new_with(app, run_config());
    let handle = runner.attach_control();
    quadracalc_core::agent::spawn_mcp_server(handle, transport);

    loop {
        match runner.pump(std::time::Duration::from_millis(50)) {
            quadraui::macos::StepOutcome::Continue => {}
            quadraui::macos::StepOutcome::Exited => break,
            _ => {}
        }
    }
    std::process::ExitCode::SUCCESS
}

#[cfg(not(target_os = "macos"))]
fn main() -> std::process::ExitCode {
    eprintln!(
        "quadracalc-mac only runs on macOS; use `quadracalc` (terminal) or the build for your platform."
    );
    std::process::ExitCode::FAILURE
}
