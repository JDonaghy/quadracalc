//! quadracalc in a native macOS window.

#[cfg(target_os = "macos")]
fn main() -> std::process::ExitCode {
    match quadracalc_core::CalcApp::with_sample() {
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
