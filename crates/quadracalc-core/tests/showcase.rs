//! Black-box TUI tests for the terminal showcase: the sample sheet's look
//! (number formats, frozen panes, gridlines, banded rows), the name box
//! and status-bar mode/help line, sub-row wheel scrolling, horizontal
//! scrolling past a frozen column, and the formula bar's own pointer
//! editing.
//!
//! These are `TuiDriver`-only, per this crate's CLAUDE.md: they assert on
//! terminal specifics — the `│` separator glyphs and the cell background
//! colours zebra striping paints, character-grid row text, and
//! cell-unit pointer positions inside the formula bar. The cross-backend
//! behaviours (find/replace, the context menu, wheel scrolling,
//! double-click editing, quitting) are in `tests/conformance/mod.rs`.

use quadraui::tui::testing::TuiDriver;
use quadraui::{Key, Modifiers, NamedKey, Point, ScrollDelta, UiEvent};

use quadracalc_core::CalcApp;

const W: u16 = 100;
const H: u16 = 30;

fn sample() -> TuiDriver<CalcApp> {
    TuiDriver::new(CalcApp::with_sample().unwrap(), W, H)
}

fn ctrl(c: char) -> UiEvent {
    UiEvent::KeyPressed {
        key: Key::Char(c),
        modifiers: Modifiers {
            ctrl: true,
            ..Default::default()
        },
        repeat: false,
    }
}

fn alt(c: char) -> UiEvent {
    UiEvent::KeyPressed {
        key: Key::Char(c),
        modifiers: Modifiers {
            alt: true,
            ..Default::default()
        },
        repeat: false,
    }
}

fn scroll(d: &mut TuiDriver<CalcApp>, at: &str, dx: f32, dy: f32) {
    let (x, y) = d
        .find(at)
        .unwrap_or_else(|| panic!("{at:?}\n{}", d.screen()));
    d.dispatch(UiEvent::Scroll {
        widget: None,
        delta: ScrollDelta::new(dx, dy),
        position: Point::new(x, y),
    });
}

/// The screen row whose text contains `needle`.
fn row_of(d: &TuiDriver<CalcApp>, needle: &str) -> u16 {
    d.screen()
        .lines()
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not on screen\n{}", d.screen())) as u16
}

fn line(d: &TuiDriver<CalcApp>, y: u16) -> String {
    d.screen()
        .lines()
        .nth(y as usize)
        .unwrap_or_default()
        .to_string()
}

#[test]
fn the_sample_sheet_shows_formatted_formula_results_at_startup() {
    let d = sample();
    let screen = d.screen();
    for needle in [
        "Discounted Total",
        "$1,209.50",
        "15.00%",
        "$1,028.08",
        "Over target?",
        "$62,027.00",
        "7 of 12",
        "Best month",
        "Dec",
        "$8,424.00",
        // Rates as percentages, unit counts with a thousands separator,
        // and the year's growth in the summary block, not the Total row.
        "5.00%",
        "2,429",
        "Jan→Dec",
        "192.50%",
    ] {
        assert!(screen.contains(needle), "{needle:?} missing\n{screen}");
    }
    assert!(!screen.contains("0.05"), "B7 is a percentage\n{screen}");
    let total = line(&d, row_of(&d, "$62,027.00"));
    assert!(
        !total.contains('%'),
        "the Total row has no growth figure: {total:?}"
    );
}

#[test]
fn gridlines_draw_column_separators_and_follow_view_gridlines() {
    let mut d = sample();
    let y = row_of(&d, "Hosting");
    let before = line(&d, y).matches('│').count();
    assert!(
        before >= 6,
        "a body row should carry a separator per column: {:?}",
        line(&d, y)
    );

    d.dispatch(alt('v'));
    let (x, gy) = d.find("Gridlines").expect("View › Gridlines");
    d.click(x, gy);
    let after = line(&d, row_of(&d, "Hosting")).matches('│').count();
    assert!(
        after < before,
        "View › Gridlines off should drop the column separators: {before} → {after}\n{}",
        d.screen()
    );
}

#[test]
fn banded_rows_stripe_alternate_rows_and_toggle_from_the_view_menu() {
    // Tall enough for the whole View dropdown, Banded Rows at its foot.
    let mut d = TuiDriver::new(CalcApp::with_sample().unwrap(), W, 60);
    // Two empty body cells one row apart, in column C (no fills there).
    let (cx, _) = d.find("Price").unwrap();
    let y24 = row_of(&d, " 24 ");
    let bg = |d: &TuiDriver<CalcApp>, y: u16| d.style_at(cx as u16 + 1, y).unwrap().bg;
    assert_ne!(
        bg(&d, y24),
        bg(&d, y24 + 1),
        "neighbouring body rows should be striped differently"
    );

    d.dispatch(alt('v'));
    let (x, my) = d.find("Banded Rows").expect("View › Banded Rows");
    d.click(x, my);
    assert_eq!(
        bg(&d, y24),
        bg(&d, y24 + 1),
        "turning Banded Rows off should paint both rows alike"
    );
}

#[test]
fn the_name_box_and_status_bar_follow_the_selection_and_mode() {
    let mut d = sample();
    assert!(d.screen_contains("READY"), "{}", d.screen());
    assert!(
        d.screen_contains("F2 edit"),
        "the help line should show\n{}",
        d.screen()
    );

    d.press_named(NamedKey::Down);
    d.dispatch(UiEvent::KeyPressed {
        key: Key::Named(NamedKey::Right),
        modifiers: Modifiers {
            shift: true,
            ..Default::default()
        },
        repeat: false,
    });
    let name_box = line(&d, 1);
    assert!(name_box.contains("A2:B2"), "name box: {name_box:?}");

    d.press_named(NamedKey::Down); // a single cell: no summary crowding the hints
    d.press_named(NamedKey::F(2));
    assert!(d.screen_contains("EDIT"), "{}", d.screen());
    assert!(d.screen_contains("Esc cancel"), "{}", d.screen());
    d.press_named(NamedKey::Escape);

    d.dispatch(ctrl('f'));
    assert!(d.screen_contains("FIND"), "{}", d.screen());
}

#[test]
fn a_fractional_wheel_delta_carries_over_instead_of_rounding_up() {
    let mut d = sample();
    // 0.2 of a notch is 0.6 rows: not enough to move on its own.
    scroll(&mut d, "Hosting", 0.0, -0.2);
    assert!(
        d.screen_contains("Cost"),
        "0.6 rows shouldn't scroll\n{}",
        d.screen()
    );
    // A second one adds up to 1.2 rows: exactly one row (row 1) scrolls.
    scroll(&mut d, "Domains", 0.0, -0.2);
    assert!(
        !d.screen_contains("Cost"),
        "1.2 rows should scroll one row\n{}",
        d.screen()
    );
    assert!(d.screen_contains("Hosting"), "only one row\n{}", d.screen());
}

#[test]
fn horizontal_scroll_keeps_the_frozen_first_column() {
    let mut d = sample();
    scroll(&mut d, "Hosting", 3.0, 0.0);
    let screen = d.screen();
    assert!(screen.contains("Hosting"), "column A is frozen\n{screen}");
    assert!(
        !screen.contains("$1,200.50"),
        "B..D scrolled away\n{screen}"
    );
    assert!(
        screen.contains("Discounted Total"),
        "F is now on screen\n{screen}"
    );
}

#[test]
fn dragging_in_the_formula_bar_selects_text_that_typing_replaces() {
    let mut d = sample();
    // The formula bar mirrors A1's "Item"; it sits above the grid, so
    // `find` lands on it before the cell's own "Item".
    let bounds = d.find_bounds("Item").unwrap();
    assert_eq!(bounds.y, 1.0, "formula bar row\n{}", d.screen());
    // From the caret boundary before "I" to the one after "t".
    let y = bounds.y + 0.5;
    d.drag(bounds.x, y, bounds.x + 2.0, y);
    d.type_char('Z');
    d.press_named(NamedKey::Enter);
    let a1 = d.app_mut().cell_input("A1");
    assert_eq!(
        a1, "Zem",
        "dragging over \"It\" then typing should replace just that"
    );
}

#[test]
fn ctrl_chords_inside_a_cell_edit_edit_the_text_not_the_sheet() {
    let mut d = sample();
    d.press_named(NamedKey::F(2)); // edit A1 = "Item"
    d.dispatch(ctrl('a')); // select all of the edit buffer
    d.dispatch(ctrl('x')); // cut it — not the sheet's Cut
    for c in "Name".chars() {
        d.type_char(c);
    }
    d.press_named(NamedKey::Enter);
    assert_eq!(d.app_mut().cell_input("A1"), "Name");
    assert!(d.app().view().error.is_none(), "{:?}", d.app().view().error);
}

#[test]
fn the_find_count_follows_edits_made_with_the_panel_open() {
    let mut d = sample();
    d.dispatch(ctrl('f'));
    for c in "yes".chars() {
        d.type_char(c);
    }
    assert!(d.screen_contains("of 7"), "{}", d.screen());

    // Click an empty cell (the grid takes the keyboard back, the panel
    // stays open) and type another match into it.
    let (x, y) = d.find(" 24 ").unwrap();
    d.click(x + 8.0, y);
    assert!(
        d.screen_contains("7 found"),
        "the panel stays open, counting matches off the cursor\n{}",
        d.screen()
    );
    for c in "yes".chars() {
        d.type_char(c);
    }
    d.press_named(NamedKey::Enter);
    assert!(
        d.screen_contains("8 found"),
        "a new matching cell should be counted\n{}",
        d.screen()
    );

    // With the grid holding the keyboard, Escape closes the panel.
    d.press_named(NamedKey::Escape);
    assert!(!d.screen_contains("8 found"), "{}", d.screen());
    assert!(!d.exited());
}

/// TUI-specific: the help line's width is in terminal cells. A default
/// 80-column terminal gets the short set of key hints instead of none.
#[test]
fn a_narrow_terminal_gets_the_short_key_hints() {
    let d = TuiDriver::new(CalcApp::with_sample().unwrap(), 80, 24);
    assert!(d.screen_contains("^Q quit"), "{}", d.screen());
    let d = sample();
    assert!(d.screen_contains("Ctrl+Q quit"), "{}", d.screen());
}

#[test]
fn escape_collapses_a_selection_instead_of_quitting() {
    let mut d = sample();
    d.dispatch(UiEvent::KeyPressed {
        key: Key::Named(NamedKey::Down),
        modifiers: Modifiers {
            shift: true,
            ..Default::default()
        },
        repeat: false,
    });
    assert!(line(&d, 1).contains("A1:A2"), "{}", d.screen());
    d.press_named(NamedKey::Escape);
    assert!(!d.exited());
    assert!(!line(&d, 1).contains("A1:A2"), "{}", d.screen());
    d.press_named(NamedKey::Escape);
    assert!(!d.exited(), "Escape with nothing to collapse does nothing");
}

/// Freeze Panes runs from the palette and freezes at the cursor; run
/// again on a frozen sheet, it unfreezes (the engine's own toggle).
#[test]
fn freeze_panes_runs_from_the_palette_at_the_cursor() {
    let mut d = TuiDriver::new(CalcApp::new(), W, H);
    d.press_named(NamedKey::Down);
    d.press_named(NamedKey::Down);
    d.press_named(NamedKey::Right);
    let run_freeze = |d: &mut TuiDriver<CalcApp>| {
        d.type_char(':');
        for c in "freeze panes".chars() {
            d.type_char(c);
        }
        assert!(!d.screen_contains("needs"), "{}", d.screen());
        d.press_named(NamedKey::Enter);
        let grid = d.app().view().grid.clone().unwrap();
        (grid.frozen_rows, grid.frozen_cols)
    };
    assert_eq!(run_freeze(&mut d), (2, 1));
    assert_eq!(run_freeze(&mut d), (0, 0));
}

/// TUI-specific (the labels are matched as terminal text): the replace
/// row's buttons read as words, and clicking "[All]" replaces every
/// counted match.
#[test]
fn the_replace_buttons_are_labelled_and_clickable() {
    let mut d = sample();
    d.dispatch(ctrl('h'));
    for c in "Premium".chars() {
        d.type_char(c);
    }
    d.press_named(NamedKey::Tab);
    for c in "Gold".chars() {
        d.type_char(c);
    }
    assert!(
        d.screen_contains("[Replace]") && d.screen_contains("[All]"),
        "{}",
        d.screen()
    );
    let (x, y) = d.find("[All]").unwrap();
    d.click(x + 1.0, y);
    assert!(d.screen_contains("Replaced 2"), "{}", d.screen());
    // A8 and D2 both read "Gold" now.
    assert!(d.screen_contains("8 Gold"), "{}", d.screen());
    assert!(d.screen_contains("│Gold "), "{}", d.screen());
}
