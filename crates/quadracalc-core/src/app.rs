//! [`CalcApp`]: GridCraft session + view state, implemented as a quadraui
//! [`AppLogic`] so every runner (TUI, GTK, macOS, Windows) can drive it.
//!
//! Reading the engine takes `&mut Session` (everything goes through
//! `Session::execute`), but `AppLogic::render` only gets `&self`. So the app
//! refreshes a [`View`] — the visible block of cell texts (already built
//! into a quadraui [`CellGrid`]) plus the active cell's details — whenever
//! the viewport or the cursor could have moved (`setup` and every
//! `handle`), and `render` only reads [`CalcApp::view`]. Painting goes
//! through `ScreenLayout`, quadraui's canonical path, which is also what
//! makes the screen visible to the accessibility tree and agents.
//!
//! The grid itself is a [`CellGridController`]-driven [`CellGrid`] —
//! quadraui's spreadsheet primitive (quadraui#2732). The controller owns
//! cursor/selection/scroll/frozen-pane state; this app owns cell content
//! and reads only the window [`CellGridController::layout`] reports is on
//! screen, never the whole sheet.

use gridcraft_engine::Session;
use quadraui::{
    AppLogic, Backend, Cell, CellGrid, CellGridController, CellGridControllerEvent, Color, Key,
    NamedKey, Reaction, Rect, ScreenLayout, StatusBar, StatusBarSegment, Surface, UiEvent,
    WidgetId,
};
use serde_json::{Value, json};

use crate::address::Address;

/// Rows in the demo sheet — far more than any terminal or window shows at
/// once, so scrolling to a distant row exercises `CellGrid`'s own
/// two-axis virtualisation rather than anything this app does.
const ROW_COUNT: usize = 10_000;
/// Columns in the demo sheet, likewise wider than any viewport.
const COL_COUNT: usize = 50;

pub struct CalcApp {
    session: Session,
    grid: CellGridController,
    /// Last engine error, shown in the status bar until the next action.
    error: Option<String>,
    view: View,
}

/// Everything `render` needs, rebuilt after every command or cursor move.
#[derive(Debug, Clone, Default)]
pub struct View {
    /// The fully-built grid for this frame — geometry plus the windowed
    /// cell text the engine supplied for exactly the rows/columns
    /// `CellGridController::layout` reported as visible. `None` only
    /// before the first `setup`/`handle` call ever runs.
    pub grid: Option<CellGrid>,
    /// The cursor's address, or `top:bottom` range notation for a
    /// multi-cell selection.
    pub selection_label: String,
    /// What the user typed into the cursor cell: the formula for a
    /// formula cell.
    pub active_input: String,
    /// The engine's displayed text for the cursor cell.
    pub active_text: String,
    pub error: Option<String>,
}

impl CalcApp {
    /// A new, empty workbook.
    pub fn new() -> Self {
        let mut session = Session::new();
        session.new_workbook();
        let mut grid = CellGridController::new("quadracalc-grid");
        grid.set_dimensions(ROW_COUNT, COL_COUNT);
        Self {
            session,
            grid,
            error: None,
            view: View::default(),
        }
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

    /// Run one GridCraft command. The UI never changes the workbook any
    /// other way. Callers that need the view refreshed afterwards (every
    /// production caller) go through `handle`/`setup`'s own `refresh`
    /// instead, since only those have a `backend` to resolve the visible
    /// window with; this is also used directly by tests that just want the
    /// engine's own result and check `cell_text`/`cell_input` themselves.
    pub fn execute(&mut self, id: &str, params: Value) -> anyhow::Result<Value> {
        self.session
            .execute(id, params)
            .map_err(|e| anyhow::anyhow!("{id}: {e}"))
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
        let (row, col) = self.grid.cursor();
        Address::new(row as u32, col as u32)
    }

    pub fn view(&self) -> &View {
        &self.view
    }

    /// The grid body's rect — the viewport minus the status bar.
    fn grid_rect(backend: &dyn Backend) -> Rect {
        let vp = backend.viewport();
        let lh = backend.line_height().max(1.0);
        let bar_h = if lh > 1.5 { lh * 1.5 } else { lh };
        Rect::new(0.0, 0.0, vp.width, (vp.height - bar_h).max(0.0))
    }

    /// `(row_height, col_width)` to hand `CellGridController::set_sizing`,
    /// derived from the backend's own chrome metrics. `CellGrid`'s
    /// `default_row_height`/`default_col_width` are raw surface-native
    /// units — cells on TUI, pixels everywhere else — so a pixel backend
    /// must pick real pixel sizes itself rather than inherit the
    /// TUI-shaped `1.0`/`8.0` constructor defaults, which would make
    /// GTK/macOS/Windows paint 1px rows and fetch the whole sheet as the
    /// "visible" window. Mirrors `quadraui-gallery`'s own
    /// `CellGridDemo::cell_size`.
    fn cell_size(backend: &dyn Backend) -> (f32, f32) {
        Self::resolve_cell_size(backend.chrome_line_height(), backend.char_width())
    }

    /// The pure formula behind [`Self::cell_size`], split out so it's
    /// unit-testable on plain floats — every `Backend` this workspace can
    /// build against in a headless test run reports a `chrome_line_height`
    /// of `1.0` (the TUI shape), so a test going through `&dyn Backend`
    /// alone could never reach the pixel-sizing branch a native window
    /// actually takes.
    fn resolve_cell_size(chrome_line_height: f32, char_width: f32) -> (f32, f32) {
        if chrome_line_height <= 1.0 {
            (1.0, 8.0)
        } else {
            (chrome_line_height, char_width * 10.0)
        }
    }

    /// Re-read exactly the visible window from the engine and rebuild
    /// `self.view` — called from `setup` and every `handle`, which are the
    /// only places with a `backend` to resolve
    /// `CellGridController::layout` against.
    fn refresh(&mut self, backend: &dyn Backend) {
        let (row_height, col_width) = Self::cell_size(backend);
        self.grid.set_sizing(row_height, col_width);
        let rect = Self::grid_rect(backend);
        let layout = self.grid.layout(backend, rect).layout;
        let cells: Vec<Vec<Cell>> = layout
            .rows
            .iter()
            .map(|r| {
                layout
                    .cols
                    .iter()
                    .map(|c| {
                        Cell::plain(
                            self.cell_text(&Address::new(r.index as u32, c.index as u32).a1()),
                        )
                    })
                    .collect()
            })
            .collect();
        let grid = self.grid.grid(cells);

        let a1 = self.cursor().a1();
        let (top, left, bottom, right) = self.grid.selection_bounds();
        let selection_label = if top == bottom && left == right {
            a1.clone()
        } else {
            format!(
                "{}:{}",
                Address::new(top as u32, left as u32).a1(),
                Address::new(bottom as u32, right as u32).a1()
            )
        };

        self.view = View {
            grid: Some(grid),
            selection_label,
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
    fn screen(&self, width: f32, height: f32, bar_h: f32) -> ScreenLayout {
        let mut layout = ScreenLayout::new();
        let grid_rect = Rect::new(0.0, 0.0, width, (height - bar_h).max(0.0));
        if let Some(grid) = &self.grid {
            layout.push(Surface::CellGrid {
                rect: grid_rect,
                grid: grid.clone(),
            });
        }
        layout.push(self.status_surface(Rect::new(0.0, grid_rect.height, width, bar_h)));
        layout
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
                    text: format!(" {} ", self.selection_label),
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

    fn setup(&mut self, backend: &mut dyn Backend) {
        self.refresh(backend);
    }

    fn render(&self, backend: &mut dyn Backend, _area: ()) {
        let vp = backend.viewport();
        let lh = backend.line_height().max(1.0);
        let bar_h = if lh > 1.5 { lh * 1.5 } else { lh };
        let layout = self.view.screen(vp.width, vp.height, bar_h);
        let _ = layout.draw(backend);
    }

    fn handle(&mut self, event: UiEvent, backend: &mut dyn Backend) -> Reaction {
        self.error = None;
        // `q`/Escape quit only outside an active edit — while editing, `q`
        // is a character to insert and Escape cancels the edit instead
        // (`CellGridController::handle` routes both to the edit buffer
        // when `is_editing`, same as quadraui's own `cell_grid_demo`).
        if !self.grid.is_editing()
            && let UiEvent::KeyPressed { key, .. } = &event
            && matches!(key, Key::Char('q') | Key::Named(NamedKey::Escape))
        {
            return Reaction::Exit;
        }
        // `CellGridController::handle` has no arm for `WindowResized` (it
        // owns no viewport-size state of its own, only scroll/cursor), so
        // it falls into `Ignored` and `refresh` below would never run.
        // Without this, `render` keeps painting the cached `View::grid`,
        // whose loaded window was sized for the old viewport — enlarging
        // the terminal or window leaves the newly revealed rows/columns
        // blank until the next cursor move. Refresh explicitly instead.
        if let UiEvent::WindowResized { .. } = &event {
            self.refresh(backend);
            return Reaction::Redraw;
        }
        let rect = Self::grid_rect(backend);
        let event = self.grid.handle(&event, backend, rect);
        let reaction = self.apply_controller_event(event);
        if matches!(reaction, Reaction::Redraw) {
            self.refresh(backend);
        }
        reaction
    }
}

impl CalcApp {
    /// Apply one [`CellGridControllerEvent`] from `handle`'s own
    /// `CellGridController::handle` call. The controller owns no cell
    /// content (see its module doc), so a request to edit or commit a
    /// cell is this app's cue to read/write that cell through the engine.
    fn apply_controller_event(&mut self, event: CellGridControllerEvent) -> Reaction {
        match event {
            CellGridControllerEvent::Ignored => Reaction::Continue,
            // F2: seed the inline editor with this cell's current raw
            // input — the controller has no cell content to seed it with
            // itself.
            CellGridControllerEvent::CellEditRequested { row, col } => {
                let a1 = Address::new(row as u32, col as u32).a1();
                let text = self.cell_input(&a1);
                self.grid.start_editing(text);
                Reaction::Redraw
            }
            // Enter/Tab/an arrow key committed the in-progress edit — hand
            // the raw text to the engine's own `cell.set`, the one way the
            // UI ever changes a workbook.
            CellGridControllerEvent::CellEditCommitted { row, col, text } => {
                let a1 = Address::new(row as u32, col as u32).a1();
                if let Err(e) = self.set_cell(&a1, &text) {
                    self.error = Some(e.to_string());
                }
                Reaction::Redraw
            }
            _ => Reaction::Redraw,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quadraui::tui::TuiBackend;
    use quadraui::{Modifiers, Viewport};

    fn backend() -> TuiBackend {
        let mut b = TuiBackend::new();
        b.begin_frame(Viewport::new(40.0, 20.0, 1.0));
        b
    }

    fn key(named: NamedKey, modifiers: Modifiers) -> UiEvent {
        UiEvent::KeyPressed {
            key: Key::Named(named),
            modifiers,
            repeat: false,
        }
    }

    fn shift() -> Modifiers {
        Modifiers {
            shift: true,
            ..Default::default()
        }
    }

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
    fn cursor_starts_at_the_origin() {
        let app = CalcApp::new();
        assert_eq!(app.cursor(), Address::new(0, 0));
    }

    #[test]
    fn arrow_keys_move_the_cursor_and_refresh_the_view() {
        let mut app = CalcApp::with_sample().unwrap();
        let mut b = backend();
        app.setup(&mut b);
        app.handle(key(NamedKey::Down, Modifiers::default()), &mut b);
        app.handle(key(NamedKey::Right, Modifiers::default()), &mut b);
        assert_eq!(app.cursor(), Address::new(1, 1));
        assert_eq!(app.view().selection_label, "B2");
        assert_eq!(app.view().active_text, "$1,200.50");
    }

    #[test]
    fn cursor_clamps_at_the_sheet_edge() {
        let mut app = CalcApp::new();
        let mut b = backend();
        app.setup(&mut b);
        app.handle(key(NamedKey::Up, Modifiers::default()), &mut b);
        app.handle(key(NamedKey::Left, Modifiers::default()), &mut b);
        assert_eq!(app.cursor(), Address::new(0, 0));
    }

    #[test]
    fn shift_arrow_extends_the_selection() {
        let mut app = CalcApp::new();
        let mut b = backend();
        app.setup(&mut b);
        app.handle(key(NamedKey::Down, Modifiers::default()), &mut b);
        app.handle(key(NamedKey::Right, shift()), &mut b);
        app.handle(key(NamedKey::Right, shift()), &mut b);
        assert_eq!(app.cursor(), Address::new(1, 2));
        assert_eq!(app.grid.selection_bounds(), (1, 0, 1, 2));
        assert_eq!(app.view().selection_label, "A2:C2");

        // A plain arrow afterwards collapses the selection back to one cell.
        app.handle(key(NamedKey::Down, Modifiers::default()), &mut b);
        assert_eq!(app.grid.selection_anchor(), None);
    }

    #[test]
    fn window_resized_refreshes_the_loaded_window() {
        let mut app = CalcApp::new();
        let mut b = backend();
        app.setup(&mut b);
        let rows_before = app.view().grid.as_ref().unwrap().cells.len();

        // Grow the viewport well past the original 20 rows and re-dispatch
        // through `handle`, the way a real resize reaches the app — without
        // an explicit `WindowResized` arm, `CellGridController::handle`
        // ignores the event and `refresh` never runs, so the cached `View`
        // stays sized for the old, smaller viewport.
        let grown = Viewport::new(40.0, 200.0, 1.0);
        b.begin_frame(grown);
        let reaction = app.handle(UiEvent::WindowResized { viewport: grown }, &mut b);
        assert_eq!(reaction, Reaction::Redraw);
        let rows_after = app.view().grid.as_ref().unwrap().cells.len();
        assert!(
            rows_after > rows_before,
            "resize should load more rows: before={rows_before} after={rows_after}"
        );
    }

    #[test]
    fn cell_size_is_tui_shaped_below_one_chrome_line_and_pixel_scaled_above_it() {
        assert_eq!(CalcApp::resolve_cell_size(1.0, 1.0), (1.0, 8.0));
        assert_eq!(CalcApp::resolve_cell_size(18.0, 7.0), (18.0, 70.0));
    }
}
