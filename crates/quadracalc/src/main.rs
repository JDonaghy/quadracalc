//! quadracalc in the terminal: runs locally or over SSH.
//!
//! `quadracalc [--mcp] [path]` opens `path` (xlsx/ods/csv/…) through
//! GridCraft's `file.open`; with no path argument it keeps the built-in
//! sample sheet. `--mcp` additionally starts a `quadraui-mcp` server
//! alongside the TUI — see `quadracalc_core::agent`'s module doc for the
//! three app commands it registers and the transport it binds
//! (`$QUADRACALC_MCP`, or a per-process socket/port with no configuration).

use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let args = match quadracalc_core::agent::parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("quadracalc: {e}");
            std::process::exit(2);
        }
    };

    let app = quadracalc_core::CalcApp::new_for_path(args.path.as_deref())?;

    if !args.mcp {
        quadraui::tui::run(app)?;
        return Ok(());
    }

    // Manual runner loop instead of `quadraui::tui::run` — only this
    // path needs the `ControlHandle` `TuiRunner::attach_control` hands
    // back, the same pattern `quadraui`'s own `tui_mcp_demo` example
    // follows.
    let transport = quadracalc_core::agent::mcp_transport();
    eprintln!("quadraui-mcp listening on {transport:?}");
    let mut runner = quadraui::tui::TuiRunner::new(app)?;
    let handle = runner.attach_control();
    quadracalc_core::agent::spawn_mcp_server(handle, transport);

    loop {
        match runner.pump(Duration::from_millis(50))? {
            quadraui::tui::StepOutcome::Continue => {}
            quadraui::tui::StepOutcome::Exited => break,
            _ => {}
        }
    }
    Ok(())
}
