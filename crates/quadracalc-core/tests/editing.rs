//! In-cell editing, formula bar and undo/redo (issue #3).
//!
//! The "core" tests below drive `CalcApp` through its real `AppLogic::handle`
//! path (the same dispatch a runner uses) with a headless `TuiBackend`, but
//! assert only on logical state (`CalcApp::cursor`/`cell_text`/`view`) —
//! never on rendered screen output, so they need no display. The one
//! `TuiDriver` test at the bottom is the black-box counterpart: it drives
//! through a real terminal-shaped render and asserts on what's actually
//! painted (see `tests/tui_grid.rs` for the sibling covering plain
//! navigation).

use quadraui::tui::TuiBackend;
use quadraui::tui::testing::TuiDriver;
use quadraui::{AppLogic, Backend, Key, Modifiers, NamedKey, UiEvent, Viewport};

use quadracalc_core::CalcApp;

fn backend() -> TuiBackend {
    let mut b = TuiBackend::new();
    b.begin_frame(Viewport::new(80.0, 20.0, 1.0));
    b
}

fn key(key: Key, modifiers: Modifiers) -> UiEvent {
    UiEvent::KeyPressed {
        key,
        modifiers,
        repeat: false,
    }
}

fn press_char(app: &mut CalcApp, b: &mut TuiBackend, c: char) {
    app.handle(key(Key::Char(c), Modifiers::default()), b);
}

fn press_named(app: &mut CalcApp, b: &mut TuiBackend, named: NamedKey) {
    app.handle(key(Key::Named(named), Modifiers::default()), b);
}

fn ctrl_char(app: &mut CalcApp, b: &mut TuiBackend, c: char) {
    app.handle(
        key(
            Key::Char(c),
            Modifiers {
                ctrl: true,
                ..Default::default()
            },
        ),
        b,
    );
}

fn type_str(app: &mut CalcApp, b: &mut TuiBackend, s: &str) {
    for c in s.chars() {
        press_char(app, b, c);
    }
}

// ── Edit / commit ────────────────────────────────────────────────────

#[test]
fn committing_a_formula_recalculates_its_dependents_in_the_same_handle_call() {
    let mut app = CalcApp::with_sample().unwrap();
    let mut b = backend();
    app.setup(&mut b);
    assert!(app.cell_text("B4").contains("1209.5") || app.cell_text("B4").contains("1,209.50"));

    // Move the cursor to B3 ("9") and retype it as "20" through the real
    // in-cell editor: a plain character starts the edit, Enter commits.
    press_named(&mut app, &mut b, NamedKey::Right);
    press_named(&mut app, &mut b, NamedKey::Down);
    press_named(&mut app, &mut b, NamedKey::Down);
    assert_eq!(app.cursor().a1(), "B3");
    type_str(&mut app, &mut b, "20");
    press_named(&mut app, &mut b, NamedKey::Enter);

    assert_eq!(app.cell_input("B3"), "20");
    let total = app.cell_text("B4");
    assert!(
        total.contains("1220.5") || total.contains("1,220.50"),
        "B4 should already reflect B3's new value by the time this handle call returns: {total}"
    );
    // `view()` is rebuilt on every `handle` that redraws, so the cursor
    // moved on to B4 (Enter's own "move down") and the formula bar
    // mirrors it.
    assert_eq!(app.cursor().a1(), "B4");
    assert_eq!(app.view().active_input, "=SUM(B2:B3)");
    assert_eq!(app.view().formula.value, "=SUM(B2:B3)");
}

#[test]
fn f2_seeds_the_editor_and_escape_cancels_leaving_the_cell_and_formula_bar_unchanged() {
    let mut app = CalcApp::with_sample().unwrap();
    let mut b = backend();
    app.setup(&mut b);
    assert_eq!(app.cursor().a1(), "A1");
    assert_eq!(app.cell_input("A1"), "Item");

    press_named(&mut app, &mut b, NamedKey::F(2));
    press_char(&mut app, &mut b, '!');
    assert!(
        app.view().formula.value.contains("Item!"),
        "F2 should seed the formula bar with the cell's existing text, cursor at the end: {:?}",
        app.view().formula
    );

    press_named(&mut app, &mut b, NamedKey::Escape);
    assert_eq!(
        app.cell_input("A1"),
        "Item",
        "Escape should discard the in-progress edit"
    );
    assert_eq!(
        app.view().formula.value,
        "Item",
        "the formula bar should revert to the committed text once the edit is cancelled"
    );
    assert!(app.view().error.is_none());
}

// ── Formula errors vs. invalid input ────────────────────────────────

#[test]
fn formula_errors_render_in_the_cell_rather_than_erroring_the_command() {
    let mut app = CalcApp::new();
    app.set_cell("A1", "=1/0").unwrap();
    assert_eq!(app.cell_text("A1"), "#DIV/0!");

    app.set_cell("A2", "=NOTAREALFUNCTION(1)").unwrap();
    assert_eq!(app.cell_text("A2"), "#NAME?");
}

#[test]
fn an_invalid_input_errors_cell_set_and_lands_in_the_status_bar() {
    let mut app = CalcApp::new();
    let mut b = backend();
    app.setup(&mut b);

    // LEFT() takes at least one argument — a formula calling a known
    // function with too few arguments is rejected by the engine at
    // entry time, as `anyhow`'d through `CalcApp::execute`, rather than
    // stored as a value that would display as an in-cell error.
    press_char(&mut app, &mut b, '=');
    type_str(&mut app, &mut b, "LEFT()");
    press_named(&mut app, &mut b, NamedKey::Enter);

    assert_eq!(
        app.cell_input("A1"),
        "",
        "a rejected cell.set must never write the bad input into the cell"
    );
    let err = app.view().error.clone();
    assert!(
        err.as_deref().is_some_and(|e| e.contains("few")),
        "the engine's own rejection message should land in the status bar: {err:?}"
    );
}

// ── Undo / redo ──────────────────────────────────────────────────────

#[test]
fn ctrl_z_and_ctrl_y_restore_both_the_value_and_the_formula_bar() {
    let mut app = CalcApp::new();
    let mut b = backend();
    app.setup(&mut b);

    app.set_cell("A1", "hello").unwrap();
    // `set_cell` talks to the engine directly and never refreshes `view`
    // itself (see its own doc) — `setup` is the public hook that does,
    // the same one every runner calls once at startup.
    app.setup(&mut b);
    assert_eq!(app.view().active_input, "hello");
    assert_eq!(app.view().formula.value, "hello");

    ctrl_char(&mut app, &mut b, 'z');
    assert_eq!(app.cell_input("A1"), "");
    assert_eq!(app.view().active_input, "");
    assert_eq!(
        app.view().formula.value,
        "",
        "undo should restore the formula bar along with the cell"
    );

    ctrl_char(&mut app, &mut b, 'y');
    assert_eq!(app.cell_input("A1"), "hello");
    assert_eq!(app.view().active_input, "hello");
    assert_eq!(
        app.view().formula.value,
        "hello",
        "redo should restore the formula bar along with the cell"
    );
}

#[test]
fn ctrl_z_never_fires_while_an_edit_is_in_progress() {
    let mut app = CalcApp::new();
    let mut b = backend();
    app.setup(&mut b);
    app.set_cell("A1", "hello").unwrap();

    // Start a fresh edit and send Ctrl+Z mid-edit: it must insert/ignore
    // like any other keypress the grid's own editor doesn't special-case,
    // never reach the engine's undo — the same "only outside an active
    // edit" guard Ctrl+S already gets.
    press_char(&mut app, &mut b, 'x');
    ctrl_char(&mut app, &mut b, 'z');
    press_named(&mut app, &mut b, NamedKey::Escape);
    assert_eq!(
        app.cell_input("A1"),
        "hello",
        "Ctrl+Z during an edit must not undo past it"
    );
}

// ── Black-box: TuiDriver ─────────────────────────────────────────────

#[test]
fn tui_driver_types_a_formula_and_commits_the_recalculated_value_and_formula_bar() {
    let mut driver = TuiDriver::new(CalcApp::with_sample().unwrap(), 80, 20);
    driver.press_named(NamedKey::Right);
    driver.press_named(NamedKey::Down);
    driver.press_named(NamedKey::Down);
    assert!(driver.app().cursor().a1() == "B3");

    for ch in "20".chars() {
        driver.type_char(ch);
    }
    let editing = driver.screen();
    assert!(
        editing.contains("20"),
        "the in-cell editor should paint the in-progress text:\n{editing}"
    );

    driver.press_named(NamedKey::Enter);
    let after = driver.screen();
    assert!(
        after.contains("1220.5") || after.contains("1,220.50"),
        "B4's dependent total should recalculate on screen in the same frame the edit commits:\n{after}"
    );
    assert!(
        after.contains("=SUM(B2:B3)"),
        "the formula bar should show B4's raw formula once the cursor lands there:\n{after}"
    );
}
