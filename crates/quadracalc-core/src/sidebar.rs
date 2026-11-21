//! Right-hand sidebar: GridCraft's own Name Manager list
//! (`formulas.nameManager` — `gridcraft_engine::cmd::formulas`), built
//! on quadraui's `SidebarPanel` + `ListView` primitives. Toggled with
//! Ctrl+F3 (Cmd+F3 on macOS) — the same shortcut GridCraft's own
//! catalog already assigns to `formulas.nameManager` — rather than
//! running that query command as a one-shot through the palette the
//! way every other catalog entry does: a query command's own return
//! value has nowhere to go today (`CommandPalette`'s `Run` outcome
//! just discards it), and a persistent panel is what the issue this
//! module exists for actually asked for. Closed by default, right-
//! hand side (Excel's own Pivot-Fields convention, matching
//! GridCraft's own screenshots), no activity-bar icon rail — one
//! panel, not a whole IDE shell.
//!
//! The sheet outline the issue names alongside this list has nothing
//! in this crate to render yet ("once one's worth showing" — the
//! issue's own words), so there's nothing here for it either.
//!
//! This module only builds the declarative `SidebarPanel`/`ListView`
//! values and the small open/selected state they're built from —
//! `app.rs` is the only place that paints them (through
//! `ScreenLayout`, same as every other surface) or hit-tests them.

use gridcraft_engine::Session;
use quadraui::{
    Color, Decoration, ListItem, ListView, SidebarPanel, StyledText, Toolbar, ToolbarButton,
    WidgetId,
};
use serde_json::{Value, json};

/// Stable id for the close button in the sidebar's own toolbar —
/// compared against in `app.rs`'s hit-test match, so it has to be a
/// `const` both sides agree on rather than a string literal repeated
/// in two places.
const CLOSE_BUTTON_ID: &str = "sidebar:close";

/// One row of GridCraft's own `formulas.nameManager` query result: a
/// defined name, what it refers to, its scope ("Workbook" or a sheet
/// name), and its current value.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NameEntry {
    pub name: String,
    pub refers_to: String,
    pub scope: String,
    pub value: String,
}

/// Open/closed + row-selection state for the sidebar. Lives on
/// `CalcApp` directly, the same way `CalcApp::formula_focused` does —
/// small enough that a dedicated file-backed struct would be more
/// ceremony than the state itself.
#[derive(Debug, Default)]
pub struct Sidebar {
    open: bool,
    selected: Option<usize>,
}

impl Sidebar {
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Ctrl+F3 / Cmd+F3 — see this module's own doc for why that
    /// shortcut drives a toggle here instead of running
    /// `formulas.nameManager` as a query.
    pub fn toggle(&mut self) {
        self.open = !self.open;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Select row `idx`. `len` is the current name list's length — a
    /// `ListViewHit::Item` for a row the engine's own list has since
    /// shrunk past (a name deleted through the palette while the
    /// sidebar was open and stale) is silently ignored rather than
    /// recording a selection nothing in the list matches.
    pub fn select(&mut self, idx: usize, len: usize) {
        if idx < len {
            self.selected = Some(idx);
        }
    }
}

/// `formulas.nameManager`: every defined name visible from the active
/// sheet, in the engine's own order. An engine error — there is no
/// open workbook, though that can't happen once `CalcApp::new` always
/// seeds one — returns an empty list rather than surfacing it through
/// `CalcApp::error`, since this is a read-only background refresh, not
/// a user-initiated action; an empty sidebar is the same honest
/// "nothing to show" an empty workbook would also produce.
pub fn fetch_names(session: &mut Session) -> Vec<NameEntry> {
    let Ok(rows) = session.execute("formulas.nameManager", json!({})) else {
        return Vec::new();
    };
    let Some(rows) = rows.as_array() else {
        return Vec::new();
    };
    rows.iter().map(entry_from_json).collect()
}

fn entry_from_json(row: &Value) -> NameEntry {
    let field = |key: &str| {
        row.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    NameEntry {
        name: field("name"),
        refers_to: field("refersTo"),
        scope: field("scope"),
        value: format_value(row.get("value").unwrap_or(&Value::Null)),
    }
}

/// `formulas.nameManager`'s own `value` field — one of
/// `gridcraft_engine::cmd::inspect::value_json`'s shapes: null, a
/// number, a string, a bool, `{"error": ...}`, or a nested array for a
/// multi-cell name. Good enough for a sidebar preview (it doesn't
/// carry the cell's own number formatting the way `CalcApp::cell_text`
/// does) — not a replacement for selecting the range itself.
fn format_value(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(b) => {
            if *b {
                "TRUE".into()
            } else {
                "FALSE".into()
            }
        }
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Object(obj) => obj
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("#ERR")
            .to_string(),
        Value::Array(_) => "{…}".into(),
    }
}

/// The sidebar's own id — only ever read through `toolbar_panel` below,
/// which is the one place `app.rs` gets it from.
const PANEL_ID: &str = "sidebar:panel";

/// `WidgetId` for the toolbar's close button — `app.rs`'s hit-test
/// match compares against this, not the string constant directly, so
/// a typo in either place is a compile error instead of a silent
/// click that never closes anything.
pub fn close_button_id() -> WidgetId {
    WidgetId::new(CLOSE_BUTTON_ID)
}

/// The sidebar's header toolbar: a title, the name count, and a close
/// button — mouse-only parity with the Ctrl+F3 toggle, since a
/// toolbar slot is already reserved for this panel and quadraui's
/// `SidebarPanel` primitive already knows how to paint one.
pub fn toolbar_panel(name_count: usize) -> SidebarPanel {
    let count_label = format!(
        " {name_count} name{} ",
        if name_count == 1 { "" } else { "s" }
    );
    SidebarPanel {
        id: WidgetId::new(PANEL_ID),
        toolbar: Some(
            Toolbar::new(WidgetId::new("sidebar:toolbar")).with_buttons(vec![
                ToolbarButton::Label {
                    text: " Name Manager ".into(),
                    fg: None,
                },
                ToolbarButton::Spacer,
                ToolbarButton::Label {
                    text: count_label,
                    fg: Some(Color::rgb(160, 160, 160)),
                },
                ToolbarButton::Separator,
                ToolbarButton::Action {
                    spec: None,
                    id: close_button_id(),
                    label: String::new(),
                    icon: Some("×".into()),
                    key_hint: None,
                    enabled: true,
                    is_active: false,
                    tooltip: "Close (Ctrl+F3)".into(),
                },
            ]),
        ),
        toolbar_height: None,
    }
}

/// The sidebar's content: one row per [`NameEntry`] — `refers_to` as
/// the row's right-aligned detail text (what Excel's own Name Manager
/// shows in its "Refers To" column), `value` folded into the primary
/// text since a flat `ListView` row has no third column.
pub fn list_view(names: &[NameEntry], selected: Option<usize>) -> ListView {
    let items = names
        .iter()
        .map(|n| {
            let label = if n.value.is_empty() {
                n.name.clone()
            } else {
                format!("{}  =  {}", n.name, n.value)
            };
            ListItem {
                text: StyledText::plain(label),
                icon: None,
                detail: Some(StyledText::plain(n.refers_to.clone())),
                decoration: Decoration::default(),
            }
        })
        .collect();
    ListView {
        id: WidgetId::new("sidebar:names"),
        title: None,
        items,
        selected_idx: selected.unwrap_or(0),
        scroll_offset: 0,
        // quadraui's list rasteriser gates both the selection
        // background and the `▶` prefix on `has_focus` (see
        // `quadraui::tui::list`'s own `is_selected` check) — only
        // paint a row as selected once there actually is a selection,
        // same as `CommandPalette::palette`'s `has_focus: true`.
        has_focus: selected.is_some(),
        bordered: false,
        h_scroll: 0,
        max_content_width: None,
        show_v_scrollbar: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, refers_to: &str, scope: &str, value: Value) -> Value {
        json!({"name": name, "refersTo": refers_to, "scope": scope, "value": value})
    }

    #[test]
    fn entry_from_json_reads_every_field() {
        let entry = entry_from_json(&row("Rate", "=Sheet1!$A$1", "Workbook", json!(0.07)));
        assert_eq!(entry.name, "Rate");
        assert_eq!(entry.refers_to, "=Sheet1!$A$1");
        assert_eq!(entry.scope, "Workbook");
        assert_eq!(entry.value, "0.07");
    }

    #[test]
    fn format_value_covers_every_name_manager_shape() {
        assert_eq!(format_value(&Value::Null), "");
        assert_eq!(format_value(&json!(true)), "TRUE");
        assert_eq!(format_value(&json!(false)), "FALSE");
        assert_eq!(format_value(&json!("Hosting")), "Hosting");
        assert_eq!(format_value(&json!(1200.5)), "1200.5");
        assert_eq!(format_value(&json!({"error": "#REF!"})), "#REF!");
        assert_eq!(format_value(&json!([[1, 2]])), "{…}");
    }

    #[test]
    fn sidebar_starts_closed_with_no_selection() {
        let sidebar = Sidebar::default();
        assert!(!sidebar.is_open());
        assert_eq!(sidebar.selected(), None);
    }

    #[test]
    fn toggle_flips_open_state_and_close_forces_it_shut() {
        let mut sidebar = Sidebar::default();
        sidebar.toggle();
        assert!(sidebar.is_open());
        sidebar.toggle();
        assert!(!sidebar.is_open());
        sidebar.toggle();
        sidebar.close();
        assert!(!sidebar.is_open());
    }

    #[test]
    fn select_ignores_an_index_past_the_current_list_length() {
        let mut sidebar = Sidebar::default();
        sidebar.select(5, 3);
        assert_eq!(sidebar.selected(), None);
        sidebar.select(1, 3);
        assert_eq!(sidebar.selected(), Some(1));
    }

    #[test]
    fn toolbar_panel_reports_singular_and_plural_counts() {
        let singular = toolbar_panel(1);
        let plural = toolbar_panel(2);
        let label_text = |panel: &SidebarPanel| -> String {
            panel
                .toolbar
                .as_ref()
                .unwrap()
                .buttons
                .iter()
                .filter_map(|b| match b {
                    ToolbarButton::Label { text, .. } => Some(text.clone()),
                    _ => None,
                })
                .nth(1)
                .unwrap()
        };
        assert_eq!(label_text(&singular), " 1 name ");
        assert_eq!(label_text(&plural), " 2 names ");
    }

    #[test]
    fn list_view_folds_value_into_the_label_and_keeps_refers_to_as_detail() {
        let names = vec![NameEntry {
            name: "Rate".into(),
            refers_to: "=Sheet1!$A$1".into(),
            scope: "Workbook".into(),
            value: "0.07".into(),
        }];
        let list = list_view(&names, Some(0));
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].text.spans[0].text, "Rate  =  0.07");
        assert_eq!(
            list.items[0].detail.as_ref().unwrap().spans[0].text,
            "=Sheet1!$A$1"
        );
        assert_eq!(list.selected_idx, 0);
    }

    #[test]
    fn list_view_falls_back_to_the_bare_name_when_value_is_empty() {
        let names = vec![NameEntry {
            name: "Empty".into(),
            refers_to: "=Sheet1!$A$1".into(),
            scope: "Workbook".into(),
            value: String::new(),
        }];
        let list = list_view(&names, None);
        assert_eq!(list.items[0].text.spans[0].text, "Empty");
    }
}
