//! Classic drop-down menu bar, built from GridCraft's own command
//! catalog (`gridcraft_engine::catalog::CATALOG`) rather than a
//! hand-written list — one [`MenuDef`] per ribbon tab, grouped and
//! labelled exactly as [`crate::palette`] already classifies them, so
//! the menu can never offer a command this build can't actually run.
//!
//! Scoped to the tabs a terminal spreadsheet session reaches for first;
//! [`TABS`] is the only place that needs editing to surface more of the
//! catalog later. `File` gets one item with no catalog counterpart —
//! "Quit" — appended by [`menus`] itself.

use gridcraft_engine::Session;
use quadraui::{ContextMenuItem, MenuDef, StyledText, WidgetId};

use crate::palette::{self, Entry};

/// Ribbon tabs surfaced here, in display order. Contextual tabs (Page
/// Layout, Review, Draw, Automate, Developer, and the chart/table/pivot
/// tabs that only exist once their object is selected) are left out of
/// this first pass rather than shown mostly disabled.
const TABS: &[(&str, &str)] = &[
    ("File", "&File"),
    ("Home", "&Home"),
    ("Insert", "&Insert"),
    ("Formulas", "For&mulas"),
    ("Data", "&Data"),
    ("View", "&View"),
];

/// Sentinel id for the one menu item with no catalog counterpart —
/// handled directly by `CalcApp::handle`, never passed to
/// `Session::execute`.
pub const QUIT: &str = "quadracalc.quit";

/// Build this frame's menu bar from `session`'s current classification
/// (`palette::entries`) — the same enabled/disabled-with-reason logic
/// the command palette already uses, so e.g. "Undo" greys out the
/// moment there's nothing left to undo.
pub fn menus(session: &Session) -> Vec<MenuDef> {
    let entries = palette::entries(session);
    let mut menus: Vec<MenuDef> = TABS
        .iter()
        .map(|&(tab, label)| MenuDef {
            id: WidgetId::new(format!("menu:{}", tab.to_lowercase())),
            label: label.to_string(),
            disabled: false,
            items: tab_items(&entries, tab),
        })
        .collect();

    if let Some(file) = menus.first_mut() {
        file.items.push(separator());
        file.items.push(action(QUIT, "Quit"));
    }
    menus
}

/// One tab's items, catalog order, with a separator between groups.
fn tab_items(entries: &[Entry], tab: &str) -> Vec<ContextMenuItem> {
    let mut items = Vec::new();
    let mut last_group: Option<&str> = None;
    for e in entries.iter().filter(|e| e.tab == tab) {
        if last_group.is_some_and(|g| g != e.group) {
            items.push(separator());
        }
        last_group = Some(e.group);

        let mut item = action(e.id, e.label);
        if e.id == "edit.paste" {
            // `palette::entries` disables this — correctly, for the
            // palette: classification has no `Backend` to read the
            // system clipboard from, and neither does a palette run
            // (`CalcApp::execute(id, json!({}))`, no params). The menu
            // dispatch path is different: `CalcApp::handle` special-cases
            // this id and calls `clipboard::paste_from_system`, which
            // does have a `Backend` and supplies `at`/`text` itself —
            // see that function's own doc. So this one item overrides
            // the generic classification instead of inheriting it.
        } else if !e.enabled {
            item.disabled = true;
            item.detail = e.disabled_reason.as_deref().map(StyledText::plain);
        }
        items.push(item);
    }
    items
}

fn action(id: &str, label: &str) -> ContextMenuItem {
    ContextMenuItem {
        id: Some(WidgetId::new(id)),
        label: StyledText::plain(label),
        ..Default::default()
    }
}

fn separator() -> ContextMenuItem {
    ContextMenuItem::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_menu_ends_in_a_separator_then_quit() {
        let mut session = Session::new();
        session.new_workbook();
        let file = &menus(&session)[0];
        assert_eq!(file.label, "&File");
        let last = file.items.last().unwrap();
        assert_eq!(last.id, Some(WidgetId::new(QUIT)));
        let separator = &file.items[file.items.len() - 2];
        assert_eq!(separator.id, None);
    }

    #[test]
    fn edit_paste_is_enabled_despite_the_palettes_own_classification() {
        let mut session = Session::new();
        session.new_workbook();
        let home = menus(&session)
            .into_iter()
            .find(|m| m.label == "&Home")
            .unwrap();
        let paste = home
            .items
            .iter()
            .find(|i| i.id == Some(WidgetId::new("edit.paste")))
            .unwrap();
        assert!(!paste.disabled);
    }

    #[test]
    fn every_tab_is_non_empty_on_a_fresh_workbook() {
        let mut session = Session::new();
        session.new_workbook();
        for menu in menus(&session) {
            assert!(!menu.items.is_empty(), "{} has no items", menu.label);
        }
    }
}
