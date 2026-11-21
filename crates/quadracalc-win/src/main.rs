//! quadracalc in a native Windows window.
//!
//! `quadracalc-win [path]` opens `path` (xlsx/ods/csv/…) through
//! GridCraft's `file.open`; with no argument it keeps the built-in
//! sample sheet.

#[cfg(target_os = "windows")]
fn main() -> std::process::ExitCode {
    let path = std::env::args().nth(1);
    match quadracalc_core::CalcApp::new_for_path(path.as_deref()) {
        Ok(app) => quadraui::win::run(app),
        Err(e) => {
            eprintln!("quadracalc: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn main() -> std::process::ExitCode {
    eprintln!(
        "quadracalc-win only runs on Windows; use `quadracalc` (terminal) or the build for your platform."
    );
    std::process::ExitCode::FAILURE
}
