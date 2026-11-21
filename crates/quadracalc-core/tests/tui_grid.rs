//! Black-box test for `CalcApp`'s `CellGrid`: drives it through quadraui's
//! `TuiDriver` (real production render + dispatch path, headless
//! `TestBackend`) and asserts on what's actually painted, not just on
//! `CellGridController`'s own state (see `crates/quadracalc-core/src/app.rs`
//! for the unit tests covering that).

use quadraui::tui::testing::TuiDriver;
use quadraui::{Key, Modifiers, NamedKey, UiEvent};

use quadracalc_core::CalcApp;

fn driver() -> TuiDriver<CalcApp> {
    TuiDriver::new(CalcApp::with_sample().unwrap(), 80, 20)
}

fn shift_right(driver: &mut TuiDriver<CalcApp>) {
    driver.dispatch(UiEvent::KeyPressed {
        key: Key::Named(NamedKey::Right),
        modifiers: Modifiers {
            shift: true,
            ..Default::default()
        },
        repeat: false,
    });
}

#[test]
fn first_frame_shows_the_cursor_at_a1_and_the_sample_data() {
    let driver = driver();
    assert!(driver.screen_contains("A1"), "{}", driver.screen());
    assert!(driver.screen_contains("Item"), "{}", driver.screen());
    assert!(driver.screen_contains("Hosting"), "{}", driver.screen());
}

#[test]
fn arrow_keys_move_the_cursor_and_the_status_bar_follows() {
    let mut driver = driver();
    driver.press_named(NamedKey::Down);
    driver.press_named(NamedKey::Right);

    assert!(driver.screen_contains("B2"), "{}", driver.screen());
    assert!(
        driver.screen_contains("1,200.50") || driver.screen_contains("1200.5"),
        "{}",
        driver.screen()
    );
}

#[test]
fn shift_arrow_selects_a_range_and_the_status_bar_shows_it() {
    let mut driver = driver();
    driver.press_named(NamedKey::Down);
    shift_right(&mut driver);

    assert!(driver.screen_contains("A2:B2"), "{}", driver.screen());
}

#[test]
fn page_down_then_home_scrolls_the_frozen_header_stays_and_column_resets() {
    let mut driver = driver();
    driver.press_named(NamedKey::Right);
    driver.press_named(NamedKey::PageDown);
    let after_page_down = driver.app().cursor();
    assert!(
        after_page_down.row > 0,
        "PageDown should move the cursor down"
    );
    assert_eq!(after_page_down.col, 1, "PageDown keeps the current column");

    driver.press_named(NamedKey::Home);
    let after_home = driver.app().cursor();
    assert_eq!(
        after_home.row, after_page_down.row,
        "Home stays on the same row"
    );
    assert_eq!(after_home.col, 0, "Home returns to the first column");

    let label = format!("A{}", after_home.row + 1);
    assert!(driver.screen_contains(&label), "{}", driver.screen());
    // The column-letter header strip is CellGrid's own fixed chrome, not
    // part of the scrolled body, so it paints every frame regardless of
    // scroll position. `C` names a column the sheet has no data in and
    // the cursor never visits in this test, so it can only come from that
    // header strip — unlike `A`, which the cursor-address assertion above
    // already puts on screen.
    assert!(driver.screen_contains("C"), "{}", driver.screen());
}
