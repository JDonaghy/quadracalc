//! The active sheet's own view settings, mapped onto [`CellGrid`]'s
//! presentation fields: frozen panes (`view.freezePanes`/
//! `view.freezeTopRow`/`view.freezeFirstColumn`), gridlines
//! (`view.gridlines` → [`CellGrid::column_separators`]) and column widths
//! (`home.columnWidth`/`home.autofitColumnWidth`).
//!
//! These live in the workbook — they're saved with the file and set by
//! the engine's own View/Home commands — so the grid follows them rather
//! than keeping a second copy here. Reading them is a plain read of
//! `Session::doc`, the same "structural view data" exception
//! `crate::sheets` and `crate::chart_view` already document; every change
//! still goes through `Session::execute`.
//!
//! Zebra striping ([`CellGrid::striped`]) has no workbook equivalent, so
//! it's an app-level setting on [`crate::CalcApp`] instead.

use std::collections::BTreeMap;

use gridcraft_engine::Session;
use quadraui::CellGrid;

/// The view settings of one sheet, already converted to [`CellGrid`]'s
/// own units.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SheetView {
    pub frozen_rows: usize,
    pub frozen_cols: usize,
    pub gridlines: bool,
    /// Explicit column widths, keyed by column index, in the same units
    /// as [`CellGrid::default_col_width`]. Columns at the sheet's default
    /// width are absent, so they follow the grid's own default.
    pub col_widths: BTreeMap<usize, f32>,
}

impl SheetView {
    /// The active sheet's settings, or `None` with no document open.
    /// `default_col_width` is the grid's own default column width; an
    /// explicit sheet width is scaled by its ratio to the sheet's own
    /// default, so a column at the default width paints exactly as wide
    /// as an untouched one.
    pub fn read(session: &Session, default_col_width: f32) -> Option<Self> {
        let doc = session.doc().ok()?;
        let sheet = doc.wb.active()?;
        let (frozen_rows, frozen_cols) = sheet.freeze.unwrap_or((0, 0));
        let col_widths = sheet
            .cols
            .iter()
            .filter(|(_, info)| !info.hidden)
            .filter_map(|(&col, info)| {
                let px = info.size?;
                Some((
                    col as usize,
                    scale_width(px, sheet.default_col_width, default_col_width),
                ))
            })
            .collect();
        Some(Self {
            frozen_rows: frozen_rows as usize,
            frozen_cols: frozen_cols as usize,
            gridlines: sheet.show_gridlines,
            col_widths,
        })
    }

    /// Write these settings onto `grid`.
    pub fn apply(&self, grid: &mut CellGrid) {
        grid.frozen_rows = self.frozen_rows.min(grid.row_count);
        grid.frozen_cols = self.frozen_cols.min(grid.col_count);
        grid.column_separators = self.gridlines;
        grid.col_widths = self.col_widths.clone();
    }
}

/// `sheet_px` (a sheet column width, in the engine's 96-dpi pixels)
/// rescaled so `sheet_default_px` maps onto `grid_default`, rounded to a
/// whole unit and never narrower than three — room for one character
/// plus the separator and a gap on a terminal.
pub fn scale_width(sheet_px: f32, sheet_default_px: f32, grid_default: f32) -> f32 {
    if sheet_default_px <= 0.0 {
        return grid_default;
    }
    (sheet_px / sheet_default_px * grid_default)
        .round()
        .max(3.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_default_width_column_scales_to_the_grids_default() {
        assert_eq!(scale_width(64.0, 64.0, 8.0), 8.0);
        assert_eq!(scale_width(128.0, 64.0, 8.0), 16.0);
        assert_eq!(scale_width(128.0, 64.0, 70.0), 140.0);
    }

    #[test]
    fn a_tiny_width_still_leaves_room_for_a_character() {
        assert_eq!(scale_width(4.0, 64.0, 8.0), 3.0);
    }

    #[test]
    fn reads_freeze_gridlines_and_widths_through_the_engines_own_commands() {
        let mut s = Session::new();
        s.new_workbook();
        let fresh = SheetView::read(&s, 8.0).unwrap();
        assert_eq!((fresh.frozen_rows, fresh.frozen_cols), (0, 0));
        assert!(fresh.gridlines, "a new sheet shows gridlines");
        assert!(fresh.col_widths.is_empty());

        s.execute("view.freezePanes", json!({"cell": "B2"}))
            .unwrap();
        s.execute("view.gridlines", json!({"on": false})).unwrap();
        s.execute("home.columnWidth", json!({"cols": "C:C", "width": 128}))
            .unwrap();
        let v = SheetView::read(&s, 8.0).unwrap();
        assert_eq!((v.frozen_rows, v.frozen_cols), (1, 1));
        assert!(!v.gridlines);
        assert_eq!(v.col_widths.get(&2), Some(&16.0));

        let mut grid = CellGrid::new(quadraui::WidgetId::new("g"));
        grid.row_count = 10;
        grid.col_count = 10;
        v.apply(&mut grid);
        assert_eq!((grid.frozen_rows, grid.frozen_cols), (1, 1));
        assert!(!grid.column_separators);
        assert_eq!(grid.col_widths.get(&2), Some(&16.0));
    }
}
