//! Clipboard and fill (issue #5).
//!
//! The "core" tests below drive `CalcApp::execute` straight against the
//! engine's own `edit.copy`/`edit.cut`/`edit.paste`/`edit.fillDown`/
//! `edit.fillRight` commands — the same surface `crate::clipboard`'s
//! Ctrl+C/X/V/D/R bridge calls — with no backend and no display, since
//! what's under test here is the engine's own reference-adjustment and
//! fill behaviour, not the keyboard wiring (`tests/editing.rs`'s own doc
//! makes the same "core vs. black-box" split). The one `TuiDriver` test
//! at the bottom is the black-box counterpart for fill-down, asserting
//! on what's actually painted.

use quadraui::tui::TuiBackend;
use quadraui::tui::testing::TuiDriver;
use quadraui::{AppLogic, Backend, Key, Modifiers, NamedKey, UiEvent, Viewport};
use serde_json::json;

use quadracalc_core::CalcApp;

fn select(app: &mut CalcApp, range: &str, active: &str) {
    app.execute("selection.set", json!({ "range": range, "active": active }))
        .unwrap();
}

fn backend() -> TuiBackend {
    let mut b = TuiBackend::new();
    b.begin_frame(Viewport::new(80.0, 20.0, 1.0));
    b
}

fn ctrl_char(app: &mut CalcApp, b: &mut TuiBackend, c: char) {
    app.handle(
        UiEvent::KeyPressed {
            key: Key::Char(c),
            modifiers: Modifiers {
                ctrl: true,
                ..Default::default()
            },
            repeat: false,
        },
        b,
    );
}

fn press_named(app: &mut CalcApp, b: &mut TuiBackend, named: NamedKey) {
    app.handle(
        UiEvent::KeyPressed {
            key: Key::Named(named),
            modifiers: Modifiers::default(),
            repeat: false,
        },
        b,
    );
}

fn shift_down(app: &mut CalcApp, b: &mut TuiBackend) {
    app.handle(
        UiEvent::KeyPressed {
            key: Key::Named(NamedKey::Down),
            modifiers: Modifiers {
                shift: true,
                ..Default::default()
            },
            repeat: false,
        },
        b,
    );
}

// ── Copy / paste ─────────────────────────────────────────────────────

#[test]
fn copy_then_paste_adjusts_relative_references_through_the_command_surface() {
    let mut app = CalcApp::new();
    app.set_cell("A1", "10").unwrap();
    app.set_cell("B1", "=A1*2").unwrap();
    assert_eq!(app.cell_text("B1"), "20");

    select(&mut app, "B1", "B1");
    let copied = app.execute("edit.copy", json!({})).unwrap();
    assert_eq!(copied["range"].as_str(), Some("B1"));
    assert_eq!(copied["text"].as_str().unwrap().trim_end(), "20");

    select(&mut app, "B5", "B5");
    app.execute("edit.paste", json!({})).unwrap();

    assert_eq!(
        app.cell_input("B5"),
        "=A5*2",
        "pasting four rows down should shift the relative reference by four rows"
    );
    // A5 is empty (0), so B5 should read 0.
    assert_eq!(app.cell_text("B5"), "0");
    // The source is untouched by a copy (only a cut clears it).
    assert_eq!(app.cell_input("B1"), "=A1*2");
}

#[test]
fn cut_clears_the_source_and_keeps_the_formula_unshifted_at_the_destination() {
    let mut app = CalcApp::new();
    app.set_cell("A1", "1").unwrap();
    app.set_cell("B1", "=A1+1").unwrap();
    assert_eq!(app.cell_text("B1"), "2");

    select(&mut app, "B1", "B1");
    app.execute("edit.cut", json!({})).unwrap();
    // `edit.cut` only arms the clipboard — the move happens on paste.
    assert_eq!(app.cell_input("B1"), "=A1+1");

    select(&mut app, "B5", "B5");
    app.execute("edit.paste", json!({})).unwrap();

    assert_eq!(
        app.cell_input("B1"),
        "",
        "cut should clear the source once pasted elsewhere"
    );
    assert_eq!(
        app.cell_input("B5"),
        "=A1+1",
        "a cut formula keeps its own references unshifted, unlike a copy"
    );
    assert_eq!(app.cell_text("B5"), "2");
}

#[test]
fn tsv_pasted_from_outside_fills_the_range_starting_at_the_cursor() {
    let mut app = CalcApp::new();
    select(&mut app, "C3", "C3");

    // Plain tab-separated text, as another app's clipboard would hold it
    // (CellGrid clipboard, quadraui#2729) — no engine clipboard was ever
    // armed, so `edit.paste` must treat this `text` as the thing to
    // paste rather than erroring with "Nothing to paste.".
    app.execute("edit.paste", json!({ "text": "10\t20\n30\t40" }))
        .unwrap();

    assert_eq!(app.cell_text("C3"), "10");
    assert_eq!(app.cell_text("D3"), "20");
    assert_eq!(app.cell_text("C4"), "30");
    assert_eq!(app.cell_text("D4"), "40");
}

// ── Fill ─────────────────────────────────────────────────────────────

#[test]
fn fill_down_through_the_command_surface_adjusts_relative_references() {
    let mut app = CalcApp::new();
    app.set_cell("A1", "5").unwrap();
    app.set_cell("A2", "6").unwrap();
    app.set_cell("A3", "7").unwrap();
    app.set_cell("B1", "=A1*2").unwrap();
    assert_eq!(app.cell_text("B1"), "10");

    select(&mut app, "B1:B3", "B1");
    app.execute("edit.fillDown", json!({})).unwrap();

    assert_eq!(app.cell_input("B2"), "=A2*2");
    assert_eq!(app.cell_text("B2"), "12");
    assert_eq!(app.cell_input("B3"), "=A3*2");
    assert_eq!(app.cell_text("B3"), "14");
}

#[test]
fn fill_right_through_the_command_surface_adjusts_relative_references() {
    let mut app = CalcApp::new();
    app.set_cell("A1", "5").unwrap();
    app.set_cell("B1", "6").unwrap();
    app.set_cell("C1", "7").unwrap();
    app.set_cell("A2", "=A1*2").unwrap();
    assert_eq!(app.cell_text("A2"), "10");

    select(&mut app, "A2:C2", "A2");
    app.execute("edit.fillRight", json!({})).unwrap();

    assert_eq!(app.cell_input("B2"), "=B1*2");
    assert_eq!(app.cell_text("B2"), "12");
    assert_eq!(app.cell_input("C2"), "=C1*2");
    assert_eq!(app.cell_text("C2"), "14");
}

// ── Keyboard wiring (no system clipboard involved) ──────────────────

#[test]
fn ctrl_d_through_handle_fills_down_the_selection() {
    let mut app = CalcApp::new();
    let mut b = backend();
    app.setup(&mut b);
    app.set_cell("A1", "5").unwrap();
    app.set_cell("A2", "6").unwrap();
    app.set_cell("B1", "=A1*2").unwrap();

    // Real navigation, so the UI's own selection (what `sync_selection`
    // reads) covers B1:B2, not just the default single-cell A1. The
    // first dispatch below also refreshes the view after the direct
    // `set_cell` calls above, same as any other `handle` call would.
    press_named(&mut app, &mut b, NamedKey::Right);
    shift_down(&mut app, &mut b);
    assert_eq!(app.view().selection_label, "B1:B2");

    ctrl_char(&mut app, &mut b, 'd');

    assert_eq!(app.cell_input("B2"), "=A2*2");
    assert_eq!(app.cell_text("B2"), "12");
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
}

// ── Black-box: TuiDriver ─────────────────────────────────────────────

#[test]
fn tui_driver_fill_down_shows_the_recalculated_values() {
    let mut driver = TuiDriver::new(CalcApp::new(), 80, 20);
    driver.app_mut().set_cell("A1", "500").unwrap();
    driver.app_mut().set_cell("A2", "600").unwrap();
    driver.app_mut().set_cell("A3", "700").unwrap();
    driver.app_mut().set_cell("B1", "=A1*2").unwrap();
    // `set_cell` talks to the engine directly and never refreshes the
    // view itself — a plain navigation dispatch below is what repaints.
    driver.press_named(NamedKey::Right);
    let before = driver.screen();
    assert!(
        before.contains("1000"),
        "B1 should show its computed value before the fill:\n{before}"
    );

    // Real navigation builds the selection B1:B3, so `sync_selection`
    // picks it up the same way a live keyboard-driven fill would.
    driver.dispatch(UiEvent::KeyPressed {
        key: Key::Named(NamedKey::Down),
        modifiers: Modifiers {
            shift: true,
            ..Default::default()
        },
        repeat: false,
    });
    driver.dispatch(UiEvent::KeyPressed {
        key: Key::Named(NamedKey::Down),
        modifiers: Modifiers {
            shift: true,
            ..Default::default()
        },
        repeat: false,
    });
    assert_eq!(driver.app().view().selection_label, "B1:B3");

    driver.ctrl_char('d');

    let after = driver.screen();
    assert!(
        after.contains("1200"),
        "B2 should recalculate to 1200 on screen after fill-down:\n{after}"
    );
    assert!(
        after.contains("1400"),
        "B3 should recalculate to 1400 on screen after fill-down:\n{after}"
    );
}

#[test]
fn ctrl_v_through_dispatch_pastes_a_copied_formula_with_adjusted_references() {
    // Drives the *production* paste path end to end: a real Ctrl+C through
    // `dispatch_event` arms the engine's own clipboard, then a real
    // Ctrl+V — also through `dispatch_event`, quadraui's full
    // backend-translation + preprocessing pipeline, not a direct
    // `CalcApp::execute("edit.paste", …)` call — reaches
    // `CellGridController::handle`'s `UiEvent::ClipboardPaste` arm,
    // which reports `CellGridControllerEvent::PasteRequested` for
    // `CalcApp::apply_controller_event` to turn into `edit.paste`. This
    // is the exact path that was unreachable before: the system
    // clipboard this process actually has (headless, no
    // `DISPLAY`/`WAYLAND_DISPLAY`) never reads back what it wrote, so
    // `UiEvent::ClipboardPaste` is injected directly with the text the
    // copy produced — standing in for "the OS clipboard round-tripped
    // it", which `crate::clipboard`'s non-blocking concern about
    // `TuiClipboard`'s write/read asymmetry covers separately — while
    // still exercising every line downstream of that event through the
    // real dispatcher.
    let mut driver = TuiDriver::new(CalcApp::new(), 80, 20);
    driver.app_mut().set_cell("A1", "10").unwrap();
    driver.app_mut().set_cell("B1", "=A1*2").unwrap();
    driver.press_named(NamedKey::Right);
    assert_eq!(driver.app().view().selection_label, "B1");

    driver.ctrl_char('c');
    let copied_text = driver.app_mut().cell_text("B1");
    assert_eq!(copied_text, "20");

    for _ in 0..4 {
        driver.press_named(NamedKey::Down);
    }
    assert_eq!(driver.app().view().selection_label, "B5");

    driver.dispatch(UiEvent::ClipboardPaste(copied_text));

    assert!(
        driver.app().view().error.is_none(),
        "{:?}",
        driver.app().view().error
    );
    assert_eq!(
        driver.app_mut().cell_input("B5"),
        "=A5*2",
        "pasting four rows down should shift the relative reference by four rows"
    );
    assert_eq!(driver.app_mut().cell_text("B5"), "0");
    // The source is untouched by a copy, same as the core-level test.
    assert_eq!(driver.app_mut().cell_input("B1"), "=A1*2");
}

/// Menu/palette Paste (`clipboard::paste_from_system`, issue #38) over a
/// system clipboard that can't be read at all — the SSH/headless case
/// `TuiClipboard::read_text` hits unconditionally in this test process
/// (no `DISPLAY`/`WAYLAND_DISPLAY`, same as `ctrl_v_through_dispatch_…`'s
/// own comment above). Driven through the command palette rather than a
/// direct `clipboard::paste_from_system` call, since that function is
/// `pub(crate)` and the palette is its production caller.
///
/// Regression for `paste_from_system` bailing with "system clipboard is
/// empty" on a `None` read instead of falling back to the engine's own
/// internal clipboard a prior in-app `edit.copy` armed — exactly the
/// path that worked before `paste_from_system` existed.
#[test]
fn palette_paste_falls_back_to_the_engines_internal_clipboard_when_the_system_clipboard_is_unreadable()
 {
    let mut driver = TuiDriver::new(CalcApp::new(), 80, 20);
    driver.app_mut().set_cell("A1", "10").unwrap();
    driver.app_mut().set_cell("B1", "=A1*2").unwrap();
    driver.press_named(NamedKey::Right);
    assert_eq!(driver.app().view().selection_label, "B1");

    driver.ctrl_char('c');
    assert_eq!(driver.app_mut().cell_text("B1"), "20");

    for _ in 0..4 {
        driver.press_named(NamedKey::Down);
    }
    assert_eq!(driver.app().view().selection_label, "B5");

    driver.type_char(':');
    for c in "paste".chars() {
        driver.type_char(c);
    }
    assert!(driver.screen_contains("Paste"), "{}", driver.screen());
    driver.press_named(NamedKey::Enter);

    assert!(
        driver.app().view().error.is_none(),
        "menu/palette Paste should fall back to the internal clipboard \
         instead of erroring when the system clipboard can't be read: {:?}",
        driver.app().view().error
    );
    assert_eq!(
        driver.app_mut().cell_input("B5"),
        "=A5*2",
        "pasting four rows down should shift the relative reference by four rows"
    );
    assert_eq!(driver.app_mut().cell_text("B5"), "0");
    assert_eq!(driver.app_mut().cell_input("B1"), "=A1*2");
}
