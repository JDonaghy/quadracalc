//! GTK runner for the shared `ConformanceDriver` scenarios (issue #27).
//!
//! Each `#[test]` below is a thin wrapper: build a `GtkDriver` and hand
//! it to the scenario function `quadracalc-core`'s own
//! `tests/conformance/mod.rs` defines once, generic over any
//! `ConformanceDriver`. `quadracalc-core`'s own `tests/conformance_tui.rs`
//! runs the exact same functions against `TuiDriver` — see that file's
//! doc for why the shared module lives there rather than duplicated here.

#[path = "../../quadracalc-core/tests/conformance/mod.rs"]
mod scenarios;

use quadraui::gtk::testing::GtkDriver;

use quadracalc_core::CalcApp;

/// GTK's native unit is the pixel, not the TUI cell — these are sized to
/// comfortably fit the same `CalcApp` grid the TUI runner's 80×20/80×24
/// cell viewports do, at the backend's own nominal 8px/16px char
/// metrics (`GtkDriver::new_fixture`'s own doc spells out the scaling).
const W: i32 = 900;
const H: i32 = 480;

#[test]
fn gtk_commit_a_formula() {
    let mut driver = GtkDriver::new(CalcApp::with_sample().unwrap(), W, H);
    scenarios::commit_a_formula(&mut driver);
}

#[test]
fn gtk_undo_redo() {
    let mut driver = GtkDriver::new(CalcApp::new(), W, H);
    scenarios::undo_redo(&mut driver);
}

#[test]
fn gtk_copy_cut_paste() {
    let mut driver = GtkDriver::new(CalcApp::new(), W, H);
    scenarios::copy_cut_paste(&mut driver);
}

#[test]
fn gtk_switch_sheets() {
    let mut driver = GtkDriver::new(CalcApp::with_sample().unwrap(), W, H);
    scenarios::switch_sheets(&mut driver);
}

#[test]
fn gtk_open_menu_and_quit() {
    let mut driver = GtkDriver::new(CalcApp::new(), W, H);
    scenarios::open_menu_and_quit(&mut driver);
}

#[test]
fn gtk_open_formulas_menu_opens_function_picker() {
    let mut driver = GtkDriver::new(CalcApp::new(), W, H);
    scenarios::open_formulas_menu_opens_function_picker(&mut driver);
}

#[test]
fn gtk_home_menu_has_grouped_submenus_with_shortcut_hints() {
    let mut driver = GtkDriver::new(CalcApp::new(), W, H);
    scenarios::home_menu_has_grouped_submenus_with_shortcut_hints(&mut driver);
}
