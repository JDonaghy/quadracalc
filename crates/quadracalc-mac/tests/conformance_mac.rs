//! macOS runner for the shared `ConformanceDriver` scenarios (issue #27).
//!
//! Each `#[test]` below is a thin wrapper: build a `MacDriver` and hand
//! it to the scenario function `quadracalc-core`'s own
//! `tests/conformance/mod.rs` defines once, generic over any
//! `ConformanceDriver`. `quadracalc-core`'s own `tests/conformance_tui.rs`
//! runs the exact same functions against `TuiDriver`, and
//! `quadracalc-gtk`'s `tests/conformance_gtk.rs` runs them against
//! `GtkDriver` — see those files' docs for why the shared module lives
//! there rather than duplicated here.
//!
//! `MacDriver` only exists when `quadraui`'s `macos` module compiles,
//! which needs both the `macos` feature *and* `target_os = "macos"`
//! (quadraui's own `src/lib.rs` gates `mod macos` on the target, not
//! just the feature — a `cargo check --features macos` on Linux
//! compiles none of it). `quadracalc-mac`'s own `Cargo.toml` already
//! only pulls in `quadraui/macos` under that same target gate, so this
//! whole file cfg's out identically on any non-macOS host; it only
//! actually runs on a macOS runner (`.github/workflows/macos.yml`'s
//! twin for this repo, or a developer's own Mac).
#![cfg(target_os = "macos")]

#[path = "../../quadracalc-core/tests/conformance/mod.rs"]
mod scenarios;

use quadraui::macos::testing::MacDriver;

use quadracalc_core::CalcApp;

/// macOS's native unit is the point (≈pixel at 1.0 backing scale), not
/// the TUI cell — these are sized to comfortably fit the same `CalcApp`
/// grid the TUI runner's 80×20/80×24 cell viewports do, at the same
/// nominal 8pt/16pt char metrics `MacDriver::new_fixture` scales by
/// (mirrors `quadracalc-gtk`'s own `conformance_gtk.rs` W/H constants).
const W: u32 = 900;
const H: u32 = 480;

#[test]
fn mac_commit_a_formula() {
    let mut driver = MacDriver::new(CalcApp::with_sample().unwrap(), W, H);
    scenarios::commit_a_formula(&mut driver);
}

#[test]
fn mac_undo_redo() {
    let mut driver = MacDriver::new(CalcApp::new(), W, H);
    scenarios::undo_redo(&mut driver);
}

#[test]
fn mac_copy_cut_paste() {
    let mut driver = MacDriver::new(CalcApp::new(), W, H);
    scenarios::copy_cut_paste(&mut driver);
}

#[test]
fn mac_switch_sheets() {
    let mut driver = MacDriver::new(CalcApp::with_sample().unwrap(), W, H);
    scenarios::switch_sheets(&mut driver);
}

#[test]
fn mac_open_menu_and_quit() {
    let mut driver = MacDriver::new(CalcApp::new(), W, H);
    scenarios::open_menu_and_quit(&mut driver);
}

#[test]
fn mac_open_formulas_menu_opens_function_picker() {
    let mut driver = MacDriver::new(CalcApp::new(), W, H);
    scenarios::open_formulas_menu_opens_function_picker(&mut driver);
}
