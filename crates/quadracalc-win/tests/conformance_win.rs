//! Windows runner for the shared `ConformanceDriver` scenarios (issue #27).
//!
//! Each `#[test]` below is a thin wrapper: build a `WinDriver` and hand
//! it to the scenario function `quadracalc-core`'s own
//! `tests/conformance/mod.rs` defines once, generic over any
//! `ConformanceDriver`. `quadracalc-core`'s own `tests/conformance_tui.rs`
//! and `quadracalc-gtk`'s own `tests/conformance_gtk.rs` run the exact
//! same functions against `TuiDriver`/`GtkDriver` — see those files' docs
//! for why the shared module lives in `quadracalc-core` rather than
//! duplicated here.
//!
//! Gated behind this crate's own `win` feature (`Cargo.toml`), not
//! `target_os = "windows"`: `quadraui`'s `win` feature type-checks on
//! every host (every real Direct2D/DirectWrite call inside
//! `quadraui::win` is itself individually `cfg(target_os = "windows")`-
//! gated — see that module's own doc), so this file compiles under
//! `cargo test -p quadracalc-win --features win` on any machine. Only
//! `WinDriver::new()` itself panics off Windows, so actually *running*
//! these tests needs the fleet's Windows host (dell64).
#![cfg(feature = "win")]

#[path = "../../quadracalc-core/tests/conformance/mod.rs"]
mod scenarios;

use quadraui::win::testing::WinDriver;

use quadracalc_core::CalcApp;

// Win-GUI's native unit is the pixel, like GTK's — see
// `quadracalc-gtk`'s own `tests/conformance_gtk.rs` for the sizing
// rationale; these use the same viewport.
const W: u32 = 900;
const H: u32 = 480;

#[test]
fn win_commit_a_formula() {
    let mut driver = WinDriver::new(CalcApp::with_sample().unwrap(), W, H);
    scenarios::commit_a_formula(&mut driver);
}

#[test]
fn win_undo_redo() {
    let mut driver = WinDriver::new(CalcApp::new(), W, H);
    scenarios::undo_redo(&mut driver);
}

#[test]
fn win_copy_cut_paste() {
    let mut driver = WinDriver::new(CalcApp::new(), W, H);
    scenarios::copy_cut_paste(&mut driver);
}

#[test]
fn win_cut_paste() {
    let mut driver = WinDriver::new(CalcApp::new(), W, H);
    scenarios::cut_paste(&mut driver);
}

#[test]
fn win_switch_sheets() {
    let mut driver = WinDriver::new(CalcApp::with_sample().unwrap(), W, H);
    scenarios::switch_sheets(&mut driver);
}

#[test]
fn win_click_a_format_picker_choice() {
    let mut driver = WinDriver::new(CalcApp::new(), W, H);
    scenarios::click_a_format_picker_choice(&mut driver);
}

#[test]
fn win_palette_chord_closes_an_open_menu() {
    let mut driver = WinDriver::new(CalcApp::new(), W, H);
    scenarios::palette_chord_closes_an_open_menu(&mut driver);
}

/// The showcase scenarios run on a larger window: the sample sheet's
/// columns are sized for content, and the find panel and context menu
/// need room beside them.
const WIDE_W: u32 = 1400;
const WIDE_H: u32 = 800;

#[test]
fn win_find_steps_through_matches() {
    let mut driver = WinDriver::new(CalcApp::with_sample().unwrap(), WIDE_W, WIDE_H);
    scenarios::find_steps_through_matches(&mut driver);
}

#[test]
fn win_replace_all_through_the_find_panel() {
    let mut driver = WinDriver::new(CalcApp::new(), WIDE_W, WIDE_H);
    scenarios::replace_all_through_the_find_panel(&mut driver);
}

#[test]
fn win_context_menu_clears_a_cell() {
    let mut driver = WinDriver::new(CalcApp::with_sample().unwrap(), WIDE_W, WIDE_H);
    scenarios::context_menu_clears_a_cell(&mut driver);
}

#[test]
fn win_wheel_scrolls_the_view_not_the_cursor() {
    let mut driver = WinDriver::new(CalcApp::with_sample().unwrap(), WIDE_W, WIDE_H);
    scenarios::wheel_scrolls_the_view_not_the_cursor(&mut driver);
}

#[test]
fn win_double_click_edits_in_place() {
    let mut driver = WinDriver::new(CalcApp::with_sample().unwrap(), WIDE_W, WIDE_H);
    scenarios::double_click_edits_in_place(&mut driver);
}

#[test]
fn win_replace_all_only_touches_counted_cells() {
    let mut driver = WinDriver::new(CalcApp::with_sample().unwrap(), WIDE_W, WIDE_H);
    scenarios::replace_all_only_touches_counted_cells(&mut driver);
}

#[test]
fn win_find_panel_keeps_its_own_pointer_gestures() {
    let mut driver = WinDriver::new(CalcApp::with_sample().unwrap(), WIDE_W, WIDE_H);
    scenarios::find_panel_keeps_its_own_pointer_gestures(&mut driver);
}

#[test]
fn win_shortcuts_work_while_the_find_panel_has_the_keyboard() {
    let mut driver = WinDriver::new(CalcApp::new(), WIDE_W, WIDE_H);
    scenarios::shortcuts_work_while_the_find_panel_has_the_keyboard(&mut driver);
}

#[test]
fn win_ctrl_q_quits_and_asks_first_with_unsaved_changes() {
    let mut driver = WinDriver::new(CalcApp::new(), WIDE_W, WIDE_H);
    scenarios::ctrl_q_quits_and_asks_first_with_unsaved_changes(&mut driver);
}

#[test]
fn win_ctrl_q_quits_a_clean_workbook() {
    let mut driver = WinDriver::new(CalcApp::new(), WIDE_W, WIDE_H);
    scenarios::ctrl_q_quits_a_clean_workbook(&mut driver);
}

#[test]
fn win_menu_quit_asks_first_with_unsaved_changes() {
    let mut driver = WinDriver::new(CalcApp::new(), WIDE_W, WIDE_H);
    scenarios::menu_quit_asks_first_with_unsaved_changes(&mut driver);
}

/// Ignored: `WinDriver::scroll_at` sends `lines × line_height` as the wheel
/// delta, but the real backend reports plain notches, so one
/// `scroll_at` line arrives here as many notches. A quadraui testing gap,
/// not an app one — the TUI and macOS runners do run this scenario.
#[test]
#[ignore = "WinDriver::scroll_at's delta unit differs from the real backend's (quadraui gap)"]
fn win_one_wheel_notch_scrolls_three_rows() {
    let mut driver = WinDriver::new(CalcApp::with_sample().unwrap(), WIDE_W, WIDE_H);
    scenarios::one_wheel_notch_scrolls_three_rows(&mut driver);
}
