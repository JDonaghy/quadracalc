//! quadracalc in a native macOS window.
//!
//! `quadracalc-mac [path]` opens `path` (xlsx/ods/csv/…) through
//! GridCraft's `file.open`; with no argument it keeps the built-in
//! sample sheet.

#[cfg(target_os = "macos")]
fn main() -> std::process::ExitCode {
    let path = std::env::args().nth(1);
    match quadracalc_core::CalcApp::new_for_path(path.as_deref()) {
        Ok(app) => quadraui::macos::run(app),
        Err(e) => {
            eprintln!("quadracalc: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() -> std::process::ExitCode {
    eprintln!(
        "quadracalc-mac only runs on macOS; use `quadracalc` (terminal) or the build for your platform."
    );
    std::process::ExitCode::FAILURE
}
