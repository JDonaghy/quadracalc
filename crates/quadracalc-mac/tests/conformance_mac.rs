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
fn mac_cut_paste() {
    let mut driver = MacDriver::new(CalcApp::new(), W, H);
    scenarios::cut_paste(&mut driver);
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

#[test]
fn mac_home_menu_has_grouped_submenus_with_shortcut_hints() {
    let mut driver = MacDriver::new(CalcApp::new(), W, H);
    scenarios::home_menu_has_grouped_submenus_with_shortcut_hints(&mut driver);
}

#[test]
fn mac_menu_save_with_no_file_shows_the_save_error() {
    let mut driver = MacDriver::new(CalcApp::new(), W, H);
    scenarios::menu_save_with_no_file_shows_the_save_error(&mut driver);
}

#[test]
fn mac_click_a_format_picker_choice() {
    let mut driver = MacDriver::new(CalcApp::new(), W, H);
    scenarios::click_a_format_picker_choice(&mut driver);
}

#[test]
fn mac_palette_chord_closes_an_open_menu() {
    let mut driver = MacDriver::new(CalcApp::new(), W, H);
    scenarios::palette_chord_closes_an_open_menu(&mut driver);
}

/// The showcase scenarios run on a larger window: the sample sheet's
/// columns are sized for content, and the find panel and context menu
/// need room beside them.
const WIDE_W: u32 = 1400;
const WIDE_H: u32 = 800;

#[test]
fn mac_find_steps_through_matches() {
    let mut driver = MacDriver::new(CalcApp::with_sample().unwrap(), WIDE_W, WIDE_H);
    scenarios::find_steps_through_matches(&mut driver);
}

#[test]
fn mac_replace_all_through_the_find_panel() {
    let mut driver = MacDriver::new(CalcApp::new(), WIDE_W, WIDE_H);
    scenarios::replace_all_through_the_find_panel(&mut driver);
}

#[test]
fn mac_context_menu_clears_a_cell() {
    let mut driver = MacDriver::new(CalcApp::with_sample().unwrap(), WIDE_W, WIDE_H);
    scenarios::context_menu_clears_a_cell(&mut driver);
}

#[test]
fn mac_wheel_scrolls_under_a_frozen_header() {
    let mut driver = MacDriver::new(CalcApp::with_sample().unwrap(), WIDE_W, WIDE_H);
    scenarios::wheel_scrolls_under_a_frozen_header(&mut driver);
}

#[test]
fn mac_double_click_edits_in_place() {
    let mut driver = MacDriver::new(CalcApp::with_sample().unwrap(), WIDE_W, WIDE_H);
    scenarios::double_click_edits_in_place(&mut driver);
}
