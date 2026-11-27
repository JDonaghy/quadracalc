//! TUI runner for the shared `ConformanceDriver` scenarios (issue #27).
//!
//! Each `#[test]` below is a thin wrapper: build a `TuiDriver` and hand it
//! to the scenario function `tests/conformance/mod.rs` defines once,
//! generic over any `ConformanceDriver`. `quadracalc-gtk`'s
//! `tests/conformance_gtk.rs` runs the exact same functions against
//! `GtkDriver`, and `quadracalc-mac`'s `tests/conformance_mac.rs` runs
//! them against `MacDriver` on a macOS host.
//!
//! This is additive to, not a replacement for, the existing per-file
//! `TuiDriver` tests in `tests/editing.rs`/`clipboard.rs`/`sheets.rs`:
//! those stay as they are where they assert on something TUI-specific
//! (character-grid string matching, tab-index coordinates). These four
//! cover the same flows through the backend-neutral surface instead.

#[path = "conformance/mod.rs"]
mod scenarios;

use quadraui::tui::testing::TuiDriver;

use quadracalc_core::CalcApp;

#[test]
fn tui_commit_a_formula() {
    let mut driver = TuiDriver::new(CalcApp::with_sample().unwrap(), 80, 20);
    scenarios::commit_a_formula(&mut driver);
}

#[test]
fn tui_undo_redo() {
    let mut driver = TuiDriver::new(CalcApp::new(), 80, 20);
    scenarios::undo_redo(&mut driver);
}

#[test]
fn tui_copy_cut_paste() {
    let mut driver = TuiDriver::new(CalcApp::new(), 80, 20);
    scenarios::copy_cut_paste(&mut driver);
}

#[test]
fn tui_cut_paste() {
    let mut driver = TuiDriver::new(CalcApp::new(), 80, 20);
    scenarios::cut_paste(&mut driver);
}

#[test]
fn tui_switch_sheets() {
    let mut driver = TuiDriver::new(CalcApp::with_sample().unwrap(), 80, 24);
    scenarios::switch_sheets(&mut driver);
}

#[test]
fn tui_open_menu_and_quit() {
    let mut driver = TuiDriver::new(CalcApp::new(), 80, 20);
    scenarios::open_menu_and_quit(&mut driver);
}

#[test]
fn tui_open_formulas_menu_opens_function_picker() {
    let mut driver = TuiDriver::new(CalcApp::new(), 80, 20);
    scenarios::open_formulas_menu_opens_function_picker(&mut driver);
}

#[test]
fn tui_home_menu_has_grouped_submenus_with_shortcut_hints() {
    let mut driver = TuiDriver::new(CalcApp::new(), 80, 20);
    scenarios::home_menu_has_grouped_submenus_with_shortcut_hints(&mut driver);
}

#[test]
fn tui_menu_save_with_no_file_shows_the_save_error() {
    let mut driver = TuiDriver::new(CalcApp::new(), 80, 20);
    scenarios::menu_save_with_no_file_shows_the_save_error(&mut driver);
}

#[test]
fn tui_click_a_format_picker_choice() {
    let mut driver = TuiDriver::new(CalcApp::new(), 80, 24);
    scenarios::click_a_format_picker_choice(&mut driver);
}

#[test]
fn tui_palette_chord_closes_an_open_menu() {
    let mut driver = TuiDriver::new(CalcApp::new(), 80, 24);
    scenarios::palette_chord_closes_an_open_menu(&mut driver);
}
