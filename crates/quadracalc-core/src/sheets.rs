//! Sheet tabs: this app's quadraui [`TabBar`] view of the engine's own
//! sheet list, and the per-sheet cursor memory that view needs but the
//! engine has no concept of.
//!
//! The engine owns the sheet list itself end to end — opening a
//! workbook, `home.insertSheet`, `sheet.rename`, `sheet.activate` and
//! `sheet.next`/`sheet.previous` (`gridcraft_engine::cmd::sheet`) all go
//! through `Session::execute` exactly like every other command this app
//! runs (`CalcApp::switch_sheet` is the one caller for the last three).
//! This module only ever *reads* `Session::doc` to turn whatever the
//! engine currently holds into a [`TabBar`] — see [`tab_bar`].
//!
//! The one thing it adds on top is [`SheetCursors`]: a
//! `CellGridController`'s cursor/selection/scroll is this app's own
//! state, not the engine's (the engine's own per-sheet `sheet_selections`
//! is a different thing — the selection formulas resolve against, not
//! the UI's own grid widget), so switching sheets and back has to save
//! and restore it here.

use gridcraft_engine::Session;
use quadraui::{CellGridController, TabBar, TabItem, WidgetId};

/// Sheet names in workbook order, exactly as the engine has them — empty
/// with no document open yet.
pub fn sheet_names(session: &Session) -> Vec<String> {
    match session.doc() {
        Ok(doc) => doc
            .wb
            .sheets
            .iter()
            .map(|sheet| sheet.name.clone())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// The active sheet's index into [`sheet_names`] — `0` with no document
/// open yet (`Workbook::active_sheet`'s own default).
pub fn active_sheet_index(session: &Session) -> usize {
    session.doc().map_or(0, |doc| doc.wb.active_sheet())
}

/// The active sheet's name, or `None` with no document open yet — the
/// key [`SheetCursors`] remembers a [`CursorMemory`] under.
pub fn active_sheet_name(session: &Session) -> Option<String> {
    sheet_names(session)
        .into_iter()
        .nth(active_sheet_index(session))
}

/// This frame's sheet-tabs [`TabBar`], built straight from `session`'s
/// active workbook. Never cached — any `home.insertSheet`/
/// `sheet.rename`/`sheet.activate` makes a stashed copy stale the moment
/// it runs, so `CalcApp::refresh` just rebuilds this every frame the way
/// it rebuilds the rest of `View`. No close button: `home.deleteSheet`
/// exists, but it isn't wired to this bar yet, so showing a `×` nothing
/// responds to would be misleading.
///
/// `scroll_offset` is the caller's own stored value (`CalcApp::tab_scroll`)
/// — per `TabBar`'s backend contract (`quadraui::TabBar::layout`'s doc),
/// the caller owns scroll and must feed back whatever
/// `Backend::tab_bar_layout`'s `correct_scroll_offset` resolves it to
/// (see `CalcApp::refresh`'s own comment for why that method rather than
/// `Backend::resolve_tab_bar_layout`), or an overflowing bar permanently
/// drops the active tab off the strip the moment the stored offset goes
/// stale.
pub fn tab_bar(session: &Session, scroll_offset: usize) -> TabBar {
    let names = sheet_names(session);
    let active = active_sheet_index(session);
    TabBar {
        id: WidgetId::new("quadracalc-sheet-tabs"),
        tabs: names
            .iter()
            .enumerate()
            .map(|(i, name)| TabItem {
                label: format!(" {name} "),
                is_active: i == active,
                is_closable: false,
                ..Default::default()
            })
            .collect(),
        scroll_offset,
        right_segments: vec![],
        active_accent: None,
        show_tab_close: false,
        compact: false,
    }
}

/// A [`CellGridController`]'s cursor/selection/scroll — the whole slice
/// of its state that's specific to one sheet, snapshotted on the way out
/// of a sheet and written back on the way into one.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CursorMemory {
    cursor: (usize, usize),
    anchor: Option<(usize, usize)>,
    row_scroll: usize,
    col_scroll: usize,
}

impl CursorMemory {
    /// Snapshot `grid`'s current state.
    pub fn capture(grid: &CellGridController) -> Self {
        Self {
            cursor: grid.cursor(),
            anchor: grid.selection_anchor(),
            row_scroll: grid.row_scroll(),
            col_scroll: grid.col_scroll(),
        }
    }

    /// Write this state back onto `grid`.
    pub fn restore(&self, grid: &mut CellGridController) {
        grid.set_selection(self.cursor, self.anchor);
        grid.set_scroll(self.row_scroll, self.col_scroll);
    }
}

/// Per-sheet [`CursorMemory`], keyed by sheet name rather than index —
/// a `home.insertSheet` elsewhere in the list shifts every later sheet's
/// index, but the engine keeps sheet names unique, so a name-keyed
/// lookup can't land on the wrong sheet the way an index-keyed one
/// could. A sheet with no entry (never visited yet this session, or
/// renamed since) gets [`CursorMemory::default`]: cursor at the origin,
/// no selection, no scroll — the same place a brand new
/// `CellGridController` starts.
#[derive(Debug, Default)]
pub struct SheetCursors(std::collections::HashMap<String, CursorMemory>);

impl SheetCursors {
    /// Remember `state` for `sheet`, replacing whatever was there.
    pub fn save(&mut self, sheet: &str, state: CursorMemory) {
        self.0.insert(sheet.to_string(), state);
    }

    /// `sheet`'s remembered state, or the default origin/no-scroll state
    /// if it has none yet.
    pub fn get(&self, sheet: &str) -> CursorMemory {
        self.0.get(sheet).copied().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unvisited_sheet_defaults_to_the_origin_with_no_scroll() {
        let cursors = SheetCursors::default();
        assert_eq!(cursors.get("Sheet2"), CursorMemory::default());
    }

    #[test]
    fn a_saved_sheets_state_comes_back_unchanged_and_others_are_unaffected() {
        let mut cursors = SheetCursors::default();
        let state = CursorMemory {
            cursor: (4, 2),
            anchor: Some((4, 0)),
            row_scroll: 3,
            col_scroll: 1,
        };
        cursors.save("Sheet1", state);
        assert_eq!(cursors.get("Sheet1"), state);
        assert_eq!(cursors.get("Sheet2"), CursorMemory::default());
    }

    #[test]
    fn saving_again_replaces_the_previous_state() {
        let mut cursors = SheetCursors::default();
        cursors.save(
            "Sheet1",
            CursorMemory {
                cursor: (1, 1),
                ..Default::default()
            },
        );
        cursors.save(
            "Sheet1",
            CursorMemory {
                cursor: (9, 9),
                ..Default::default()
            },
        );
        assert_eq!(cursors.get("Sheet1").cursor, (9, 9));
    }
}
