//! quadracalc in a GTK4 window.
//!
//! `quadracalc-gtk [path]` opens `path` (xlsx/ods/csv/…) through
//! GridCraft's `file.open`; with no argument it keeps the built-in
//! sample sheet.

fn main() -> std::process::ExitCode {
    let path = std::env::args().nth(1);
    match quadracalc_core::CalcApp::new_for_path(path.as_deref()) {
        Ok(app) => quadraui::gtk::run(app),
        Err(e) => {
            eprintln!("quadracalc: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
