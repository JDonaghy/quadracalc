//! quadracalc in a GTK4 window.
//!
//! `quadracalc-gtk [--mcp] [path]` opens `path` (xlsx/ods/csv/…) through
//! GridCraft's `file.open`; with no path argument it keeps the built-in
//! sample sheet. `--mcp` additionally starts a `quadraui-mcp` server
//! alongside the window — see `quadracalc_core::agent`'s module doc for
//! the three app commands it registers and the transport it binds.

use std::time::Duration;

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
        return quadraui::gtk::run(app);
    }

    // Manual runner loop instead of `quadraui::gtk::run` — only this
    // path needs the `ControlHandle` `GtkRunner::attach_control` hands
    // back, the same pattern the TUI and macOS binaries follow (see
    // `quadracalc`'s own `main.rs`) and `quadraui`'s `tui_mcp_demo`
    // example. GTK's control port is serviced automatically on every
    // GLib main-context iteration, so this loop only needs to keep
    // pumping — it doesn't have to drain anything itself.
    let transport = quadracalc_core::agent::mcp_transport();
    eprintln!("quadraui-mcp listening on {transport:?}");
    let mut runner = quadraui::gtk::GtkRunner::new(app);
    let handle = runner.attach_control();
    quadracalc_core::agent::spawn_mcp_server(handle, transport);

    loop {
        match runner.pump(Duration::from_millis(50)) {
            quadraui::gtk::StepOutcome::Continue => {}
            quadraui::gtk::StepOutcome::Exited => break,
            _ => {}
        }
    }
    if runner.smoke_failed() {
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}
