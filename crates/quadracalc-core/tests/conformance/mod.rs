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
/// `Ctrl+C` exercises the real copy path (`CalcApp::handle`'s own
/// shortcut dispatch arms the engine's own clipboard and writes the OS
/// clipboard, discarded here — a headless test backend has no display
/// to round-trip through). The
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

/// Cut a cell, paste it elsewhere, and confirm the source is cleared —
/// the cut leg `copy_cut_paste` never exercised (issue #50; that
/// scenario's own doc above only covers copy). `tests/clipboard.rs`'s own
/// `cut_clears_the_source_and_keeps_the_formula_unshifted_at_the_destination`
/// drives the same engine behaviour directly against `CalcApp::execute`;
/// this is its black-box counterpart through the real Ctrl+X keyboard
/// path and a synthetic `ClipboardPaste` dispatch, the same stand-in
/// `copy_cut_paste` uses for "the OS clipboard round-tripped it". Needs a
/// blank `CalcApp::new()` as its fixture.
///
/// Excel semantics: `edit.cut` only arms the engine's clipboard — the
/// source isn't cleared until the paste lands elsewhere (see
/// `crate::clipboard::cut`'s own doc), so the source must still be
/// readable right after Ctrl+X and only disappear once pasted.
///
/// The destination keeps the source's own screen text (cut doesn't
/// adjust it the way copy adjusts a relative reference), so `screen_has`
/// alone can't tell "source cleared" from "source still there, counted
/// twice" — both the destination's grid cell and, were the source still
/// selected, its own formula-bar echo would contain the same text. The
/// scenario instead moves the selection *back* to the source after
/// pasting and asserts `count` is exactly one: the source cell and its
/// own formula-bar echo both go silent once cleared, leaving only the
/// (now unselected) destination's grid cell behind.
pub fn cut_paste<D: ConformanceDriver + DriverInput>(d: &mut D) {
    d.type_text("77");
    ConformanceDriver::press_named(d, NamedKey::Enter); // commits A1=77, moves to A2
    ConformanceDriver::press_named(d, NamedKey::Up); // back to A1
    assert!(
        d.screen_has("77"),
        "A1 should show its value before cutting"
    );

    ConformanceDriver::ctrl_char(d, 'x');
    assert!(
        d.screen_has("77"),
        "Ctrl+X only arms the clipboard — the source isn't cleared until the paste lands"
    );

    for _ in 0..4 {
        ConformanceDriver::press_named(d, NamedKey::Down); // A1 -> A5
    }
    d.dispatch(UiEvent::ClipboardPaste("77".to_string()));
    assert!(
        d.screen_has("77"),
        "pasting the cut value should land it at the destination (A5)"
    );

    for _ in 0..4 {
        ConformanceDriver::press_named(d, NamedKey::Up); // A5 -> A1, the source
    }
    assert_eq!(
        d.inventory().count("77"),
        1,
        "once pasted, the source cell (now selected, so with no formula-bar echo of its \
         own either) should be the only place \"77\" went quiet — the one remaining \
         occurrence is the destination's grid cell"
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

/// Home's ~90 catalog placements are grouped into one
/// `ContextMenuItem::submenu` per `Entry::group` instead of one long flat
/// dropdown — open Home, confirm a group label (not one of its nested
/// items) is what's on screen first, open that group's submenu and
/// confirm a nested item is now visible, then switch to the Clipboard
/// group and confirm its accelerator hint renders. `AppMenu`'s submenu
/// state machine is backend-neutral (same as [`open_menu_and_quit`]'s
/// dispatch), so this runs against every driver, not just `TuiDriver`.
/// Needs a blank `CalcApp::new()` as its fixture.
pub fn home_menu_has_grouped_submenus_with_shortcut_hints<D: ConformanceDriver>(d: &mut D) {
    assert!(
        d.screen_has("Home"),
        "the menu bar's own top-level labels should be visible before anything is clicked"
    );

    d.click_text("Home");
    assert!(
        d.screen_has("Font"),
        "clicking Home should open its dropdown, with the \"Font\" group label visible"
    );
    assert!(
        !d.screen_has("Bold"),
        "\"Bold\" lives inside the Font submenu, not flat in the Home dropdown — it \
         shouldn't be on screen before that submenu is opened"
    );

    d.click_text("Font");
    assert!(
        d.screen_has("Bold"),
        "clicking the \"Font\" group should open its submenu, with \"Bold\" (a real \
         catalog item) now visible"
    );

    // Switching to another top-level group closes Font's submenu and
    // opens Clipboard's — "Copy" is one of the few Home ids with a
    // working keyboard shortcut (`shortcut_for` in `crate::menu`), so its
    // row should show the rendered accelerator hint alongside the label.
    d.click_text("Clipboard");
    assert!(
        d.screen_has("Copy"),
        "clicking the \"Clipboard\" group should open its own submenu"
    );
    // `KeyBinding::Copy` renders platform-appropriately: `⌘C` on macOS,
    // `Ctrl+C` everywhere else.
    assert!(
        d.screen_has("Ctrl+C") || d.screen_has("⌘C"),
        "\"Copy\" should show its Copy key_equivalent hint (Ctrl+C / ⌘C) in the dropdown"
    );
}

/// Clicking the menu's "Save" item on a workbook with no backing file
/// should surface `CalcApp::save`'s own "nothing to save to yet" error,
/// not silently succeed. `AppMenuEvent::Activated`'s dispatch special-cases
/// `file.save` to call `CalcApp::save` the same way Ctrl+S's keyboard
/// dispatch does; a bare `execute("file.save", {})` instead would hit
/// the engine's `UiRequest::Dialog("saveAs", ...)` fallback, return `Ok`,
/// and leave the status bar showing no error at all — this is the
/// regression check for that. Needs a blank `CalcApp::new()` as its
/// fixture (never opened from or saved to a file).
pub fn menu_save_with_no_file_shows_the_save_error<D: ConformanceDriver>(d: &mut D) {
    d.click_text("File");
    assert!(
        d.screen_has("Save"),
        "the File dropdown should show a real \"Save\" catalog item"
    );

    d.click_text("Save");
    assert!(
        d.screen_has("nothing to save"),
        "activating Save through the menu with no backing file should show \
         `CalcApp::save`'s own error in the status bar, not succeed silently"
    );
}

/// Ctrl+Shift+P — the palette chord with the same modifiers on every
/// backend (`CalcApp::handle` accepts Ctrl or Cmd).
fn ctrl_shift_p() -> UiEvent {
    UiEvent::KeyPressed {
        key: Key::Char('P'),
        modifiers: Modifiers {
            ctrl: true,
            shift: true,
            ..Modifiers::default()
        },
        repeat: false,
    }
}

/// Open the Number Format picker from the palette and *click* one of
/// its choices — the picker's mouse path, which hit-tests the click
/// against `quadraui::MessageDialog`'s own layout of the dialog it
/// painted. The keyboard path has TUI coverage in `tests/format_cells.rs`;
/// the click has to land on the right button on a pixel backend too,
/// where button widths come from real text measurement. Needs a blank
/// `CalcApp::new()` as its fixture.
pub fn click_a_format_picker_choice<D: ConformanceDriver + DriverInput>(d: &mut D) {
    d.type_text("1234");
    ConformanceDriver::press_named(d, NamedKey::Enter); // commits A1, moves to A2
    ConformanceDriver::press_named(d, NamedKey::Up); // back to A1

    d.dispatch(ctrl_shift_p());
    d.type_text("number format");
    ConformanceDriver::press_named(d, NamedKey::Enter);
    assert!(
        d.screen_has("Currency, no decimals"),
        "running Number Format from the palette should open its picker"
    );

    d.click_text("Currency, no decimals");
    assert!(
        !d.screen_has("Cancel"),
        "clicking a choice should close the picker"
    );
    assert!(
        d.screen_has("$1,234"),
        "the clicked choice's currency format should apply to A1"
    );
}

/// The palette chord pressed while a menu dropdown is open closes the
/// dropdown and opens the palette, rather than leaving the dropdown
/// open (and interactive again once the palette closes) underneath it.
/// Needs a blank `CalcApp::new()` as its fixture.
pub fn palette_chord_closes_an_open_menu<D: ConformanceDriver + DriverInput>(d: &mut D) {
    d.click_text("File");
    assert!(
        d.screen_has("Quit"),
        "clicking File should open its dropdown"
    );

    d.dispatch(ctrl_shift_p());
    assert!(
        d.screen_has("Commands"),
        "Ctrl+Shift+P should open the command palette even with a dropdown open"
    );
    assert!(
        !d.screen_has("Quit"),
        "opening the palette should close the File dropdown"
    );

    ConformanceDriver::press_named(d, NamedKey::Escape);
    assert!(
        !d.screen_has("Quit"),
        "closing the palette should not bring the dropdown back"
    );
}

/// The centre of `needle`'s first painted occurrence, in the driver's
/// own units — `ConformanceDriver` has `click_text` but no right-click or
/// double-click counterpart, so those gestures are dispatched here at
/// the point `click_text` would have clicked. Located within its text
/// run proportionally, since a run can be wider than the needle.
fn locate<D: ConformanceDriver>(d: &D, needle: &str) -> quadraui::Point {
    let inventory = d.inventory();
    let run = inventory
        .text_runs()
        .iter()
        .find(|r| r.text.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} is not painted"));
    let chars = run.text.chars().count().max(1) as f32;
    let before = run.text[..run.text.find(needle).unwrap()].chars().count() as f32;
    let mid = before + needle.chars().count() as f32 / 2.0;
    quadraui::Point::new(
        run.bounds.x + run.bounds.width * mid / chars,
        run.bounds.y + run.bounds.height / 2.0,
    )
}

fn key_with(key: Key, modifiers: Modifiers) -> UiEvent {
    UiEvent::KeyPressed {
        key,
        modifiers,
        repeat: false,
    }
}

/// Ctrl+F opens the find panel; typing a query jumps to the first match
/// at or after the cursor and counts them, Enter steps to the next, and
/// Escape closes the panel without quitting. Needs
/// `CalcApp::with_sample()` as its fixture: the monthly table's
/// "Over target?" column says "yes" in F16:F22.
pub fn find_steps_through_matches<D: ConformanceDriver + DriverInput>(d: &mut D) {
    ConformanceDriver::ctrl_char(d, 'f');
    d.type_text("yes");
    assert!(
        d.screen_has("1 of 7"),
        "typing a query should count the matches and land on the first"
    );
    assert!(
        d.screen_has("F16"),
        "the cursor should jump to the first match, F16"
    );

    ConformanceDriver::press_named(d, NamedKey::Enter);
    assert!(
        d.screen_has("2 of 7"),
        "Enter should step to the next match"
    );
    assert!(d.screen_has("F17"), "the next match is F17");

    d.dispatch(key_with(
        Key::Named(NamedKey::Enter),
        Modifiers {
            shift: true,
            ..Default::default()
        },
    ));
    assert!(
        d.screen_has("1 of 7"),
        "Shift+Enter should step back to the previous match"
    );

    ConformanceDriver::press_named(d, NamedKey::Escape);
    assert!(!d.screen_has("of 7"), "Escape should close the find panel");
    assert!(
        !d.exited(),
        "Escape in the find panel must not quit the app"
    );
}

/// Ctrl+H opens find-and-replace; Ctrl+Enter in the replace field
/// replaces every match through the engine's own `edit.replace`. Needs a
/// blank `CalcApp::new()` as its fixture.
pub fn replace_all_through_the_find_panel<D: ConformanceDriver + DriverInput>(d: &mut D) {
    // Short enough to fit an 8-wide default column unclipped.
    for text in ["ab", "ab cd", "ef"] {
        d.type_text(text);
        ConformanceDriver::press_named(d, NamedKey::Enter);
    }
    ConformanceDriver::ctrl_char(d, 'h');
    d.type_text("ab");
    assert!(d.screen_has("of 2"), "two cells contain \"ab\"");
    ConformanceDriver::press_named(d, NamedKey::Tab); // to the replace field
    d.type_text("xy");
    d.dispatch(key_with(
        Key::Named(NamedKey::Enter),
        Modifiers {
            ctrl: true,
            ..Default::default()
        },
    ));
    assert!(
        d.screen_has("Replaced 2"),
        "replace-all should report how many cells it changed"
    );
    assert!(d.screen_has("xy cd"), "A2 should read \"xy cd\"");
    assert!(d.screen_has("ef"), "a non-matching cell is left alone");
    assert!(!d.screen_has("ab cd"), "A2's old text should be gone");
}

/// Right-clicking a cell opens the grid's context menu at the pointer;
/// clicking "Clear Contents" clears the clicked cell through
/// `edit.clearContents` and closes the menu. Shift+F10 opens the same
/// menu from the keyboard, and Escape closes it. Needs
/// `CalcApp::with_sample()` as its fixture.
pub fn context_menu_clears_a_cell<D: ConformanceDriver + DriverInput>(d: &mut D) {
    let position = locate(d, "Hosting");
    d.dispatch(UiEvent::MouseDown {
        widget: None,
        button: quadraui::MouseButton::Right,
        position,
        modifiers: Modifiers::default(),
    });
    assert!(
        d.screen_has("Insert Row") && d.screen_has("Clear Contents"),
        "right-click should open the grid's context menu"
    );

    d.click_text("Clear Contents");
    assert!(
        !d.screen_has("Insert Row"),
        "activating an item should close the menu"
    );
    assert!(
        !d.screen_has("Hosting"),
        "Clear Contents should empty the right-clicked cell, A2"
    );

    d.dispatch(key_with(
        Key::Named(NamedKey::F(10)),
        Modifiers {
            shift: true,
            ..Default::default()
        },
    ));
    assert!(
        d.screen_has("Clear Contents"),
        "Shift+F10 should open the context menu at the cursor"
    );
    ConformanceDriver::press_named(d, NamedKey::Escape);
    assert!(
        !d.screen_has("Clear Contents"),
        "Escape should close the context menu"
    );
    assert!(!d.exited(), "Escape in the context menu must not quit");
}

/// The mouse wheel scrolls the grid while the frozen header row stays
/// put, and scrolling back up brings the top rows back. Needs
/// `CalcApp::with_sample()` as its fixture: row 1 is frozen.
pub fn wheel_scrolls_under_a_frozen_header<D: ConformanceDriver + DriverInput>(d: &mut D) {
    d.scroll_at("Hosting", -10);
    assert!(
        !d.screen_has("Hosting") && !d.screen_has("Domains"),
        "scrolling down should move rows 2-3 off screen"
    );
    assert!(
        d.screen_has("Discounted Total"),
        "the frozen header row should stay on screen"
    );
    assert!(
        d.screen_has("A1"),
        "the wheel scrolls the view, not the cursor"
    );

    d.scroll_at("Discounted Total", 20);
    assert!(
        d.screen_has("Hosting"),
        "scrolling back up should bring row 2 back"
    );
}

/// Double-clicking a cell opens it for editing with the caret after its
/// text, like F2. Needs `CalcApp::with_sample()` as its fixture.
pub fn double_click_edits_in_place<D: ConformanceDriver + DriverInput>(d: &mut D) {
    let position = locate(d, "Domains");
    d.dispatch(UiEvent::DoubleClick {
        widget: None,
        position,
    });
    d.type_text("!");
    ConformanceDriver::press_named(d, NamedKey::Enter);
    assert!(
        d.screen_has("Domains!"),
        "double-click should edit A3 in place, keeping its text"
    );
}
