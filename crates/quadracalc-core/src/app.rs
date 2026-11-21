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

use gridcraft_engine::{Session, UiRequest};
use quadraui::control::{CommandError, CommandRegistry, CommandSpec};
use quadraui::{
    AppLogic, Backend, Cell, CellGrid, CellGridController, CellGridControllerEvent, Color, Key,
    MouseButton, NamedKey, Reaction, Rect, ScreenLayout, StatusBar, StatusBarSegment, Surface,
    TextField, UiEvent, WidgetId,
};
use serde_json::{Value, json};

use crate::address::Address;
use crate::agent;
use crate::clipboard;
use crate::formula_bar::{self, Bands};
use crate::palette::{CommandPalette, Outcome as PaletteOutcome};

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
    /// The last title `refresh` actually wrote through `backend.window()`
    /// — skips the OSC 0/2 write (TUI) or native title-set call on every
    /// frame when nothing changed, since `refresh` runs on every cursor
    /// move, not just the `cell.set`/save calls that can change it.
    last_title: Option<String>,
    /// Whether the formula bar — rather than the grid's own in-cell
    /// editor — currently owns the in-progress edit's caret. Always
    /// `false` when no edit is active; set by a click on the formula
    /// bar, cleared the moment that edit commits or cancels (from
    /// either the bar or the grid), since both end the one shared edit
    /// `CellGridController` tracks (see its module doc).
    formula_focused: bool,
    /// The command palette's state while it's open (Ctrl+Shift+P, or
    /// `:` on a TUI-shaped backend); `None` the rest of the time. While
    /// `Some`, `handle` routes every event to it instead of the grid.
    palette: Option<CommandPalette>,
    /// "Level 2" app commands an MCP client (or `quadraui::control::cli`)
    /// drives this app with — see `crate::agent`'s module doc. Swapped
    /// out for the duration of each `run_command` call per
    /// `CommandRegistry::run`'s own documented dance, then swapped back.
    commands: CommandRegistry<CalcApp>,
    /// Set by `run_command` on a successful Level 2 command, since that
    /// call has no `&mut dyn Backend` to refresh `view` with on the
    /// spot the way `handle` does. `tick` is the next hook that *does*
    /// get a backend, so it refreshes here instead — see that impl's
    /// own comment.
    agent_refresh_pending: bool,
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
    /// This frame's formula bar — mirrors `active_input` when no edit is
    /// in progress, or the live edit buffer while one is (see
    /// `formula_bar::build_field`).
    pub formula: TextField,
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
            last_title: None,
            formula_focused: false,
            palette: None,
            commands: agent::registry(),
            agent_refresh_pending: false,
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

    /// `with_sample()` with no `path`; otherwise a blank workbook with
    /// `path` opened into it through `file.open`. An open failure (a
    /// missing file, a corrupt workbook) never panics or fails
    /// construction — it lands in `self.error`, so the first `render`
    /// reports it in the status bar exactly like a bad `cell.set` does.
    pub fn new_for_path(path: Option<&str>) -> anyhow::Result<Self> {
        let Some(path) = path else {
            return Self::with_sample();
        };
        let mut app = Self::new();
        if let Err(e) = app.open_path(path) {
            app.error = Some(e.to_string());
        }
        Ok(app)
    }

    /// `file.open`: load `path` into the session, replacing whatever
    /// `new()`/`with_sample()` seeded it with.
    pub fn open_path(&mut self, path: &str) -> anyhow::Result<()> {
        self.execute("file.open", json!({ "path": path }))?;
        Ok(())
    }

    /// `file.save`: write the active workbook back to the path it was
    /// opened/saved from. With no such path yet — a workbook that was
    /// never opened from or saved to a file, or one opened from a format
    /// the engine won't save in place (e.g. `.ods`) — the engine queues a
    /// `saveAs` dialog request instead of erroring. Answering that dialog
    /// is out of scope here, so this turns it into a plain `Err` instead
    /// of silently doing nothing.
    pub fn save(&mut self) -> anyhow::Result<()> {
        let before = self.session.ui_requests.len();
        self.execute("file.save", json!({}))?;
        let asked_for_a_path = self.session.ui_requests.get(before..).is_some_and(|new| {
            new.iter()
                .any(|r| matches!(r, UiRequest::Dialog(kind, _) if kind == "saveAs"))
        });
        self.session.ui_requests.truncate(before);
        if asked_for_a_path {
            anyhow::bail!(
                "nothing to save to yet — this workbook has no file to save in place \
                 (never opened from a file, or opened from a format that can't be \
                 saved in place); use \"save as\" instead"
            );
        }
        Ok(())
    }

    /// `file.saveAs`: write the active workbook to `path`, which becomes
    /// its file (later plain `save()` calls write back here).
    pub fn save_as(&mut self, path: &str) -> anyhow::Result<()> {
        self.execute("file.saveAs", json!({ "path": path }))?;
        Ok(())
    }

    /// Whether the active workbook has changes since it was opened or
    /// last saved — what the window/terminal title's dirty marker shows.
    pub fn is_dirty(&self) -> bool {
        self.session
            .doc()
            .is_ok_and(gridcraft_engine::DocState::is_dirty)
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

    /// Push the UI's own cursor/selection (`CellGridController`, which the
    /// engine never sees) into the engine's `doc().selection` via
    /// `selection.set`, so a command run with no explicit `range`/`cell`
    /// (every palette run, `edit.undo`/`edit.redo`) resolves against what
    /// the user actually has selected instead of the engine's own
    /// `Default` (`Selection::at(CellRef::default())`, i.e. A1) — see
    /// `gridcraft_engine::cmd::target_ranges`. Called before every command
    /// the palette can run, and before snapshotting the catalog's own
    /// classification, since a precondition (`spec.info(session)`) can
    /// also depend on the selection.
    pub(crate) fn sync_selection(&mut self) -> anyhow::Result<()> {
        let a1 = self.cursor().a1();
        let (top, left, bottom, right) = self.grid.selection_bounds();
        let range = if top == bottom && left == right {
            a1.clone()
        } else {
            format!(
                "{}:{}",
                Address::new(top as u32, left as u32).a1(),
                Address::new(bottom as u32, right as u32).a1()
            )
        };
        self.execute("selection.set", json!({ "range": range, "active": a1 }))?;
        Ok(())
    }

    pub fn view(&self) -> &View {
        &self.view
    }

    /// This frame's [`Bands`] — formula bar, grid body, status bar —
    /// resolved from `backend`'s own viewport/line-height. Called
    /// identically by `render` (to paint) and `handle` (to hit-test and
    /// to size the grid controller's own layout), so the two can never
    /// disagree on where each band sits.
    fn bands(backend: &dyn Backend) -> Bands {
        let vp = backend.viewport();
        let lh = backend.line_height().max(1.0);
        formula_bar::bands(vp.width, vp.height, lh)
    }

    /// The grid body's rect — the viewport minus the formula bar and the
    /// status bar.
    fn grid_rect(backend: &dyn Backend) -> Rect {
        Self::bands(backend).grid
    }

    /// The command palette's popup rect: centred, a fixed fraction of
    /// the viewport (mirrors `quadraui-gallery`'s own
    /// `PaletteDemo::popup_rect`), floored at a size the primitive can
    /// still paint a title/query row plus a few items in.
    fn palette_rect(backend: &dyn Backend) -> Rect {
        let vp = backend.viewport();
        let lh = backend.line_height().max(1.0);
        let w = (vp.width * 0.7).max(30.0).min(vp.width);
        let h = (vp.height * 0.6).max(lh * 8.0).min(vp.height);
        Rect::new((vp.width - w) / 2.0, (vp.height - h) / 2.0, w, h)
    }

    /// Whether this backend is TUI-shaped — same `chrome_line_height`
    /// convention `resolve_cell_size` uses, since every current
    /// `Backend` this workspace can build against reports `1.0` there
    /// only for the TUI shape. Used to gate the `:` palette shortcut
    /// (a character TUI users type over Vim-style command lines; on a
    /// pixel backend it's just a literal colon to enter into a cell).
    fn is_tui_shaped(backend: &dyn Backend) -> bool {
        backend.chrome_line_height() <= 1.0
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

    /// The window/terminal title: the workbook's display name plus a
    /// trailing dirty marker when it has unsaved changes.
    fn window_title(name: &str, dirty: bool) -> String {
        if dirty {
            format!("{name} •")
        } else {
            name.to_string()
        }
    }

    /// Re-read exactly the visible window from the engine and rebuild
    /// `self.view` — called from `setup` and every `handle`, which are the
    /// only places with a `backend` to resolve
    /// `CellGridController::layout` against. Also the one place that
    /// retitles the window/terminal (`backend.window()`), since it is
    /// the same "did a command just run" cue the rest of the view uses.
    fn refresh(&mut self, backend: &mut dyn Backend) {
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

        let active_input = self.cell_input(&a1);
        let active_text = self.cell_text(&a1);
        let formula = formula_bar::build_field(&active_input, &self.grid, self.formula_focused);

        self.view = View {
            grid: Some(grid),
            selection_label,
            active_input,
            active_text,
            formula,
            error: self.error.clone(),
        };

        if let Ok(doc) = self.session.doc() {
            let title = Self::window_title(&doc.display_title(), doc.is_dirty());
            if self.last_title.as_deref() != Some(title.as_str())
                && let Some(window) = backend.window()
                && window.set_title(&title).is_ok()
            {
                self.last_title = Some(title);
            }
        }
    }
}

impl Default for CalcApp {
    fn default() -> Self {
        Self::new()
    }
}

impl View {
    /// The whole screen as `ScreenLayout` surfaces, back to front:
    /// formula bar, grid, status bar.
    fn screen(&self, bands: Bands) -> ScreenLayout {
        let mut layout = ScreenLayout::new();
        layout.push(Surface::TextField {
            rect: bands.formula,
            tf: self.formula.clone(),
        });
        if let Some(grid) = &self.grid {
            layout.push(Surface::CellGrid {
                rect: bands.grid,
                grid: grid.clone(),
            });
        }
        layout.push(self.status_surface(bands.status));
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
        // `snapshot`/`act`/`screenshot` (quadraui-mcp's Level 1 tools)
        // read `Backend::last_frame`, which only exists once capture is
        // turned on — see `quadraui::set_frame_capture`'s own doc.
        // `key`/`type` and this crate's own Level 2 commands don't need
        // it, but turning it on unconditionally costs nothing on a
        // backend that never gets an MCP server attached.
        quadraui::set_frame_capture(backend, true);
        self.refresh(backend);
    }

    fn render(&self, backend: &mut dyn Backend, _area: ()) {
        let bands = Self::bands(backend);
        let mut layout = self.view.screen(bands);
        // On top of everything else — the palette is a modal overlay,
        // so it paints last, same as `ScreenLayout`'s own
        // back-to-front push order for every other overlay surface.
        if let Some(palette) = &self.palette {
            layout.push(Surface::Palette {
                rect: Self::palette_rect(backend),
                palette: palette.palette(),
            });
        }
        let _ = layout.draw(backend);
    }

    fn handle(&mut self, event: UiEvent, backend: &mut dyn Backend) -> Reaction {
        self.error = None;
        // The palette is modal: while it's open, every key goes to it
        // (its own doc: "intercept ALL mouse and keyboard events when
        // open") and nothing else in `handle` runs — a click underneath
        // it is swallowed rather than reaching the grid, same as a
        // dialog/context-menu overlay would. A resize still needs
        // `refresh` so the grid behind the palette isn't left painting a
        // stale window once the palette closes (same reasoning as the
        // `WindowResized` arm below).
        if let Some(palette) = self.palette.as_mut() {
            if matches!(event, UiEvent::WindowResized { .. }) {
                self.refresh(backend);
                return Reaction::Continue;
            }
            let UiEvent::KeyPressed { key, modifiers, .. } = &event else {
                return Reaction::Continue;
            };
            return match palette.handle_key(key, modifiers) {
                PaletteOutcome::Changed => Reaction::Redraw,
                PaletteOutcome::Closed => {
                    self.palette = None;
                    Reaction::Redraw
                }
                PaletteOutcome::Run(id) => {
                    self.palette = None;
                    // The palette was classified and is now run against
                    // the UI's own selection, not necessarily the one the
                    // engine still has cached from the last `cell.set`/
                    // `edit.undo` — see `sync_selection`'s own doc.
                    if let Err(e) = self
                        .sync_selection()
                        .and_then(|()| self.execute(id, json!({})))
                    {
                        self.error = Some(e.to_string());
                    }
                    self.refresh(backend);
                    Reaction::Redraw
                }
                PaletteOutcome::Ignored => Reaction::Continue,
            };
        }
        // Ctrl+Shift+P (Cmd+Shift+P on macOS), or a bare `:` on a
        // TUI-shaped backend, opens the palette — outside an active
        // edit, so neither shortcut fights with typing a literal `:`
        // or capital `P` into a cell.
        if !self.grid.is_editing()
            && !self.formula_focused
            && let UiEvent::KeyPressed { key, modifiers, .. } = &event
        {
            let ctrl_shift_p = (modifiers.ctrl || modifiers.cmd)
                && modifiers.shift
                && matches!(key, Key::Char('p') | Key::Char('P'));
            let colon_on_tui = Self::is_tui_shaped(backend) && matches!(key, Key::Char(':'));
            if ctrl_shift_p || colon_on_tui {
                // Classification (`spec.info(session)`'s own preconditions)
                // reads the engine's selection, so it must already match
                // the UI's before `CommandPalette::open` snapshots it —
                // see `sync_selection`'s own doc.
                if let Err(e) = self.sync_selection() {
                    self.error = Some(e.to_string());
                }
                self.palette = Some(CommandPalette::open(&self.session));
                return Reaction::Redraw;
            }
        }
        // A click on the formula bar opens the same in-progress edit F2
        // would, just with the caret rendered in the bar instead of over
        // the cell (`quadraui`'s own `cell_grid_demo` does the same). A
        // click that lands anywhere else falls through to the checks
        // below unchanged.
        if let UiEvent::MouseDown {
            button: MouseButton::Left,
            position,
            ..
        } = &event
            && Self::bands(backend).formula.contains(*position)
        {
            if !self.grid.is_editing() {
                let a1 = self.cursor().a1();
                let text = self.cell_input(&a1);
                self.grid.start_editing(text);
            }
            self.formula_focused = true;
            self.refresh(backend);
            return Reaction::Redraw;
        }
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
        // Ctrl+S (Cmd+S on macOS) saves — also only outside an active
        // edit, so the shortcut never fights with typing a literal "s"
        // into a cell.
        if !self.grid.is_editing()
            && let UiEvent::KeyPressed { key, modifiers, .. } = &event
            && (modifiers.ctrl || modifiers.cmd)
            && matches!(key, Key::Char('s') | Key::Char('S'))
        {
            if let Err(e) = self.save() {
                self.error = Some(e.to_string());
            }
            self.refresh(backend);
            return Reaction::Redraw;
        }
        // Ctrl+Z/Ctrl+Y (Cmd+Z/Cmd+Y on macOS) undo/redo through the
        // engine's own history — same "outside an active edit" guard as
        // Ctrl+S, so the shortcut never fights with typing a literal "z"
        // or "y" into a cell.
        if !self.grid.is_editing()
            && let UiEvent::KeyPressed { key, modifiers, .. } = &event
            && (modifiers.ctrl || modifiers.cmd)
        {
            let command = if matches!(key, Key::Char('z') | Key::Char('Z')) {
                Some("edit.undo")
            } else if matches!(key, Key::Char('y') | Key::Char('Y')) {
                Some("edit.redo")
            } else {
                None
            };
            if let Some(command) = command {
                if let Err(e) = self.execute(command, json!({})) {
                    self.error = Some(e.to_string());
                }
                self.refresh(backend);
                return Reaction::Redraw;
            }
        }
        // Ctrl+C/X (Cmd on macOS) copy/cut the selection through
        // `crate::clipboard`'s bridge over `edit.copy`/`edit.cut`;
        // Ctrl+D/Ctrl+R fill the selection down/right through
        // `edit.fillDown`/`edit.fillRight`. Same "outside an active
        // edit" guard as every other shortcut above, so none of these
        // fight with typing a literal c/x/d/r into a cell. Ctrl+V has no
        // arm here: quadraui's runtime intercepts the native paste
        // keypress before it ever reaches `handle` as a `KeyPressed` —
        // it arrives below as `UiEvent::ClipboardPaste`, routed through
        // the grid controller like any other event, and lands in
        // `apply_controller_event`'s `PasteRequested` arm instead.
        if !self.grid.is_editing()
            && let UiEvent::KeyPressed { key, modifiers, .. } = &event
            && (modifiers.ctrl || modifiers.cmd)
        {
            type Action = fn(&mut CalcApp, &mut dyn Backend) -> anyhow::Result<()>;
            let action: Option<Action> = if matches!(key, Key::Char('c') | Key::Char('C')) {
                Some(clipboard::copy)
            } else if matches!(key, Key::Char('x') | Key::Char('X')) {
                Some(clipboard::cut)
            } else if matches!(key, Key::Char('d') | Key::Char('D')) {
                Some(clipboard::fill_down)
            } else if matches!(key, Key::Char('r') | Key::Char('R')) {
                Some(clipboard::fill_right)
            } else {
                None
            };
            if let Some(action) = action {
                if let Err(e) = action(self, backend) {
                    self.error = Some(e.to_string());
                }
                self.refresh(backend);
                return Reaction::Redraw;
            }
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
        // While the formula bar owns the caret, keys go to it instead of
        // the grid's own key dispatch — `handle_formula_bar_key` updates
        // the exact same in-progress edit, so typing there commits the
        // same way (Enter/Tab/arrows commit and move, Escape cancels).
        // Anything that isn't a keypress (e.g. a click elsewhere) is left
        // for a future frame rather than misrouted into the edit buffer.
        if self.formula_focused {
            return match &event {
                UiEvent::KeyPressed { key, modifiers, .. } => {
                    let event = self
                        .grid
                        .handle_formula_bar_key(key, modifiers, backend, rect);
                    let reaction = self.apply_controller_event(event);
                    if matches!(reaction, Reaction::Redraw) {
                        self.refresh(backend);
                    }
                    reaction
                }
                _ => Reaction::Continue,
            };
        }
        let event = self.grid.handle(&event, backend, rect);
        let reaction = self.apply_controller_event(event);
        if matches!(reaction, Reaction::Redraw) {
            self.refresh(backend);
        }
        reaction
    }

    /// Picks up the refresh `run_command` couldn't do itself (see
    /// `commands`'s field doc) — the next point in the per-frame loop
    /// that hands this app a `&mut dyn Backend`. One extra frame of
    /// delay between an agent-run command and its repaint, the same
    /// kind of delay `quadraui-mcp`'s own round-trip test polls
    /// `snapshot` to ride out.
    fn tick(&mut self, backend: &mut dyn Backend) -> Reaction {
        if self.agent_refresh_pending {
            self.agent_refresh_pending = false;
            self.refresh(backend);
            Reaction::Redraw
        } else {
            Reaction::Continue
        }
    }

    fn command_specs(&self) -> Vec<CommandSpec> {
        self.commands.specs()
    }

    fn run_command(&mut self, name: &str, args: Value) -> Result<Value, CommandError> {
        // `CommandRegistry::run`'s documented dance: swap the registry
        // out so its closures can take `&mut self` without aliasing the
        // field that holds them, then swap it back.
        let mut commands = std::mem::take(&mut self.commands);
        let result = commands.run(self, name, args);
        self.commands = commands;
        if result.is_ok() {
            self.agent_refresh_pending = true;
        }
        result
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
                self.formula_focused = false;
                Reaction::Redraw
            }
            // Escape cancelled the edit — whether it was typed through
            // the grid or the formula bar, neither owns the (now gone)
            // caret any more.
            CellGridControllerEvent::CellEditCancelled { .. } => {
                self.formula_focused = false;
                Reaction::Redraw
            }
            // Ctrl/Cmd+V with no edit in progress — quadraui's runtime
            // already resolved the system clipboard's text before this
            // ever ran (see `crate::clipboard::paste`'s own doc) and the
            // controller anchored it at the selection's top-left corner,
            // so this just hands both straight to `edit.paste`.
            CellGridControllerEvent::PasteRequested {
                anchor_row,
                anchor_col,
                text,
            } => {
                let anchor = Address::new(anchor_row as u32, anchor_col as u32).a1();
                if let Err(e) = clipboard::paste(self, &anchor, &text) {
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
    fn window_resize_refreshes_the_view_even_while_the_palette_is_open() {
        let mut app = CalcApp::new();
        let mut b = backend();
        app.setup(&mut b);
        app.palette = Some(CommandPalette::open(&app.session));
        let rows_before = app.view().grid.as_ref().unwrap().cells.len();

        let grown = Viewport::new(40.0, 200.0, 1.0);
        b.begin_frame(grown);
        let reaction = app.handle(UiEvent::WindowResized { viewport: grown }, &mut b);
        // Swallowed from the grid's own dispatch, same as every other
        // event while the palette is modal, but the view behind it is
        // still refreshed.
        assert_eq!(reaction, Reaction::Continue);
        assert!(app.palette.is_some(), "the palette should stay open");
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

    #[test]
    fn window_title_adds_a_dirty_marker_only_when_dirty() {
        assert_eq!(CalcApp::window_title("Book1", false), "Book1");
        assert_eq!(CalcApp::window_title("Book1", true), "Book1 •");
    }

    #[test]
    fn editing_a_cell_marks_the_workbook_dirty_until_it_is_saved() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "quadracalc-dirty-{}-{}.xlsx",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = path.to_str().unwrap();

        let mut app = CalcApp::new();
        app.save_as(path).unwrap();
        assert!(!app.is_dirty(), "a freshly saved workbook should be clean");

        app.set_cell("A1", "hello").unwrap();
        assert!(app.is_dirty(), "cell.set should mark the workbook dirty");

        app.save().unwrap();
        assert!(!app.is_dirty(), "save() should clear the dirty marker");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn save_without_a_file_errors_instead_of_silently_doing_nothing() {
        let mut app = CalcApp::new();
        assert!(app.save().is_err());
    }

    #[test]
    fn ctrl_s_saves_through_handle_and_clears_the_title_dirty_marker() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "quadracalc-ctrl-s-{}-{}.xlsx",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = path.to_str().unwrap();

        let mut app = CalcApp::new();
        let mut b = backend();
        app.setup(&mut b);
        app.save_as(path).unwrap();
        // `set_cell` rather than typing through the controller — only the
        // Ctrl+S path under test needs to go through `handle`.
        app.set_cell("A1", "hello").unwrap();
        app.refresh(&mut b);
        assert!(
            app.last_title.as_deref().unwrap().ends_with('•'),
            "dirty workbook should show the marker before saving: {:?}",
            app.last_title
        );

        let ctrl_s = UiEvent::KeyPressed {
            key: Key::Char('s'),
            modifiers: Modifiers {
                ctrl: true,
                ..Default::default()
            },
            repeat: false,
        };
        let reaction = app.handle(ctrl_s, &mut b);

        assert_eq!(reaction, Reaction::Redraw);
        assert!(
            app.view().error.is_none(),
            "Ctrl+S should not error: {:?}",
            app.view().error
        );
        assert!(
            !app.last_title.as_deref().unwrap().ends_with('•'),
            "save() through Ctrl+S should clear the title's dirty marker: {:?}",
            app.last_title
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn new_for_path_reports_a_bad_path_in_the_error_field_without_panicking() {
        let app = CalcApp::new_for_path(Some("/no/such/workbook.xlsx")).unwrap();
        assert!(app.view.error.is_none(), "no render has run yet");
        // `open_path` itself failed, so the error lives on `self.error`
        // before the first `refresh` ever copies it into `view.error`.
        assert!(app.error.is_some());
    }

    #[test]
    fn new_for_path_with_no_path_keeps_the_sample_sheet() {
        let mut app = CalcApp::new_for_path(None).unwrap();
        assert_eq!(app.cell_text("A1"), "Item");
    }
}
