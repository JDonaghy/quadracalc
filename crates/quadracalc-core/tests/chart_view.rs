//! Insert › Charts/PivotChart (#20): `insert.chart`/`insert.pivotChart`
//! ran fine against the engine before this existed — every param in
//! their own catalog doc is optional, so `crate::palette::classify`
//! never disabled the row — but nothing painted the result: this is
//! the black-box check that it now does, the same reasoning
//! `tests/sidebar.rs`'s own doc gives for driving through quadraui's
//! `TuiDriver` rather than asserting on `CalcApp`'s state directly.

use quadraui::tui::testing::TuiDriver;
use quadraui::{UiEvent, Viewport};
use serde_json::json;

use quadracalc_core::CalcApp;

fn driver() -> TuiDriver<CalcApp> {
    TuiDriver::new(CalcApp::with_sample().unwrap(), 80, 24)
}

/// `app_mut().execute` (like `tests/sidebar.rs`'s own `formulas.defineName`
/// call) runs the command but leaves `CalcApp::view` stale until some
/// event reaches `handle`, which is the only thing that calls
/// `refresh`. A same-size `WindowResized` forces exactly that without
/// otherwise changing anything, so it's used here (and in
/// `app.rs::tests::window_resized_refreshes_the_loaded_window`) purely
/// as a refresh trigger.
fn force_refresh(driver: &mut TuiDriver<CalcApp>) {
    driver.dispatch(UiEvent::WindowResized {
        viewport: Viewport::new(80.0, 24.0, 1.0),
    });
}

#[test]
fn a_fresh_sheet_shows_no_chart_panel() {
    let driver = driver();
    assert!(!driver.screen_contains("Series 1"), "{}", driver.screen());
}

#[test]
fn inserting_a_chart_paints_its_series_legend() {
    let mut driver = driver();
    driver
        .app_mut()
        .execute("insert.chart", json!({"range": "A2:B3", "type": "line"}))
        .unwrap();
    force_refresh(&mut driver);
    assert!(driver.screen_contains("Series 1"), "{}", driver.screen());
}

#[test]
fn a_pivot_chart_also_paints() {
    let mut driver = driver();
    driver
        .app_mut()
        .execute(
            "insert.pivotTable",
            json!({"source": "A1:B4", "rows": ["Item"], "values": ["Cost"]}),
        )
        .unwrap();
    driver
        .app_mut()
        .execute("insert.pivotChart", json!({}))
        .unwrap();
    force_refresh(&mut driver);
    assert!(driver.screen_contains("Series 1"), "{}", driver.screen());
}
