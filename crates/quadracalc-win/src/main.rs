//! quadracalc in a native Windows window.
//!
//! `quadracalc-win [--mcp] [path]` opens `path` (xlsx/ods/csv/…) through
//! GridCraft's `file.open`; with no path argument it keeps the built-in
//! sample sheet. `--mcp` additionally starts a `quadraui-mcp` server
//! alongside the window — see `quadracalc_core::agent`'s module doc for
//! the three app commands it registers and the transport it binds.

#[cfg(target_os = "windows")]
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
        return quadraui::win::run(app);
    }

    // Manual runner loop instead of `quadraui::win::run` — only this
    // path needs the `ControlHandle` `WinRunner::attach_control` hands
    // back, the same pattern the TUI, GTK and macOS binaries follow
    // (see `quadracalc`'s own `main.rs`) and `quadraui`'s `tui_mcp_demo`
    // example.
    let transport = quadracalc_core::agent::mcp_transport();
    eprintln!("quadraui-mcp listening on {transport:?}");
    // SAFETY: this thread is the process's single GUI/message-pump
    // thread — `main` runs on it and steps `runner` from nowhere else.
    let mut runner = match unsafe { quadraui::win::WinRunner::new(app) } {
        Ok(runner) => runner,
        Err(e) => {
            eprintln!("quadracalc: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let handle = runner.attach_control();
    quadracalc_core::agent::spawn_mcp_server(handle, transport);

    loop {
        match runner.pump(std::time::Duration::from_millis(50)) {
            Ok(quadraui::win::StepOutcome::Continue) => {}
            Ok(quadraui::win::StepOutcome::Exited) => break,
            Ok(_) => {}
            Err(e) => {
                eprintln!("quadracalc: {e}");
                return std::process::ExitCode::FAILURE;
            }
        }
    }
    std::process::ExitCode::SUCCESS
}

#[cfg(not(target_os = "windows"))]
fn main() -> std::process::ExitCode {
    eprintln!(
        "quadracalc-win only runs on Windows; use `quadracalc` (terminal) or the build for your platform."
    );
    std::process::ExitCode::FAILURE
}
