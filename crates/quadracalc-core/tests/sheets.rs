//! Sheet tabs: switch, add and rename sheets (issue #6).
//!
//! The "core" tests below drive `CalcApp` through its own public API
//! (`activate_sheet`/`next_sheet`/`previous_sheet`/`execute`) and assert
//! on logical state (`sheet_names`/`active_sheet_index`/`cursor`) — no
//! display needed, same split `tests/editing.rs` already uses. The
//! `TuiDriver` test at the bottom is the black-box counterpart: it
//! drives a real terminal-shaped render, clicks a tab, and asserts the
//! visible sheet content actually changes.

use quadraui::tui::TuiBackend;
use quadraui::tui::testing::TuiDriver;
use quadraui::{AppLogic, Backend, Key, Modifiers, NamedKey, UiEvent, Viewport, WidgetId};
use serde_json::json;

use quadracalc_core::{Address, CalcApp};

fn backend() -> TuiBackend {
    let mut b = TuiBackend::new();
    b.begin_frame(Viewport::new(80.0, 20.0, 1.0));
    b
}

fn press(app: &mut CalcApp, b: &mut TuiBackend, named: NamedKey) {
    app.handle(
        UiEvent::KeyPressed {
            key: Key::Named(named),
            modifiers: Modifiers::default(),
            repeat: false,
        },
        b,
    );
}

/// `with_sample()`'s one sheet ("Sheet1") plus a freshly inserted
/// "Data" — the engine activates a newly inserted sheet immediately, so
/// this starts on "Data" at index 1.
fn app_with_two_sheets() -> CalcApp {
    let mut app = CalcApp::with_sample().unwrap();
    app.execute("home.insertSheet", json!({"name": "Data"}))
        .unwrap();
    app
}

#[test]
fn a_fresh_workbook_has_one_sheet_and_it_is_active() {
    let app = CalcApp::with_sample().unwrap();
    assert_eq!(app.sheet_names(), vec!["Sheet1".to_string()]);
    assert_eq!(app.active_sheet_index(), 0);
}

#[test]
fn inserting_a_sheet_adds_a_tab_and_activates_it() {
    let app = app_with_two_sheets();
    assert_eq!(
        app.sheet_names(),
        vec!["Sheet1".to_string(), "Data".to_string()]
    );
    assert_eq!(
        app.active_sheet_index(),
        1,
        "the engine activates a newly inserted sheet"
    );
}

#[test]
fn renaming_a_sheet_updates_its_tab() {
    let mut app = CalcApp::with_sample().unwrap();
    app.execute("sheet.rename", json!({"sheet": 0, "name": "Costs"}))
        .unwrap();
    assert_eq!(app.sheet_names(), vec!["Costs".to_string()]);
}

#[test]
fn activate_sheet_switches_the_active_index() {
    let mut app = app_with_two_sheets();
    assert_eq!(app.active_sheet_index(), 1);
    app.activate_sheet(0).unwrap();
    assert_eq!(app.active_sheet_index(), 0);
    app.activate_sheet(1).unwrap();
    assert_eq!(app.active_sheet_index(), 1);
}

/// `sheet.next`/`sheet.previous` move one sheet at a time and stop (not
/// wrap) at either end of the workbook — the engine's own behaviour
/// (`gridcraft_engine`'s `hidden_sheet_navigation` tests), which
/// `CalcApp::next_sheet`/`previous_sheet` pass straight through.
#[test]
fn next_and_previous_sheet_move_and_stop_at_the_ends() {
    let mut app = app_with_two_sheets();
    assert_eq!(app.active_sheet_index(), 1);
    app.next_sheet().unwrap();
    assert_eq!(app.active_sheet_index(), 1, "next stays on the last sheet");
    app.previous_sheet().unwrap();
    assert_eq!(app.active_sheet_index(), 0);
    app.previous_sheet().unwrap();
    assert_eq!(
        app.active_sheet_index(),
        0,
        "previous stays on the first sheet"
    );
}

/// Acceptance criterion: switching sheets keeps a separate cursor per
/// sheet. A sheet visited for the first time starts at the origin;
/// switching away and back restores exactly where it was left.
#[test]
fn each_sheet_keeps_its_own_cursor_across_a_switch() {
    let mut app = app_with_two_sheets();
    let mut b = backend();
    app.setup(&mut b);
    assert_eq!(app.active_sheet_index(), 1, "starts on \"Data\"");

    // Move "Data"'s cursor to row 2, column 1 (B3).
    press(&mut app, &mut b, NamedKey::Down);
    press(&mut app, &mut b, NamedKey::Down);
    press(&mut app, &mut b, NamedKey::Right);
    assert_eq!(app.cursor(), Address::new(2, 1));

    // "Sheet1" has never been switched to via `activate_sheet` before,
    // so it starts at the origin rather than inheriting "Data"'s cursor.
    app.activate_sheet(0).unwrap();
    assert_eq!(app.cursor(), Address::new(0, 0));

    // Move "Sheet1"'s own cursor, then switch to "Data": its B3 cursor
    // comes back exactly as it was left.
    press(&mut app, &mut b, NamedKey::Right);
    app.activate_sheet(1).unwrap();
    assert_eq!(
        app.cursor(),
        Address::new(2, 1),
        "\"Data\"'s cursor should be restored"
    );

    // And switching back to "Sheet1" restores its own cursor too.
    app.activate_sheet(0).unwrap();
    assert_eq!(app.cursor(), Address::new(0, 1));
}

#[test]
fn ctrl_page_down_and_up_switch_sheets_through_handle() {
    let mut app = app_with_two_sheets();
    let mut b = backend();
    app.setup(&mut b);
    assert_eq!(app.active_sheet_index(), 1);

    let ctrl_page = |named| UiEvent::KeyPressed {
        key: Key::Named(named),
        modifiers: Modifiers {
            ctrl: true,
            ..Default::default()
        },
        repeat: false,
    };

    app.handle(ctrl_page(NamedKey::PageUp), &mut b);
    assert_eq!(app.active_sheet_index(), 0);

    app.handle(ctrl_page(NamedKey::PageDown), &mut b);
    assert_eq!(app.active_sheet_index(), 1);
}

fn driver() -> TuiDriver<CalcApp> {
    TuiDriver::new(CalcApp::with_sample().unwrap(), 80, 24)
}

fn type_str(driver: &mut TuiDriver<CalcApp>, s: &str) {
    for c in s.chars() {
        driver.type_char(c);
    }
}

/// Black-box: insert a second sheet through the command palette (its
/// params are all optional, so it runs with no further input), then
/// click back to the first tab and confirm the visible sheet content
/// actually changes.
#[test]
fn tui_driver_clicking_a_tab_switches_the_visible_sheet() {
    let mut driver = driver();
    assert!(driver.screen_contains("Item"), "{}", driver.screen());
    assert!(driver.screen_contains("Sheet1"), "{}", driver.screen());

    driver.type_char(':');
    // "insert sheet home" — not just "insert sheet" — so the query also
    // matches into the row's own "Home Cells" tab/group text: "Insert
    // Sheet"'s own `home cells` sits right after the match with no gap,
    // while "Insert Sheet Rows"/"Insert Sheet Columns" only reach it
    // after the extra word, scoring lower. Without the longer query all
    // three tie and the catalog's own (alphabetically earlier) "Insert
    // Sheet Rows" sorts first instead.
    type_str(&mut driver, "insert sheet home");
    assert!(
        driver.screen_contains("Insert Sheet"),
        "{}",
        driver.screen()
    );
    driver.press_named(NamedKey::Enter);

    assert_eq!(driver.app().sheet_names().len(), 2);
    assert_eq!(
        driver.app().active_sheet_index(),
        1,
        "inserting a sheet activates it"
    );
    // The new sheet is empty — the sample's "Item" is gone from screen.
    assert!(!driver.screen_contains("Item"), "{}", driver.screen());

    let bar_id = WidgetId::new("quadracalc-sheet-tabs");
    let (x, y) = driver
        .tab_center(&bar_id, 0)
        .expect("tab 0 should be visible on an 80-column bar with two short tabs");
    driver.click(x, y);

    assert_eq!(driver.app().active_sheet_index(), 0);
    assert!(driver.screen_contains("Item"), "{}", driver.screen());
}

/// Blocking fix: once there are more sheets than an 80-column bar can show
/// at once, the active tab must stay on (or scroll back onto) the strip —
/// not get silently clipped from index 0 the moment the bar overflows.
/// Each `home.insertSheet` activates the new sheet immediately, so this
/// drives the overflow case through the exact path a user hits by adding
/// sheets one at a time via the command palette.
#[test]
fn inserting_past_an_overflowing_tab_bar_keeps_the_active_tab_reachable() {
    let mut driver = driver();
    let bar_id = WidgetId::new("quadracalc-sheet-tabs");

    // "Sheet1" plus enough inserted sheets that their " SheetN " labels
    // can no longer all fit an 80-column strip at once.
    for _ in 0..14 {
        driver.type_char(':');
        type_str(&mut driver, "insert sheet home");
        driver.press_named(NamedKey::Enter);
    }

    let last = driver.app().sheet_names().len() - 1;
    assert_eq!(
        driver.app().active_sheet_index(),
        last,
        "inserting a sheet activates it"
    );
    assert!(
        driver.tab_center(&bar_id, last).is_some(),
        "the just-activated sheet's tab must be scrolled onto the bar, \
         not left behind at a stale scroll offset: {}",
        driver.screen()
    );

    // Switching all the way back to the first sheet, then forward again,
    // must keep tracking the active tab rather than getting stuck at
    // whichever scroll offset the first sheet needed. Driven through
    // `Ctrl+PageUp`/`Ctrl+PageDown`, same as a real user, so the
    // assertion exercises `handle`'s own refresh rather than calling
    // `CalcApp::activate_sheet` directly and skipping it.
    let ctrl_page = |named| UiEvent::KeyPressed {
        key: Key::Named(named),
        modifiers: Modifiers {
            ctrl: true,
            ..Default::default()
        },
        repeat: false,
    };
    for _ in 0..last {
        driver.dispatch(ctrl_page(NamedKey::PageUp));
    }
    assert_eq!(driver.app().active_sheet_index(), 0);
    assert!(
        driver.tab_center(&bar_id, 0).is_some(),
        "{}",
        driver.screen()
    );

    for _ in 0..last {
        driver.dispatch(ctrl_page(NamedKey::PageDown));
    }
    assert_eq!(driver.app().active_sheet_index(), last);
    assert!(
        driver.tab_center(&bar_id, last).is_some(),
        "re-activating the last sheet must scroll it back into view: {}",
        driver.screen()
    );
}
