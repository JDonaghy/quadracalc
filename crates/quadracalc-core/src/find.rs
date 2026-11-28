//! Find & Replace across the active sheet: quadraui's [`FindReplacePanel`]
//! widget as the UI, GridCraft's own `edit.find`/`edit.replace` commands
//! as the search.
//!
//! The panel is controlled — [`FindBar`] owns its fields and writes each
//! [`quadraui::FindReplaceEvent`] back with `apply_event` — and anchors at
//! the top-right of the grid. `CalcApp` routes keys to it while it has
//! the keyboard ([`FindBar::focused`]) and clicks inside its box to it
//! always; a click on the grid hands the keyboard back to the grid while
//! leaving the panel open, so the user can look around between matches.
//!
//! Matching is the engine's: `edit.find` with `all: true` lists every hit,
//! and this module only orders them and steps through them. Find alone
//! looks in what each cell shows (Excel's "Look in: Values" — a formula
//! cell matches on its result). With the replace row showing it looks in
//! what the user typed instead ("Look in: Formulas", the only option
//! Excel's Replace offers), because that is the text `edit.replace`
//! rewrites: the listed hits and the cells a replace may change are then
//! the same cells, and Replace All is further restricted to exactly the
//! listed hits (see [`hits_range`]). Match case maps onto `matchCase` and
//! the whole-word toggle onto `wholeCell` (a spreadsheet's "match entire
//! cell contents"). The engine has no regex or preserve-case search, so
//! those two toggles flip on the panel but don't change the search.

use quadraui::{FR_PANEL_WIDTH, FindReplacePanel, Rect, WidgetId};
use serde_json::{Value, json};

use crate::address::Address;

/// The replace row's button labels: words rather than glyphs, since no
/// icon font can be assumed on every backend this runs on.
pub const REPLACE_ONE_LABEL: &str = "[Replace]";
pub const REPLACE_ALL_LABEL: &str = "[All]";

/// The open find/replace panel and whether it has the keyboard.
#[derive(Debug, Clone)]
pub struct FindBar {
    pub panel: FindReplacePanel,
    pub focused: bool,
    /// A one-off message shown in place of the match count — the result
    /// of a replace-all — until the panel's next event.
    pub note: Option<String>,
}

impl FindBar {
    /// A fresh panel — with the replace row showing when `replace` —
    /// focused on its find field, seeded with `query`.
    pub fn open(replace: bool, query: String) -> Self {
        let cursor = query.chars().count();
        let mut panel = FindReplacePanel {
            id: WidgetId::new("quadracalc-find"),
            query,
            replacement: String::new(),
            show_replace: replace,
            focus: 0,
            cursor,
            sel_anchor: (cursor > 0).then_some(0),
            match_info: String::new(),
            case_sensitive: false,
            whole_word: false,
            use_regex: false,
            preserve_case: false,
            in_selection: false,
            group_bounds: Rect::new(0.0, 0.0, 0.0, 0.0),
            panel_width: FR_PANEL_WIDTH,
            replace_one_glyph: REPLACE_ONE_LABEL.into(),
            replace_all_glyph: REPLACE_ALL_LABEL.into(),
            hit_regions: Vec::new(),
            hidden_toggles: Vec::new(),
        };
        panel.rebuild_hit_regions();
        Self {
            panel,
            focused: true,
            note: None,
        }
    }

    /// Show (or hide) the replace row and take the keyboard — what
    /// Ctrl+F/Ctrl+H do on an already-open panel.
    pub fn refocus(&mut self, replace: bool) {
        if replace != self.panel.show_replace {
            self.panel.show_replace = replace;
            self.panel.rebuild_hit_regions();
        }
        self.panel.focus = 0;
        self.panel.cursor = self.panel.query.chars().count();
        self.panel.sel_anchor = (self.panel.cursor > 0).then_some(0);
        self.focused = true;
    }

    /// This frame's panel, anchored to `group` (the grid's rect).
    pub fn panel_in(&self, group: Rect) -> FindReplacePanel {
        let mut panel = self.panel.clone();
        panel.group_bounds = group;
        panel
    }

    /// `edit.find`'s parameters for listing every match of the current
    /// query — `None` while the query is empty. Looks in cell values for
    /// a plain find and in typed input (formula source) while the
    /// replace row shows, so every hit listed then is one `edit.replace`
    /// can rewrite.
    pub fn find_all_params(&self) -> Option<Value> {
        let look_in = if self.panel.show_replace {
            "formulas"
        } else {
            "values"
        };
        (!self.panel.query.is_empty()).then(|| {
            json!({
                "what": self.panel.query,
                "matchCase": self.panel.case_sensitive,
                "wholeCell": self.panel.whole_word,
                "lookIn": look_in,
                "all": true,
            })
        })
    }

    /// `edit.replace`'s parameters: every match inside the engine's
    /// selection when `all` (the caller selects exactly the listed hits
    /// first), otherwise only the active cell's.
    pub fn replace_params(&self, all: bool) -> Value {
        let mut params = json!({
            "what": self.panel.query,
            "with": self.panel.replacement,
            "matchCase": self.panel.case_sensitive,
            "wholeCell": self.panel.whole_word,
            "all": all,
        });
        if all {
            params["within"] = json!("selection");
        }
        params
    }
}

/// `hits` as one multi-area range for `selection.set` (`"A1,C3,F12"`) —
/// what Replace All selects so it can only touch cells Find listed.
/// `None` with no hits.
pub fn hits_range(hits: &[(usize, usize)]) -> Option<String> {
    (!hits.is_empty()).then(|| {
        hits.iter()
            .map(|&(row, col)| Address::new(row as u32, col as u32).a1())
            .collect::<Vec<_>>()
            .join(",")
    })
}

/// The active sheet's hits from an `edit.find {all: true}` result, in
/// reading order (row by row). Hits on another sheet are dropped:
/// without `within: "workbook"` the engine only searches the active one,
/// and this is the guard if that ever changes.
pub fn matches(result: &Value, active_sheet: &str) -> Vec<(usize, usize)> {
    let mut hits: Vec<(usize, usize)> = result
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|r| {
            r.get("sheet")
                .and_then(Value::as_str)
                .is_none_or(|s| s == active_sheet)
        })
        .filter_map(|r| r.get("cell").and_then(Value::as_str))
        .filter_map(Address::parse)
        .map(|a| (a.row as usize, a.col as usize))
        .collect();
    hits.sort_unstable();
    hits.dedup();
    hits
}

/// The match to step to from `cursor`: the first one after it in reading
/// order (`forward`), or the last one before it, wrapping round either
/// end. `None` with no matches.
pub fn step(
    hits: &[(usize, usize)],
    cursor: (usize, usize),
    forward: bool,
) -> Option<(usize, usize)> {
    if forward {
        hits.iter()
            .find(|&&h| h > cursor)
            .or_else(|| hits.first())
            .copied()
    } else {
        hits.iter()
            .rev()
            .find(|&&h| h < cursor)
            .or_else(|| hits.last())
            .copied()
    }
}

/// The first match at or after `cursor` — where typing a query jumps to,
/// so a match under the cursor stays put.
pub fn nearest(hits: &[(usize, usize)], cursor: (usize, usize)) -> Option<(usize, usize)> {
    hits.iter()
        .find(|&&h| h >= cursor)
        .or_else(|| hits.first())
        .copied()
}

/// The panel's match count: `3 of 12` with the cursor on a match,
/// `12 found` otherwise, `No results` for none, nothing for an empty
/// query.
pub fn match_info(hits: &[(usize, usize)], cursor: (usize, usize), query_empty: bool) -> String {
    if query_empty {
        return String::new();
    }
    if hits.is_empty() {
        return "No results".into();
    }
    match hits.iter().position(|&h| h == cursor) {
        Some(i) => format!("{} of {}", i + 1, hits.len()),
        None => format!("{} found", hits.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HITS: [(usize, usize); 3] = [(1, 0), (1, 4), (7, 2)];

    #[test]
    fn matches_come_back_in_reading_order_from_the_active_sheet_only() {
        let result = json!({"count": 4, "results": [
            {"sheet": "Sheet1", "cell": "C8", "value": "x"},
            {"sheet": "Sheet1", "cell": "E2", "value": "x"},
            {"sheet": "Other", "cell": "A1", "value": "x"},
            {"sheet": "Sheet1", "cell": "A2", "value": "x"},
        ]});
        assert_eq!(matches(&result, "Sheet1"), HITS.to_vec());
    }

    #[test]
    fn stepping_wraps_round_both_ends() {
        assert_eq!(step(&HITS, (0, 0), true), Some((1, 0)));
        assert_eq!(step(&HITS, (1, 0), true), Some((1, 4)));
        assert_eq!(step(&HITS, (7, 2), true), Some((1, 0)));
        assert_eq!(step(&HITS, (1, 0), false), Some((7, 2)));
        assert_eq!(step(&HITS, (5, 5), false), Some((1, 4)));
        assert_eq!(step(&[], (0, 0), true), None);
    }

    #[test]
    fn nearest_keeps_a_match_under_the_cursor() {
        assert_eq!(nearest(&HITS, (1, 4)), Some((1, 4)));
        assert_eq!(nearest(&HITS, (2, 0)), Some((7, 2)));
        assert_eq!(nearest(&HITS, (9, 0)), Some((1, 0)));
    }

    #[test]
    fn match_info_counts_and_positions() {
        assert_eq!(match_info(&HITS, (1, 4), false), "2 of 3");
        assert_eq!(match_info(&HITS, (0, 0), false), "3 found");
        assert_eq!(match_info(&[], (0, 0), false), "No results");
        assert_eq!(match_info(&[], (0, 0), true), "");
    }

    #[test]
    fn replace_mode_searches_the_same_text_replace_rewrites() {
        let mut bar = FindBar::open(false, "D2".into());
        assert_eq!(bar.find_all_params().unwrap()["lookIn"], "values");
        bar.refocus(true);
        assert_eq!(bar.find_all_params().unwrap()["lookIn"], "formulas");
        assert_eq!(bar.replace_params(true)["within"], "selection");
        assert!(bar.replace_params(false).get("within").is_none());
    }

    #[test]
    fn hits_become_one_multi_area_range() {
        assert_eq!(hits_range(&HITS).as_deref(), Some("A2,E2,C8"));
        assert_eq!(hits_range(&[]), None);
    }

    #[test]
    fn opening_selects_the_seeded_query() {
        let bar = FindBar::open(false, "Total".into());
        assert_eq!(bar.panel.cursor, 5);
        assert_eq!(bar.panel.sel_anchor, Some(0));
        assert!(!bar.panel.show_replace);
        assert!(FindBar::open(true, String::new()).panel.show_replace);
    }
}
