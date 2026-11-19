//! quadracalc in a GTK4 window.

fn main() -> std::process::ExitCode {
    match quadracalc_core::CalcApp::with_sample() {
        Ok(app) => quadraui::gtk::run(app),
        Err(e) => {
            eprintln!("quadracalc: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
