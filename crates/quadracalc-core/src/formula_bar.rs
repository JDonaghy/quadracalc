//! The formula bar: a [`TextField`] mirroring the cursor cell's raw
//! input, stacked above the [`CellGrid`] band. Mirrors [`CellGrid::editing`]
//! (quadraui#2918/#2920) so a formula bar edit and the grid's own in-cell
//! editor are always the same in-progress edit — see
//! `quadraui`'s `examples/common/cell_grid_demo.rs` for the reference
//! shape this mirrors.
//!
//! Split out from `app.rs` so the band geometry
//! ([`bands`]) is a pure function of plain floats, unit-testable with no
//! `Backend` at all — the same reasoning
//! `CalcApp::resolve_cell_size` already follows.

use quadraui::{CellGrid, Rect, TextField, WidgetId};

/// The three stacked bands this app paints into, top to bottom: the
/// formula bar, the `CellGrid` body, and the status bar. A pure function
/// of the viewport and line height, called identically to paint
/// (`render`) and to hit-test (`handle`) — computing it once per call
/// site instead of caching it keeps the two from ever drifting apart.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bands {
    pub formula: Rect,
    pub grid: Rect,
    pub status: Rect,
}

/// Resolve [`Bands`] for a `width`×`height` viewport whose one-line
/// chrome height is `line_height` — the formula bar and status bar each
/// get one chrome-sized row (same `1.5x` bump above a sub-1.5 TUI line
/// height `CalcApp` already used for the status bar alone), and the
/// `CellGrid` body gets whatever's left.
pub fn bands(width: f32, height: f32, line_height: f32) -> Bands {
    let lh = line_height.max(1.0);
    let bar_h = if lh > 1.5 { lh * 1.5 } else { lh };
    let formula_h = bar_h.min(height);
    let status_h = bar_h.min((height - formula_h).max(0.0));
    let grid_h = (height - formula_h - status_h).max(0.0);
    Bands {
        formula: Rect::new(0.0, 0.0, width, formula_h),
        grid: Rect::new(0.0, formula_h, width, grid_h),
        status: Rect::new(0.0, formula_h + grid_h, width, status_h),
    }
}

/// Build this frame's formula-bar [`TextField`]: seeded with `raw_input`
/// (the cursor cell's current raw text, from `cell.get`'s `input`
/// field), then overwritten with [`CellGrid::editing`]'s own
/// `text`/`cursor`/`selection_anchor` whenever an edit is active — so
/// the field shows the live edit while typing (through either the grid
/// or the bar itself) and the committed raw text otherwise. `focused` is
/// `true` only while the formula bar itself (not the grid) owns the
/// caret — see `CalcApp::formula_focused`'s own doc.
pub fn build_field(raw_input: &str, grid: &CellGrid, focused: bool) -> TextField {
    let mut field = TextField::new(WidgetId::new("quadracalc-formula-bar"))
        .with_placeholder("Formula")
        .with_value(raw_input)
        .with_has_focus(focused);
    match &grid.editing {
        Some(edit) => {
            field.value = edit.text.clone();
            field.cursor = Some(edit.cursor);
            field.selection_anchor = edit.selection_anchor;
        }
        None => {
            field.cursor = None;
            field.selection_anchor = None;
        }
    }
    field
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_stack_formula_then_grid_then_status_top_to_bottom() {
        let b = bands(80.0, 20.0, 1.0);
        assert_eq!(b.formula, Rect::new(0.0, 0.0, 80.0, 1.0));
        assert_eq!(b.grid, Rect::new(0.0, 1.0, 80.0, 18.0));
        assert_eq!(b.status, Rect::new(0.0, 19.0, 80.0, 1.0));
    }

    #[test]
    fn bands_scale_the_chrome_rows_above_a_tall_line_height() {
        // bar_h = 18 * 1.5 = 27; a 90-tall viewport has room for both
        // chrome rows at their full scaled height, with 36 left for the
        // grid.
        let b = bands(100.0, 90.0, 18.0);
        assert_eq!(b.formula.height, 27.0);
        assert_eq!(b.status.height, 27.0);
        assert_eq!(b.grid.height, 36.0);
    }

    #[test]
    fn bands_never_produce_a_negative_grid_height_on_a_tiny_viewport() {
        let b = bands(10.0, 1.0, 1.0);
        assert!(b.grid.height >= 0.0);
        assert!(b.status.height >= 0.0);
    }

    #[test]
    fn field_mirrors_the_cursor_cells_raw_input_when_not_editing() {
        let grid = CellGrid::new(WidgetId::new("grid"));
        let field = build_field("=SUM(A1:A5)", &grid, false);
        assert_eq!(field.value, "=SUM(A1:A5)");
        assert!(!field.has_focus);
        assert_eq!(field.cursor, None);
    }

    #[test]
    fn field_shows_the_live_edit_buffer_while_editing() {
        use quadraui::primitives::cell_grid::CellEditState;

        let mut grid = CellGrid::new(WidgetId::new("grid"));
        grid.row_count = 5;
        grid.col_count = 5;
        grid.editing = Some(CellEditState {
            text: "=OLD".to_string(),
            cursor: "=OLD".len(),
            selection_anchor: None,
            placeholder: None,
        });
        let field = build_field("=OLD", &grid, true);
        assert_eq!(field.value, "=OLD");
        assert!(field.has_focus);
        assert_eq!(field.cursor, Some("=OLD".len()));
    }
}
