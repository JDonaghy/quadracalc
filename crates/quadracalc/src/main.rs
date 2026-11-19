//! quadracalc in the terminal: runs locally or over SSH.

fn main() -> anyhow::Result<()> {
    let app = quadracalc_core::CalcApp::with_sample()?;
    quadraui::tui::run(app)?;
    Ok(())
}
