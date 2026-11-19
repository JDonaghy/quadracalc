//! [`CalcApp`]: GridCraft session + view state, implemented as a quadraui
//! [`AppLogic`] so every runner (TUI, GTK, macOS, Windows) can drive it.
//!
//! Reading the engine takes `&mut Session` (everything goes through
//! `Session::execute`), but `AppLogic::render` only gets `&self`. So the app
//! refreshes a [`View`] — the visible block of cell texts plus the active
//! cell's details — after every command, and `render` paints from it.
//!
//! The grid is a `DataTable` stand-in until quadraui's CellGrid primitive
//! (quadraui#2732) lands; swapping it in changes only [`View::grid_surface`].
//! Painting goes through `ScreenLayout`, quadraui's canonical path, which is
//! also what makes the screen visible to the accessibility tree and agents.

use gridcraft_engine::Session;
use quadraui::{
    AppLogic, Backend, Color, Column, ColumnAlign, ColumnWidth, DataRow, DataTable, Key, NamedKey,
    Reaction, Rect, ScreenLayout, StatusBar, StatusBarSegment, StyledText, Surface, UiEvent,
    WidgetId,
};
use serde_json::{Value, json};

use crate::address::{Address, column_label};

/// Columns shown by the stand-in grid.
const VISIBLE_COLS: u32 = 8;
/// Rows cached per refresh; the painter shows as many as fit.
const CACHED_ROWS: u32 = 60;

pub struct CalcApp {
    session: Session,
    cursor: Address,
    top_row: u32,
    /// Rows that fit on screen, learned from the backend in `handle`.
    page_rows: u32,
    /// Last engine error, shown in the status bar until the next action.
    error: Option<String>,
    view: View,
}

/// Everything `render` needs, read from the engine after each command.
#[derive(Debug, Clone, Default)]
pub struct View {
    pub top_row: u32,
    pub cursor: Address,
    /// `cells[r][c]`: displayed text, rows from `top_row`, columns from A.
    pub cells: Vec<Vec<String>>,
    pub active_input: String,
    pub active_text: String,
    pub error: Option<String>,
}

impl CalcApp {
    /// A new, empty workbook.
    pub fn new() -> Self {
        let mut session = Session::new();
        session.new_workbook();
        let mut app = Self {
            session,
            cursor: Address::default(),
            top_row: 0,
            page_rows: 20,
            error: None,
            view: View::default(),
        };
        app.refresh();
        app
    }

    /// A workbook with a small sample, so a first launch shows a live formula.
    pub fn with_sample() -> anyhow::Result<Self> {
        let mut app = Self::new();
        for (cell, input) in [
            ("A1", "Item"),
            ("B1", "Cost"),
            ("A2", "Hosting"),
            ("B2", "$1,200.50"),
            ("A3", "Domains"),
            ("B3", "9"),
            ("A4", "Total"),
            ("B4", "=SUM(B2:B3)"),
        ] {
            app.set_cell(cell, input)?;
        }
        Ok(app)
    }

    /// Run one GridCraft command, then refresh the view. The UI never changes
    /// the workbook any other way.
    pub fn execute(&mut self, id: &str, params: Value) -> anyhow::Result<Value> {
        let result = self
            .session
            .execute(id, params)
            .map_err(|e| anyhow::anyhow!("{id}: {e}"));
        self.refresh();
        result
    }

    /// `cell.set`: enter text, a number or a formula into one cell.
    pub fn set_cell(&mut self, a1: &str, input: &str) -> anyhow::Result<()> {
        self.execute("cell.set", json!({ "cell": a1, "input": input }))?;
        Ok(())
    }

    /// The engine's displayed text for a cell (formatted value, not the formula).
    pub fn cell_text(&mut self, a1: &str) -> String {
        self.cell_field(a1, "text")
    }

    /// What the user typed into a cell: the formula for a formula cell.
    pub fn cell_input(&mut self, a1: &str) -> String {
        self.cell_field(a1, "input")
    }

    fn cell_field(&mut self, a1: &str, field: &str) -> String {
        self.session
            .execute("cell.get", json!({ "cell": a1 }))
            .ok()
            .and_then(|v| v.get(field).and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_default()
    }

    pub fn cursor(&self) -> Address {
        self.cursor
    }

    pub fn view(&self) -> &View {
        &self.view
    }

    fn move_cursor(&mut self, d_row: i64, d_col: i64) {
        let row = (self.cursor.row as i64 + d_row).max(0) as u32;
        let col = (self.cursor.col as i64 + d_col).clamp(0, VISIBLE_COLS as i64 - 1) as u32;
        self.cursor = Address::new(row, col);
        let page = self.page_rows.max(1);
        if row < self.top_row {
            self.top_row = row;
        } else if row >= self.top_row + page {
            self.top_row = row + 1 - page;
        }
        self.refresh();
    }

    /// Re-read the visible block and the active cell from the engine.
    fn refresh(&mut self) {
        let top = self.top_row;
        let cells = (top..top + CACHED_ROWS)
            .map(|r| {
                (0..VISIBLE_COLS)
                    .map(|c| self.cell_text(&Address::new(r, c).a1()))
                    .collect()
            })
            .collect();
        let a1 = self.cursor.a1();
        self.view = View {
            top_row: top,
            cursor: self.cursor,
            cells,
            active_input: self.cell_input(&a1),
            active_text: self.cell_text(&a1),
            error: self.error.clone(),
        };
    }
}

impl Default for CalcApp {
    fn default() -> Self {
        Self::new()
    }
}

impl View {
    /// The whole screen as `ScreenLayout` surfaces, back to front.
    pub fn screen(&self, width: f32, height: f32, line_height: f32) -> ScreenLayout {
        let bar_h = if line_height > 1.5 {
            line_height * 1.5
        } else {
            line_height
        };
        let grid_rect = Rect::new(0.0, 0.0, width, (height - bar_h).max(0.0));
        let mut layout = ScreenLayout::new();
        layout.push(self.grid_surface(grid_rect));
        layout.push(self.status_surface(Rect::new(0.0, grid_rect.height, width, bar_h)));
        layout
    }

    /// The stand-in grid: row numbers plus columns A…H, cursor row selected.
    fn grid_surface(&self, rect: Rect) -> Surface {
        let mut columns = vec![Column {
            title: String::new(),
            width: ColumnWidth::FixedCells(5.0),
            align: ColumnAlign::Right,
        }];
        columns.extend((0..VISIBLE_COLS).map(|c| Column {
            title: column_label(c),
            width: ColumnWidth::Flex(1.0),
            align: ColumnAlign::Left,
        }));

        let rows = self
            .cells
            .iter()
            .enumerate()
            .map(|(i, texts)| {
                let r = self.top_row + i as u32;
                let mut cells = vec![StyledText::plain((r + 1).to_string())];
                for (c, text) in texts.iter().enumerate() {
                    let here = Address::new(r, c as u32) == self.cursor;
                    cells.push(if here {
                        StyledText::colored(format!("[{text}]"), Color::rgb(255, 210, 90))
                    } else {
                        StyledText::plain(text.clone())
                    });
                }
                DataRow {
                    cells,
                    decoration: Default::default(),
                }
            })
            .collect();

        let table = DataTable {
            id: WidgetId::new("sheet"),
            columns,
            rows,
            selected_idx: Some((self.cursor.row - self.top_row) as usize),
            scroll_offset: 0,
            sort: None,
            has_focus: true,
            show_scrollbar: false,
            min_total_width: None,
            h_scroll: 0.0,
            column_overrides: Default::default(),
            footer: None,
            bordered: false,
        };
        Surface::DataTable {
            rect,
            table,
            hovered: None,
        }
    }

    fn status_surface(&self, rect: Rect) -> Surface {
        let detail = match (&self.error, self.active_input.starts_with('=')) {
            (Some(err), _) => format!(" {err} "),
            (None, true) => format!(" {}  →  {} ", self.active_input, self.active_text),
            (None, false) => format!(" {} ", self.active_input),
        };
        let bar_bg = Color::rgb(33, 66, 110);
        let bar = StatusBar {
            id: WidgetId::new("status"),
            left_segments: vec![
                StatusBarSegment {
                    text: format!(" {} ", self.cursor.a1()),
                    fg: Color::rgb(255, 255, 255),
                    bg: Color::rgb(20, 45, 80),
                    bold: true,
                    action_id: None,
                },
                StatusBarSegment {
                    text: detail,
                    fg: Color::rgb(230, 230, 230),
                    bg: bar_bg,
                    bold: false,
                    action_id: None,
                },
            ],
            right_segments: vec![StatusBarSegment {
                text: " quadracalc · GridCraft engine · q quits ".into(),
                fg: Color::rgb(200, 210, 225),
                bg: bar_bg,
                bold: false,
                action_id: None,
            }],
        };
        Surface::StatusBar {
            rect,
            bar,
            hovered: None,
            pressed: None,
        }
    }
}

impl AppLogic for CalcApp {
    type AreaId = ();

    fn render(&self, backend: &mut dyn Backend, _area: ()) {
        let vp = backend.viewport();
        let layout = self.view.screen(vp.width, vp.height, backend.line_height());
        let _ = layout.draw(backend);
    }

    fn handle(&mut self, event: UiEvent, backend: &mut dyn Backend) -> Reaction {
        let UiEvent::KeyPressed { key, .. } = event else {
            return Reaction::Continue;
        };
        let lh = backend.line_height().max(1.0);
        let bar_h = if lh > 1.5 { lh * 1.5 } else { lh };
        self.page_rows = (((backend.viewport().height - bar_h) / lh) as u32)
            .saturating_sub(1)
            .max(1);
        self.error = None;
        match key {
            Key::Char('q') | Key::Named(NamedKey::Escape) => return Reaction::Exit,
            Key::Named(NamedKey::Up) => self.move_cursor(-1, 0),
            Key::Named(NamedKey::Down) | Key::Named(NamedKey::Enter) => self.move_cursor(1, 0),
            Key::Named(NamedKey::Left) => self.move_cursor(0, -1),
            Key::Named(NamedKey::Right) | Key::Named(NamedKey::Tab) => self.move_cursor(0, 1),
            _ => return Reaction::Continue,
        }
        Reaction::Redraw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_recalculates_through_the_command_surface() {
        let mut app = CalcApp::with_sample().unwrap();
        assert_eq!(app.cell_input("B4"), "=SUM(B2:B3)");
        let before = app.cell_text("B4");
        app.set_cell("B3", "10").unwrap();
        let after = app.cell_text("B4");
        assert_ne!(before, after, "B4 should recalculate when B3 changes");
        assert!(
            after.contains("1210.5") || after.contains("1,210.50"),
            "got {after}"
        );
    }

    #[test]
    fn view_follows_commands_and_cursor() {
        let mut app = CalcApp::with_sample().unwrap();
        assert_eq!(app.view().cells[0][0], "Item");
        app.move_cursor(3, 1);
        assert_eq!(app.view().cursor, Address::new(3, 1));
        assert_eq!(app.view().active_input, "=SUM(B2:B3)");
    }

    #[test]
    fn cursor_stays_in_bounds() {
        let mut app = CalcApp::new();
        app.move_cursor(-5, -5);
        assert_eq!(app.cursor(), Address::new(0, 0));
        app.move_cursor(3, 100);
        assert_eq!(app.cursor(), Address::new(3, VISIBLE_COLS - 1));
    }
}
