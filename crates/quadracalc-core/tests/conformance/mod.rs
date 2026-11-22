//! Shared `ConformanceDriver` scenarios (issue #27).
//!
//! Each function below is written once, generic over any
//! `D: quadraui::testing::ConformanceDriver`, and runs unmodified against
//! every backend driver that implements the trait: `tests/conformance_tui.rs`
//! in this crate runs them against `TuiDriver`, `tests/conformance_gtk.rs`
//! in `quadracalc-gtk` runs the exact same functions against `GtkDriver`,
//! and `tests/conformance_mac.rs` in `quadracalc-mac` runs them against
//! `MacDriver` (that last one only actually builds on a macOS host —
//! see that file's own doc for why).
//!
//! Per `quadraui`'s own shared-body rules (`docs/TESTING.md`, the
//! `quadraui::testing` module doc): locate targets with `click_text`/
//! `type_text`/`press_named`, never a literal `click(x, y)` — TUI cells
//! and GTK pixels are different units, so a hardcoded coordinate here
//! would silently be wrong on one side. Assert on `screen_has`, never on
//! pixel geometry.
//!
//! This file is included via `#[path = "conformance/mod.rs"]` rather than
//! living directly under `tests/`, so Cargo never treats it as its own
//! integration-test binary (it has no `#[test]`s of its own) — only the
//! two thin runner files that `mod` it do.
//!
//! [`open_menu_and_quit`] is the menu-bar scenario — added once the menu
//! itself landed; issue #27's own review correctly found no menu bar to
//! drive yet when this file was first written.

use quadraui::testing::{ConformanceDriver, DriverInput};
use quadraui::{Key, Modifiers, NamedKey, UiEvent};

/// Commit a formula through the real in-cell editor and see its dependent
/// recalculate on screen in the same frame — `tests/editing.rs`'s own
/// `tui_driver_types_a_formula_and_commits_the_recalculated_value_and_formula_bar`,
/// lifted to run against any backend. Needs `CalcApp::with_sample()` as
/// its fixture: B3 starts at "9" and B4 is `=SUM(B2:B3)`.
pub fn commit_a_formula<D: ConformanceDriver>(d: &mut D) {
    d.press_named(NamedKey::Right);
    d.press_named(NamedKey::Down);
    d.press_named(NamedKey::Down);
    d.type_text("20");
    assert!(
        d.screen_has("20"),
        "the in-cell editor should paint the in-progress text"
    );

    d.press_named(NamedKey::Enter);
    assert!(
        d.screen_has("1220.5") || d.screen_has("1,220.50"),
        "B4's dependent total should recalculate on screen in the same frame the edit commits"
    );
    assert!(
        d.screen_has("=SUM(B2:B3)"),
        "the formula bar should show B4's raw formula once the cursor lands there"
    );
}

/// Ctrl+Z/Ctrl+Y restore a committed cell's text on screen —
/// `tests/editing.rs`'s own `ctrl_z_and_ctrl_y_restore_both_the_value_and_the_formula_bar`,
/// lifted to run against any backend. Needs a blank `CalcApp::new()` as
/// its fixture.
pub fn undo_redo<D: ConformanceDriver>(d: &mut D) {
    d.type_text("hello");
    d.press_named(NamedKey::Enter);
    assert!(
        d.screen_has("hello"),
        "committing text should paint it on screen"
    );

    d.ctrl_char('z');
    assert!(
        !d.screen_has("hello"),
        "undo should remove the committed text from screen"
    );

    d.ctrl_char('y');
    assert!(
        d.screen_has("hello"),
        "redo should restore the committed text"
    );
}

/// Copy a formula, move the selection, and paste — the formula's relative
/// reference should shift, visible in the formula bar once the cursor
/// lands on the pasted cell. `tests/clipboard.rs`'s own
/// `ctrl_v_through_dispatch_pastes_a_copied_formula_with_adjusted_references`,
/// lifted to run against any backend. Needs a blank `CalcApp::new()` as
/// its fixture.
///
/// `Ctrl+C` exercises the real copy path (`CellGridController` arms the
/// engine's own clipboard and writes the OS clipboard, discarded here —
/// a headless test backend has no display to round-trip through). The
/// synthetic `ClipboardPaste` dispatch stands in for "the OS clipboard
/// round-tripped it", the same stand-in `tests/clipboard.rs`'s own
/// black-box test uses; its payload only matters when nothing was
/// already armed, so the literal text doesn't need to be read back off
/// the app.
pub fn copy_cut_paste<D: ConformanceDriver + DriverInput>(d: &mut D) {
    // `press_named`/`ctrl_char` are declared on both `ConformanceDriver`
    // and `DriverInput` (the latter's defaults are what every concrete
    // driver's own inherent method — and so `ConformanceDriver`'s impl —
    // forwards to), so a bare `d.press_named(..)` is ambiguous once both
    // traits are in scope together. Fully qualify to `ConformanceDriver`'s
    // copy throughout this function for that reason alone.
    d.type_text("10");
    ConformanceDriver::press_named(d, NamedKey::Enter); // commits A1=10, moves to A2
    ConformanceDriver::press_named(d, NamedKey::Up); // back to A1
    ConformanceDriver::press_named(d, NamedKey::Right); // to B1
    d.type_text("=A1*2");
    ConformanceDriver::press_named(d, NamedKey::Enter); // commits B1=20, moves to B2
    ConformanceDriver::press_named(d, NamedKey::Up); // back to B1
    assert!(
        d.screen_has("20"),
        "B1 should show its computed value before copying"
    );

    ConformanceDriver::ctrl_char(d, 'c');
    for _ in 0..4 {
        ConformanceDriver::press_named(d, NamedKey::Down); // B1 -> B5
    }
    d.dispatch(UiEvent::ClipboardPaste("20".to_string()));

    assert!(
        d.screen_has("=A5*2"),
        "pasting four rows down should shift the relative reference, \
         visible in the formula bar once the cursor lands on B5"
    );
}

/// Insert a sheet, confirm the view switches to it, then click back to
/// the first sheet's tab by its painted label — `tests/sheets.rs`'s own
/// `tui_driver_clicking_a_tab_switches_the_visible_sheet`, lifted to run
/// against any backend via `click_text` instead of a tab-index lookup.
/// Needs `CalcApp::with_sample()` as its fixture: "Sheet1", with "Item"
/// visible in its data.
///
/// Opens the palette with Ctrl+Shift+P rather than `tests/sheets.rs`'s
/// own bare `:` — `CalcApp::is_tui_shaped`'s own doc notes `:` only opens
/// the palette on a TUI-shaped backend; on a pixel backend (GTK) it's
/// just a literal colon typed into the cell. Ctrl+Shift+P is the one
/// shortcut both shapes honor, so it's the backend-neutral choice here.
pub fn switch_sheets<D: ConformanceDriver + DriverInput>(d: &mut D) {
    assert!(
        d.screen_has("Item"),
        "the sample sheet's data should be visible"
    );
    assert!(
        d.screen_has("Sheet1"),
        "the first sheet's tab should be visible"
    );

    d.dispatch(UiEvent::KeyPressed {
        key: Key::Char('p'),
        modifiers: Modifiers {
            ctrl: true,
            shift: true,
            ..Modifiers::default()
        },
        repeat: false,
    });
    d.type_text("insert sheet home");
    ConformanceDriver::press_named(d, NamedKey::Enter);
    assert!(
        !d.screen_has("Item"),
        "the newly inserted sheet should be empty, not showing the first sheet's data"
    );

    d.click_text("Sheet1");
    assert!(
        d.screen_has("Item"),
        "clicking the first sheet's tab should switch the visible sheet back to it"
    );
}

/// Open the File menu by clicking its label, confirm the dropdown's real
/// catalog items are on screen, then click Quit. Needs a blank
/// `CalcApp::new()` as its fixture. Only `ConformanceDriver` itself —
/// `click_text`/`screen_has`/`exited` are all declared there, none on
/// `DriverInput`, so no ambiguity to fully-qualify against the way
/// `copy_cut_paste`/`switch_sheets` above have to.
pub fn open_menu_and_quit<D: ConformanceDriver>(d: &mut D) {
    assert!(
        d.screen_has("File"),
        "the menu bar's own top-level labels should be visible before anything is clicked"
    );

    d.click_text("File");
    assert!(
        d.screen_has("Quit"),
        "clicking File should open its dropdown, with the Quit item it has no catalog \
         counterpart for (see crate::menu's own doc) visible on screen"
    );
    assert!(
        d.screen_has("Save"),
        "the dropdown should also show a real catalog item, not just the appended Quit"
    );

    d.click_text("Quit");
    assert!(d.exited(), "activating Quit should return Reaction::Exit");
}

/// Open the Formulas menu and click "Financial" — one of the
/// `function_picker::has_picker` ids the menu's own `Activated` dispatch
/// has to special-case the same way it already does for
/// `format_picker::has_picker`, parallel to [`open_menu_and_quit`]'s
/// File-tab coverage of the generic/Quit branches. Needs a blank
/// `CalcApp::new()` as its fixture.
///
/// "Financial" rather than the Function Library group's own first row
/// ("Insert Function") deliberately: that row's dropdown position is
/// right at the menu bar's own row, and on a backend whose dropdown
/// doesn't fit the fixture's viewport without shifting (`GtkDriver`'s
/// 480px-tall one, this tab's dropdown has ~20 rows across three
/// groups) a click there can land back on the tab button instead of the
/// item — not what this scenario means to exercise. Every
/// `function_picker::has_picker` id (this one included) routes through
/// the exact same dispatch branch, so any one of them proves the fix.
pub fn open_formulas_menu_opens_function_picker<D: ConformanceDriver>(d: &mut D) {
    assert!(
        d.screen_has("Formulas"),
        "the menu bar's own top-level labels should be visible before anything is clicked"
    );

    d.click_text("Formulas");
    assert!(
        d.screen_has("Financial"),
        "clicking Formulas should open its dropdown, with the real catalog item \
         \"Financial\" visible on screen"
    );

    d.click_text("Financial");
    assert!(
        d.screen_has("Insert Function"),
        "activating the id should open the function picker overlay, titled \
         \"Insert Function\" the same as every category id opens it — not fall through \
         to the generic `execute` arm, which discards the query result and shows \
         nothing at all. This title alone rules out the generic arm's own failure mode: \
         it never shows up from a stray keystroke landing in a cell instead"
    );
    // "ACCRINT" sorts first alphabetically among `formulas.functions`'
    // own Financial-category rows, so it's on screen without typing or
    // scrolling — a real catalog row, not the title alone.
    assert!(
        d.screen_has("ACCRINT"),
        "the Financial category should list its own real functions"
    );
}
