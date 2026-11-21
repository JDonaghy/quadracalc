//! quadracalc in the terminal: runs locally or over SSH.
//!
//! `quadracalc [path]` opens `path` (xlsx/ods/csv/…) through GridCraft's
//! `file.open`; with no argument it keeps the built-in sample sheet.

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1);
    let app = quadracalc_core::CalcApp::new_for_path(path.as_deref())?;
    quadraui::tui::run(app)?;
    Ok(())
}
