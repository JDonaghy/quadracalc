//! Right-hand Name Manager sidebar (issue #18): closed by default,
//! toggled with Ctrl+F3, lists `formulas.defineName`'s own names and
//! their current values, and can be closed again by clicking the
//! toolbar's close button. Black-box through quadraui's `TuiDriver` —
//! the same `AppLogic::handle`/`render` path a runner uses — since
//! every behaviour here is visible on screen.

use quadraui::tui::testing::TuiDriver;
use quadraui::{Key, Modifiers, NamedKey, UiEvent};
use serde_json::json;

use quadracalc_core::CalcApp;

fn driver() -> TuiDriver<CalcApp> {
    TuiDriver::new(CalcApp::with_sample().unwrap(), 80, 24)
}

fn ctrl_f3() -> UiEvent {
    UiEvent::KeyPressed {
        key: Key::Named(NamedKey::F(3)),
        modifiers: Modifiers {
            ctrl: true,
            ..Default::default()
        },
        repeat: false,
    }
}

#[test]
fn the_sidebar_is_closed_by_default() {
    let driver = driver();
    assert!(
        !driver.screen_contains("Name Manager"),
        "{}",
        driver.screen()
    );
}

#[test]
fn ctrl_f3_toggles_the_sidebar_open_and_closed() {
    let mut driver = driver();
    driver.dispatch(ctrl_f3());
    assert!(
        driver.screen_contains("Name Manager"),
        "{}",
        driver.screen()
    );

    driver.dispatch(ctrl_f3());
    assert!(
        !driver.screen_contains("Name Manager"),
        "{}",
        driver.screen()
    );
}

/// A name defined through GridCraft's own command surface shows up in
/// the sidebar with its current value — the thing this panel exists
/// to surface (nothing else in this app displays a query command's
/// result).
#[test]
fn an_opened_sidebar_lists_a_defined_name_and_its_value() {
    let mut driver = driver();
    driver
        .app_mut()
        .execute(
            "formulas.defineName",
            json!({"name": "Rate", "refersTo": "=0.07"}),
        )
        .unwrap();

    driver.dispatch(ctrl_f3());
    assert!(driver.screen_contains("Rate"), "{}", driver.screen());
    assert!(driver.screen_contains("0.07"), "{}", driver.screen());
}

/// Clicking the toolbar's close button closes the sidebar — mouse-only
/// parity with the Ctrl+F3 toggle.
#[test]
fn clicking_the_close_button_closes_the_sidebar() {
    let mut driver = driver();
    driver.dispatch(ctrl_f3());
    assert!(
        driver.screen_contains("Name Manager"),
        "{}",
        driver.screen()
    );

    let (x, y) = driver
        .find("×")
        .expect("close button glyph should be on screen");
    driver.click(x, y);

    assert!(
        !driver.screen_contains("Name Manager"),
        "{}",
        driver.screen()
    );
}

/// Clicking a row in the name list selects it — painted as quadraui's
/// own `▶ ` selection prefix (`quadraui::tui::list`'s `is_selected`
/// marker) — without closing the sidebar or disturbing the grid's own
/// cursor.
#[test]
fn clicking_a_name_row_selects_it_without_closing_the_sidebar() {
    let mut driver = driver();
    driver
        .app_mut()
        .execute(
            "formulas.defineName",
            json!({"name": "Rate", "refersTo": "=0.07"}),
        )
        .unwrap();
    driver.dispatch(ctrl_f3());
    assert!(driver.screen_contains("Rate"), "{}", driver.screen());
    assert!(
        !driver.screen_contains("▶"),
        "nothing should be selected yet: {}",
        driver.screen()
    );

    let (x, y) = driver.find("Rate").expect("the row should be on screen");
    driver.click(x, y);

    assert!(
        driver.screen_contains("Name Manager"),
        "clicking a row must not close the sidebar: {}",
        driver.screen()
    );
    assert!(
        driver.screen_contains("▶ Rate"),
        "the clicked row should now be marked selected: {}",
        driver.screen()
    );
}
