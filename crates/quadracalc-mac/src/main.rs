//! quadracalc in a native macOS window.
//!
//! `quadracalc-mac [--mcp] [path]` opens `path` (xlsx/ods/csv/…) through
//! GridCraft's `file.open`; with no path argument it keeps the built-in
//! sample sheet. `--mcp` additionally starts a `quadraui-mcp` server
//! alongside the window — see `quadracalc_core::agent`'s module doc for
//! the three app commands it registers and the transport it binds.

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
        return match quadraui::macos::run(app) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("quadracalc: {e}");
                std::process::ExitCode::FAILURE
            }
        };
    }

    // Manual runner loop instead of `quadraui::macos::run` — only this
    // path needs the `ControlHandle` `MacRunner::attach_control` hands
    // back, the same pattern the TUI binary follows (see
    // `quadracalc`'s own `main.rs`) and `quadraui`'s `tui_mcp_demo`
    // example.
    let transport = quadracalc_core::agent::mcp_transport();
    eprintln!("quadraui-mcp listening on {transport:?}");
    let mut runner = quadraui::macos::MacRunner::new(app);
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
