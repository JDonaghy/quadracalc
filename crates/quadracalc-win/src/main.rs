//! quadracalc in a native Windows window.

#[cfg(target_os = "windows")]
fn main() -> std::process::ExitCode {
    match quadracalc_core::CalcApp::with_sample() {
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
