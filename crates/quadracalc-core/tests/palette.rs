//! Command palette over GridCraft's command catalog (issue #4).
//!
//! The first two tests drive `quadracalc_core::palette`'s public API
//! directly against a plain `gridcraft_engine::Session` — "the catalog
//! loads and filters" needs no display, same split as
//! `tests/editing.rs`'s core-state tests vs. its one `TuiDriver` test.
//! The rest are black-box: they drive `CalcApp` through quadraui's
//! `TuiDriver` (the same `AppLogic::handle`/`render` path a runner
//! uses) and assert on what's actually painted.

use gridcraft_engine::Session;
use quadraui::tui::testing::TuiDriver;
use quadraui::{Key, Modifiers, NamedKey, UiEvent};

use quadracalc_core::CalcApp;
use quadracalc_core::palette::{CommandPalette, entries};

fn session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s
}

#[test]
fn the_catalog_loads_in_full_and_a_simple_command_is_runnable() {
    let s = session();
    let all = entries(&s);
    assert_eq!(all.len(), gridcraft_engine::catalog::CATALOG.len());
    let bold = all.iter().find(|e| e.id == "home.bold").unwrap();
    assert!(bold.enabled, "{:?}", bold.disabled_reason);
}

#[test]
fn filtering_by_label_narrows_the_list_to_the_matching_command() {
    let s = session();
    let mut palette = CommandPalette::open(&s);
    for c in "bold".chars() {
        palette.handle_key(&Key::Char(c), &Modifiers::default());
    }
    let labels: Vec<&str> = palette.filtered().iter().map(|e| e.label).collect();
    assert!(labels.contains(&"Bold"), "{labels:?}");
    assert!(labels.len() < gridcraft_engine::catalog::CATALOG.len());
}

fn driver() -> TuiDriver<CalcApp> {
    TuiDriver::new(CalcApp::with_sample().unwrap(), 80, 24)
}

fn type_str(driver: &mut TuiDriver<CalcApp>, s: &str) {
    for c in s.chars() {
        driver.type_char(c);
    }
}

/// `:` opens the palette on the TUI, filtering to "Clear Contents" and
/// confirming it clears the cursor cell's content (A1 = "Item" in the
/// sample sheet) — the effect lands on screen and the palette itself
/// closes.
#[test]
fn colon_opens_the_palette_filters_and_runs_clear_contents() {
    let mut driver = driver();
    assert!(driver.screen_contains("Item"), "{}", driver.screen());

    driver.type_char(':');
    assert!(driver.screen_contains("Commands"), "{}", driver.screen());

    type_str(&mut driver, "clear contents");
    assert!(
        driver.screen_contains("Clear Contents"),
        "{}",
        driver.screen()
    );

    driver.press_named(NamedKey::Enter);

    // The palette closed...
    assert!(!driver.screen_contains("Commands"), "{}", driver.screen());
    // ...and A1's content is gone.
    assert!(!driver.screen_contains("Item"), "{}", driver.screen());
}

/// Ctrl+Shift+P opens the same palette as `:` — the GUI-shaped
/// shortcut from the issue.
#[test]
fn ctrl_shift_p_also_opens_the_palette() {
    let mut driver = driver();
    driver.dispatch(UiEvent::KeyPressed {
        key: Key::Char('P'),
        modifiers: Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        },
        repeat: false,
    });
    assert!(driver.screen_contains("Commands"), "{}", driver.screen());
}

/// A catalog entry whose command needs a parameter the palette can't
/// supply yet (`file.open` needs a path) is shown with its reason
/// instead of being hidden, and Enter on it does nothing.
#[test]
fn a_disabled_entry_shows_its_reason_and_does_not_run_on_enter() {
    let mut driver = driver();
    driver.type_char(':');
    type_str(&mut driver, "open");
    assert!(driver.screen_contains("Open"), "{}", driver.screen());
    assert!(driver.screen_contains("needs"), "{}", driver.screen());

    driver.press_named(NamedKey::Enter);
    // Still open — Enter on a disabled row is a no-op, not a run.
    assert!(driver.screen_contains("Commands"), "{}", driver.screen());
}

/// Escape dismisses the palette without running anything.
#[test]
fn escape_closes_the_palette_without_running_anything() {
    let mut driver = driver();
    driver.type_char(':');
    assert!(driver.screen_contains("Commands"), "{}", driver.screen());

    driver.press_named(NamedKey::Escape);
    assert!(!driver.screen_contains("Commands"), "{}", driver.screen());
    // The sample sheet is untouched.
    assert!(driver.screen_contains("Item"), "{}", driver.screen());
}
