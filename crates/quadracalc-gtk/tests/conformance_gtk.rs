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
fn gtk_cut_paste() {
    let mut driver = GtkDriver::new(CalcApp::new(), W, H);
    scenarios::cut_paste(&mut driver);
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

#[test]
fn gtk_menu_save_with_no_file_shows_the_save_error() {
    let mut driver = GtkDriver::new(CalcApp::new(), W, H);
    scenarios::menu_save_with_no_file_shows_the_save_error(&mut driver);
}

#[test]
fn gtk_click_a_format_picker_choice() {
    let mut driver = GtkDriver::new(CalcApp::new(), W, H);
    scenarios::click_a_format_picker_choice(&mut driver);
}

#[test]
fn gtk_palette_chord_closes_an_open_menu() {
    let mut driver = GtkDriver::new(CalcApp::new(), W, H);
    scenarios::palette_chord_closes_an_open_menu(&mut driver);
}

/// The showcase scenarios run on a larger window: the sample sheet's
/// columns are sized for content, and the find panel and context menu
/// need room beside them.
const WIDE_W: i32 = 1400;
const WIDE_H: i32 = 800;

#[test]
fn gtk_find_steps_through_matches() {
    let mut driver = GtkDriver::new(CalcApp::with_sample().unwrap(), WIDE_W, WIDE_H);
    scenarios::find_steps_through_matches(&mut driver);
}

#[test]
fn gtk_replace_all_through_the_find_panel() {
    let mut driver = GtkDriver::new(CalcApp::new(), WIDE_W, WIDE_H);
    scenarios::replace_all_through_the_find_panel(&mut driver);
}

#[test]
fn gtk_context_menu_clears_a_cell() {
    let mut driver = GtkDriver::new(CalcApp::with_sample().unwrap(), WIDE_W, WIDE_H);
    scenarios::context_menu_clears_a_cell(&mut driver);
}

#[test]
fn gtk_wheel_scrolls_the_view_not_the_cursor() {
    let mut driver = GtkDriver::new(CalcApp::with_sample().unwrap(), WIDE_W, WIDE_H);
    scenarios::wheel_scrolls_the_view_not_the_cursor(&mut driver);
}

#[test]
fn gtk_double_click_edits_in_place() {
    let mut driver = GtkDriver::new(CalcApp::with_sample().unwrap(), WIDE_W, WIDE_H);
    scenarios::double_click_edits_in_place(&mut driver);
}

#[test]
fn gtk_replace_all_only_touches_counted_cells() {
    let mut driver = GtkDriver::new(CalcApp::with_sample().unwrap(), WIDE_W, WIDE_H);
    scenarios::replace_all_only_touches_counted_cells(&mut driver);
}

#[test]
fn gtk_find_panel_keeps_its_own_pointer_gestures() {
    let mut driver = GtkDriver::new(CalcApp::with_sample().unwrap(), WIDE_W, WIDE_H);
    scenarios::find_panel_keeps_its_own_pointer_gestures(&mut driver);
}

#[test]
fn gtk_shortcuts_work_while_the_find_panel_has_the_keyboard() {
    let mut driver = GtkDriver::new(CalcApp::new(), WIDE_W, WIDE_H);
    scenarios::shortcuts_work_while_the_find_panel_has_the_keyboard(&mut driver);
}

#[test]
fn gtk_ctrl_q_quits_and_asks_first_with_unsaved_changes() {
    let mut driver = GtkDriver::new(CalcApp::new(), WIDE_W, WIDE_H);
    scenarios::ctrl_q_quits_and_asks_first_with_unsaved_changes(&mut driver);
}

#[test]
fn gtk_ctrl_q_quits_a_clean_workbook() {
    let mut driver = GtkDriver::new(CalcApp::new(), WIDE_W, WIDE_H);
    scenarios::ctrl_q_quits_a_clean_workbook(&mut driver);
}

#[test]
fn gtk_menu_quit_asks_first_with_unsaved_changes() {
    let mut driver = GtkDriver::new(CalcApp::new(), WIDE_W, WIDE_H);
    scenarios::menu_quit_asks_first_with_unsaved_changes(&mut driver);
}

/// Ignored: `GtkDriver::scroll_at` sends `lines × line_height` as the wheel
/// delta, but the real backend reports plain notches, so one
/// `scroll_at` line arrives here as many notches. A quadraui testing gap,
/// not an app one — the TUI and macOS runners do run this scenario.
#[test]
#[ignore = "GtkDriver::scroll_at's delta unit differs from the real backend's (quadraui gap)"]
fn gtk_one_wheel_notch_scrolls_three_rows() {
    let mut driver = GtkDriver::new(CalcApp::with_sample().unwrap(), WIDE_W, WIDE_H);
    scenarios::one_wheel_notch_scrolls_three_rows(&mut driver);
}
