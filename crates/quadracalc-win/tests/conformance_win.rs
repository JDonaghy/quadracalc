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
