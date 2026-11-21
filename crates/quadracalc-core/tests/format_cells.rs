//! Format Cells pickers (issue #17): a font-name/size, fill/font color,
//! border-preset and number-format picker over `crate::format_picker`,
//! wired through the command palette.
//!
//! Every GridCraft command these commands run (`home.fontSize`,
//! `home.fillColor`, `home.borders`, `home.numberFormat`, …) needs a
//! parameter the palette itself can't supply, so before this picker
//! existed the row showed up disabled with a "needs …" reason
//! (`tests/palette.rs`'s own `a_disabled_entry_shows_its_reason…` test
//! covers that still-true case for `file.open`, which has no picker).
//! These are black-box: every assertion drives `CalcApp` through
//! quadraui's `TuiDriver`, the same `AppLogic::handle`/`render` path a
//! runner uses, and reads the effect back off `cell.get`'s own style
//! JSON — never `crate::format_picker`'s types directly (those get
//! their own in-crate unit tests).

use quadraui::{Backend, NamedKey, OwnedContent};
use quadraui::tui::testing::TuiDriver;
use serde_json::json;

use quadracalc_core::CalcApp;

fn driver() -> TuiDriver<CalcApp> {
    TuiDriver::new(CalcApp::with_sample().unwrap(), 80, 24)
}

fn type_str(driver: &mut TuiDriver<CalcApp>, s: &str) {
    for c in s.chars() {
        driver.type_char(c);
    }
}

fn style(driver: &mut TuiDriver<CalcApp>, cell: &str) -> serde_json::Value {
    driver
        .app_mut()
        .execute("cell.get", json!({ "cell": cell }))
        .unwrap()["style"]
        .clone()
}

/// Opening the palette and filtering to "font size" shows the row
/// enabled (no "needs" reason) — the one thing that changed about this
/// catalog entry's classification.
#[test]
fn font_size_is_enabled_in_the_palette_now_that_a_picker_can_supply_it() {
    let mut driver = driver();
    driver.type_char(':');
    type_str(&mut driver, "font size");
    assert!(driver.screen_contains("Font Size"), "{}", driver.screen());
    assert!(!driver.screen_contains("needs"), "{}", driver.screen());
}

/// Running "Font Size" from the palette opens the picker dialog instead
/// of the palette closing with nothing happening; its default choice
/// (the first, smallest size) applies on Enter.
#[test]
fn font_size_opens_a_picker_and_the_default_choice_applies_on_enter() {
    let mut driver = driver();
    driver.type_char(':');
    type_str(&mut driver, "font size");
    driver.press_named(NamedKey::Enter);

    // The palette closed, replaced by the picker dialog.
    assert!(!driver.screen_contains("Commands"), "{}", driver.screen());
    assert!(driver.screen_contains("Font Size"), "{}", driver.screen());
    assert!(driver.screen_contains("Cancel"), "{}", driver.screen());

    driver.press_named(NamedKey::Enter);

    // The picker closed and A1's font size changed from the default.
    assert!(!driver.screen_contains("Cancel"), "{}", driver.screen());
    let size = style(&mut driver, "A1")["font"]["size"].as_f64().unwrap();
    assert_eq!(size, 8.0, "expected the first/default choice");
}

/// Escape on the picker closes it without touching the cell.
#[test]
fn escape_on_the_picker_cancels_without_applying_anything() {
    let mut driver = driver();
    let before = style(&mut driver, "A1")["font"]["size"].clone();

    driver.type_char(':');
    type_str(&mut driver, "font size");
    driver.press_named(NamedKey::Enter);
    assert!(driver.screen_contains("Font Size"), "{}", driver.screen());

    driver.press_named(NamedKey::Escape);
    assert!(!driver.screen_contains("Cancel"), "{}", driver.screen());
    assert_eq!(style(&mut driver, "A1")["font"]["size"], before);
}

/// Down (remapped onto the dialog's own Left/Right button-cycling, since
/// every picker here renders vertically) moves off the default choice
/// before Enter confirms it — fill color's second entry ("Black") after
/// its own default ("No Fill").
#[test]
fn down_then_enter_picks_the_second_choice_not_the_default() {
    let mut driver = driver();
    driver.type_char(':');
    type_str(&mut driver, "fill color");
    driver.press_named(NamedKey::Enter);
    assert!(driver.screen_contains("Fill Color"), "{}", driver.screen());

    driver.press_named(NamedKey::Down);
    driver.press_named(NamedKey::Enter);

    let fill = style(&mut driver, "A1")["fill"].clone();
    assert_eq!(fill["pattern"], "Solid", "{fill:?}");
}

/// Borders' default preset ("All") sets every edge once confirmed —
/// the same `home.borders` command the palette used to disable outright
/// (it needs a required `preset`).
#[test]
fn borders_default_preset_sets_every_edge() {
    let mut driver = driver();
    driver.type_char(':');
    type_str(&mut driver, "borders");
    // Narrow past "Draw Border"/other "border…" labels to the exact row.
    assert!(driver.screen_contains("Borders"), "{}", driver.screen());
    driver.press_named(NamedKey::Enter);
    assert!(driver.screen_contains("All"), "{}", driver.screen());

    driver.press_named(NamedKey::Enter);

    let borders = style(&mut driver, "A1")["borders"].clone();
    assert_ne!(borders["top"]["style"], "None", "{borders:?}");
}

/// Number Format's default choice ("General") is a no-op on a cell
/// that's already General, so this picks "Currency" (two Downs past
/// "Number") to see a real effect.
#[test]
fn number_format_can_pick_past_the_default_choice() {
    let mut driver = driver();
    driver.type_char(':');
    type_str(&mut driver, "number format");
    driver.press_named(NamedKey::Enter);
    assert!(driver.screen_contains("Currency"), "{}", driver.screen());

    driver.press_named(NamedKey::Down);
    driver.press_named(NamedKey::Down);
    driver.press_named(NamedKey::Enter);

    let code = style(&mut driver, "A1")["num_fmt"].clone();
    assert!(code.as_str().unwrap_or_default().contains('$'), "{code:?}");
}

/// The open picker is pushed through `ScreenLayout` as a `Surface::Dialog`
/// — not painted with a direct `backend.draw_dialog` call that bypasses
/// frame capture — so it shows up in `Backend::last_frame`'s captured
/// node tree, the same place the accessibility tree and the MCP
/// `snapshot`/`act` tools read a screen from (`Backend::last_frame`'s own
/// doc: "an app that calls `backend.draw_*` methods directly, without a
/// `ScreenLayout`, gets `None` here even with capture enabled").
#[test]
fn the_open_picker_is_captured_in_the_frame_as_a_dialog_node() {
    let mut driver = driver();
    driver.type_char(':');
    type_str(&mut driver, "font size");
    driver.press_named(NamedKey::Enter);
    assert!(driver.screen_contains("Font Size"), "{}", driver.screen());

    driver.render();
    let frame = driver
        .backend()
        .last_frame()
        .expect("frame capture must still have a frame once the picker is open");
    assert!(
        frame
            .nodes()
            .iter()
            .any(|n| matches!(n.content, OwnedContent::Dialog(ref d) if d.title.spans[0].text == "Font Size")),
        "expected a captured Dialog node titled \"Font Size\"; nodes were {:?}",
        frame.nodes(),
    );
}

/// A click outside the picker's dialog box closes it without applying
/// anything — matching `PickerOutcome::Closed`'s own doc.
#[test]
fn clicking_outside_the_picker_closes_it_without_applying_anything() {
    let mut driver = driver();
    let before = style(&mut driver, "A1")["font"]["size"].clone();

    driver.type_char(':');
    type_str(&mut driver, "font size");
    driver.press_named(NamedKey::Enter);
    assert!(driver.screen_contains("Font Size"), "{}", driver.screen());

    // The dialog is centred in an 80x24 viewport; the very top-left
    // corner is well outside its bounds.
    driver.click(0.0, 0.0);

    assert!(!driver.screen_contains("Cancel"), "{}", driver.screen());
    assert_eq!(style(&mut driver, "A1")["font"]["size"], before);
}
