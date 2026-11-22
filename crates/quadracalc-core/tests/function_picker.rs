//! Formulas tab: "Insert Function…" and the Function Library category
//! buttons (issue #21) open `crate::function_picker::FunctionPicker`
//! instead of the no-op a plain `execute(id, {})` would be.
//!
//! Black-box: every assertion drives `CalcApp` through quadraui's
//! `TuiDriver`, the same `AppLogic::handle`/`render` path a runner uses,
//! and reads the effect back off the screen and `cell.get`'s own `input`
//! field — never `crate::function_picker`'s types directly (those get
//! their own in-crate unit tests).

use quadraui::NamedKey;
use quadraui::tui::testing::TuiDriver;

use quadracalc_core::CalcApp;

fn driver() -> TuiDriver<CalcApp> {
    TuiDriver::new(CalcApp::with_sample().unwrap(), 80, 24)
}

fn type_str(driver: &mut TuiDriver<CalcApp>, s: &str) {
    for c in s.chars() {
        driver.type_char(c);
    }
}

/// Opens the palette, filters to "Insert Function", and confirms it —
/// the function picker replaces the command palette instead of the row
/// just running `formulas.insertFunction` generically with nothing to
/// show for it.
fn open_insert_function(driver: &mut TuiDriver<CalcApp>) {
    driver.type_char(':');
    type_str(driver, "insert function");
    assert!(
        driver.screen_contains("Insert Function"),
        "{}",
        driver.screen()
    );
    driver.press_named(NamedKey::Enter);
}

/// Confirming "Insert Function…" from the palette opens a searchable
/// list, not nothing — the palette itself closes and the picker's own
/// title and first (alphabetically) catalog row show instead.
#[test]
fn insert_function_opens_a_searchable_list_instead_of_doing_nothing() {
    let mut driver = driver();
    open_insert_function(&mut driver);

    assert!(!driver.screen_contains("Commands"), "{}", driver.screen());
    assert!(
        driver.screen_contains("Insert Function"),
        "{}",
        driver.screen()
    );
    // The full, unfiltered catalog is showing — "ABS" sorts first.
    assert!(driver.screen_contains("ABS"), "{}", driver.screen());
}

/// Typing a query fuzzy-filters the list down to the matching function
/// (and nothing it doesn't match), same `fuzzy_score` behaviour the
/// command palette itself already uses.
#[test]
fn typing_narrows_the_list_to_the_matching_function() {
    let mut driver = driver();
    open_insert_function(&mut driver);

    type_str(&mut driver, "vlookup");
    assert!(driver.screen_contains("VLOOKUP"), "{}", driver.screen());
    assert!(!driver.screen_contains("XIRR"), "{}", driver.screen());
}

/// Confirming a row on an empty cell starts a formula with that
/// function's name plus `()` — typed through the formula bar's own
/// caret, not run as a command — and the picker closes. Uses `TODAY`
/// (valid with zero arguments) so the commit below actually succeeds;
/// `VLOOKUP()` with no arguments is a real arity error the engine
/// rejects on `cell.set`, which this test isn't about.
#[test]
fn confirming_a_function_on_an_empty_cell_starts_a_formula_with_it() {
    let mut driver = driver();
    // D1 is blank in the sample sheet.
    driver.press_named(NamedKey::Right);
    driver.press_named(NamedKey::Right);
    driver.press_named(NamedKey::Right);
    assert_eq!(driver.app().cursor().a1(), "D1");

    open_insert_function(&mut driver);
    type_str(&mut driver, "today");
    driver.press_named(NamedKey::Enter);

    // The picker closed...
    assert!(
        !driver.screen_contains("Insert Function"),
        "{}",
        driver.screen()
    );
    // ...and D1 is now mid-edit on a formula that starts with the
    // chosen function, parens included.
    let formula_value = driver.app_mut().view().formula.value.clone();
    assert_eq!(formula_value, "=TODAY()", "{}", driver.screen());

    // Committing writes it back as D1's raw input and the engine
    // evaluates it to today's date, not an error.
    driver.press_named(NamedKey::Enter);
    assert_eq!(driver.app_mut().cell_input("D1"), "=TODAY()");
    assert!(
        !driver.app_mut().cell_text("D1").starts_with('#'),
        "{}",
        driver.app_mut().cell_text("D1")
    );
}

/// A Function Library category button (Financial) opens the same
/// picker pre-filtered to that category via `formulas.functions`'s own
/// `category` param — typing further narrows to one well-known
/// financial function, and a well-known function from a different
/// category never shows at all (the category filter ran before any
/// query did).
#[test]
fn a_category_button_opens_the_same_list_pre_filtered_to_it() {
    let mut driver = driver();
    driver.type_char(':');
    type_str(&mut driver, "financial");
    assert!(driver.screen_contains("Financial"), "{}", driver.screen());
    driver.press_named(NamedKey::Enter);

    type_str(&mut driver, "pmt");
    assert!(driver.screen_contains("PMT"), "{}", driver.screen());
    assert!(!driver.screen_contains("VLOOKUP"), "{}", driver.screen());
}

/// Escape closes the picker without touching the cell.
#[test]
fn escape_closes_the_picker_without_changing_the_cell() {
    let mut driver = driver();
    open_insert_function(&mut driver);
    type_str(&mut driver, "vlookup");

    driver.press_named(NamedKey::Escape);

    assert!(
        !driver.screen_contains("Insert Function"),
        "{}",
        driver.screen()
    );
    assert_eq!(driver.app_mut().cell_input("A1"), "Item");
}
