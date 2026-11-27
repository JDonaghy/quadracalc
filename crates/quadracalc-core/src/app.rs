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
//! The grid itself is [`CellGrid`] — quadraui's spreadsheet primitive
//! (quadraui#2732), a widget-contract [`Interactive`] widget reached
//! through `Surface::Widget` rather than a bespoke `Surface::CellGrid`
//! variant (quadraui#2918/#2920). It owns cursor/selection/scroll/
//! frozen-pane/in-progress-edit state as plain fields a host rebuilds
//! every frame; this app owns cell content and reads only the window
//! [`CellGrid::layout`] reports is on screen, never the whole sheet.
//! `Interactive::handle` only ever has `&self` (see that trait's own
//! doc), so every navigation/edit outcome comes back as a
//! [`CellGridEvent`] for [`CalcApp::apply_grid_event`] to write onto
//! `self.grid`'s fields, rather than the grid mutating itself in place —
//! `Self::dispatch_to_grid` is this app's own hand-rolled routing into
//! it, mirroring quadraui's own `examples/common/cell_grid_demo.rs`.

use gridcraft_engine::catalog::CATALOG;
use gridcraft_engine::{Session, UiRequest};
use quadraui::control::{CommandError, CommandRegistry, CommandSpec};
use quadraui::interactive::SubTarget;
use quadraui::primitive::{LayoutCtx, Primitive, Size, Units};
use quadraui::primitives::cell_grid::CellEditState;
use quadraui::primitives::context_menu::{ContextMenuEvent, ContextMenuState};
use quadraui::primitives::text_field::{TextFieldAction, TextFieldState};
use quadraui::{
    AppLogic, AppMenu, AppMenuEvent, AppMenuState, Backend, ButtonMask, Cell, CellGrid,
    CellGridEvent, CellGridHit, CellGridPrimitiveLayout, CellGridState, Chart, Color, ContextMenu,
    ContextMenuItem, DrawsPrimitives, EditOp, EventCx, EventHost, FindReplaceEvent,
    FindReplaceField, Handled, Interactive, Key, KeyEvent, ListViewHit, Modifiers, MouseButton,
    NamedKey, Point, PointerKind, PointerShape, Reaction, Rect, RoutedEvent, ScreenLayout,
    SidebarPanelHit, StatusBar, StatusBarSegment, StyledText, Surface, TabBar, TabBarHit,
    TextField, UiEvent, WidgetId, resolve_bar_height,
};
use serde_json::{Value, json};

use crate::address::Address;
use crate::agent;
use crate::cell_style;
use crate::chart_view;
use crate::clipboard;
use crate::find::{self, FindBar};
use crate::format_picker::{self, FormatPicker, PickerOutcome};
use crate::formula_bar::{self, Bands};
use crate::function_picker::{self, FunctionPicker, Outcome as FunctionPickerOutcome};
use crate::menu;
use crate::palette::{CommandPalette, Outcome as PaletteOutcome};
use crate::sheet_view::SheetView;
use crate::sheets;
use crate::sidebar::{self, Sidebar};

/// Rows in the demo sheet — far more than any terminal or window shows at
/// once, so scrolling to a distant row exercises `CellGrid`'s own
/// two-axis virtualisation rather than anything this app does.
const ROW_COUNT: usize = 10_000;
/// Columns in the demo sheet, likewise wider than any viewport.
const COL_COUNT: usize = 50;

pub struct CalcApp {
    session: Session,
    grid: CellGrid,
    /// [`CellGrid`]'s own [`Interactive::State`] — the in-progress
    /// mouse-drag anchor, the one piece of interaction state with no
    /// `CellGrid` field of its own (see that type's own doc).
    grid_state: CellGridState,
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
    /// `CellGrid::editing` tracks.
    formula_focused: bool,
    /// The command palette's state while it's open (Ctrl+Shift+P, or
    /// `:` on a TUI-shaped backend); `None` the rest of the time. While
    /// `Some`, `handle` routes every event to it instead of the grid.
    palette: Option<CommandPalette>,
    /// Open while a Home-menu command that needs a parameter this app
    /// can't supply up front (a font name/size, a fill/font color, a
    /// border preset, a number format) is being picked — opened by
    /// `PaletteOutcome::Pick` in place of running that command
    /// directly. Modal exactly like `palette`: every event routes here
    /// instead of the grid while `Some`.
    format_picker: Option<FormatPicker>,
    /// Open while the Formulas tab's searchable function list is up
    /// (`formulas.insertFunction`, or a Function Library category —
    /// `function_picker::has_picker`) — opened by `PaletteOutcome::PickFunction`
    /// in place of running that id directly. Modal exactly like `palette`
    /// and `format_picker`: every event routes here instead of the grid
    /// while `Some`.
    function_picker: Option<FunctionPicker>,
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
    /// Every sheet's own `CellGrid` cursor/selection/scroll, so switching
    /// sheets and back restores exactly where the user left off — see
    /// `crate::sheets`'s module doc for why this lives here rather than
    /// in the engine.
    sheet_cursors: sheets::SheetCursors,
    /// The sheet-tabs strip's own scroll offset — the app's half of the
    /// `TabBar` backend contract (`quadraui::TabBar::layout`'s doc): the
    /// bar's `scroll_offset` field is only ever an *input*, and the
    /// caller must write back whatever the backend resolves it to each
    /// frame (`refresh` does this), or the active tab silently scrolls
    /// off the strip the first time the sheet labels overflow the bar's
    /// width.
    tab_scroll: usize,
    /// The right-hand Name Manager sidebar's open/selected state — see
    /// `crate::sidebar`'s module doc. Closed by default.
    sidebar: Sidebar,
    /// The classic menu bar — `quadraui`'s `AppMenu` widget, driven
    /// with raw events through `AppMenu::handle_ui_event`, which keeps
    /// the open/highlight view on the descriptor and the open dropdown
    /// on the modal stack; `crate::menu::menus` rebuilds its `MenuDef`s
    /// from the engine's own command catalog every `refresh`, so e.g.
    /// "Undo" greys out the moment there's nothing left to undo.
    app_menu: AppMenu,
    /// [`AppMenu`]'s own [`Interactive::State`] — the open dropdown's
    /// type-ahead buffer.
    app_menu_state: AppMenuState,
    /// Zebra-stripe the grid's body rows ([`CellGrid::striped`]). On by
    /// default; a workbook has no setting of its own for it.
    striped: bool,
    /// The find/replace panel while it's open — see `crate::find`.
    find: Option<FindBar>,
    /// The open find panel's matches on the active sheet, in reading
    /// order, recomputed whenever the query, its options or the sheet
    /// change.
    find_hits: Vec<(usize, usize)>,
    /// The grid's right-click (or Shift+F10) menu while it's open, and
    /// the box it was placed in.
    context_menu: Option<(ContextMenu, Rect)>,
    /// [`ContextMenu`]'s own [`Interactive::State`] — its type-ahead
    /// buffer.
    context_menu_state: ContextMenuState,
    /// Wheel travel not yet turned into whole rows/columns, so a
    /// trackpad's sub-row deltas add up instead of being dropped or each
    /// rounded up to a full row.
    wheel_carry: (f32, f32),
    /// The formula bar's own [`TextField`] state — its caret and a
    /// pointer drag in progress — while the formula bar owns the edit.
    formula_state: TextFieldState,
}

/// Everything `render` needs, rebuilt after every command or cursor move.
#[derive(Debug, Clone, Default)]
pub struct View {
    /// The fully-built grid for this frame — geometry plus the windowed
    /// cell text the engine supplied for exactly the rows/columns
    /// `CellGrid::layout` reported as visible. `None` only before the
    /// first `setup`/`handle` call ever runs.
    pub grid: Option<CellGrid>,
    /// This frame's sheet tabs — the engine's own sheet list, rebuilt by
    /// `crate::sheets::tab_bar` on every `refresh`. `None` only before
    /// the first `setup`/`handle` call ever runs, same as `grid`.
    pub tab_bar: Option<TabBar>,
    /// This frame's Name Manager rows (`formulas.nameManager`) —
    /// re-fetched on every `refresh` while `CalcApp`'s own `sidebar` is
    /// open, left empty while it's closed so a closed sidebar never
    /// pays for a query nothing is showing.
    pub sidebar_names: Vec<sidebar::NameEntry>,
    /// The cursor's address, or `top:bottom` range notation for a
    /// multi-cell selection.
    pub selection_label: String,
    /// Excel's own bottom-right status-bar summary for the current
    /// selection — `Average: … Count: … Sum: …` when the selection has at
    /// least one number, or just `Count: …` for a non-numeric selection
    /// (text/formulas-as-text/empty), same fallback Excel uses. Empty
    /// when the selection has no non-empty cells at all, so
    /// `status_surface` paints nothing rather than a bare `Count: 0`.
    pub selection_stats: String,
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
    /// What the keyboard is driving right now — the status bar's mode
    /// badge and the key hints beside it follow it.
    pub mode: Mode,
    /// Whether this frame is for a terminal, where `:` opens the palette.
    pub tui_shaped: bool,
    /// The active sheet's charts (`crate::chart_view::charts`), resolved
    /// fresh on every `refresh` — empty when it has none, which is also
    /// what tells [`CalcApp::grid_rect`] and [`View::screen`] not to
    /// carve out a chart panel at all. See `crate::chart_view`'s module
    /// doc for why this is a quadraui [`Chart`] already resolved to
    /// plain numbers, not the engine's own `Chart` model.
    pub charts: Vec<Chart>,
}

/// What the keyboard is driving, as the status bar names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Moving around the grid.
    #[default]
    Ready,
    /// An in-progress cell edit, in the cell or the formula bar.
    Edit,
    /// The find/replace panel has the keyboard.
    Find,
}

impl Mode {
    fn badge(self) -> &'static str {
        match self {
            Mode::Ready => " READY ",
            Mode::Edit => " EDIT ",
            Mode::Find => " FIND ",
        }
    }

    /// The keys worth knowing in this mode — the status bar's help line.
    /// `tui` picks the terminal's own palette key (`:`) over the
    /// chord every backend accepts.
    fn hints(self, tui: bool) -> &'static str {
        match self {
            Mode::Ready if tui => " F2 edit · Ctrl+F find · : commands · Alt+H menu · q quit ",
            Mode::Ready => " F2 edit · Ctrl+F find · Ctrl+Shift+P commands · q quit ",
            Mode::Edit => " Enter commit · Esc cancel · Tab next · Ctrl+A all ",
            Mode::Find => " Enter next · Shift+Enter prev · Esc close ",
        }
    }
}

impl CalcApp {
    /// A new, empty workbook.
    pub fn new() -> Self {
        let mut session = Session::new();
        session.new_workbook();
        let mut grid = CellGrid::new(WidgetId::new("quadracalc-grid"));
        grid.row_count = ROW_COUNT;
        grid.col_count = COL_COUNT;
        grid.has_focus = true;
        let app_menu = AppMenu::new(WidgetId::new("quadracalc-menu"), menu::menus(&session));
        Self {
            session,
            grid,
            grid_state: CellGridState::default(),
            error: None,
            view: View::default(),
            last_title: None,
            formula_focused: false,
            palette: None,
            format_picker: None,
            function_picker: None,
            commands: agent::registry(),
            agent_refresh_pending: false,
            sheet_cursors: sheets::SheetCursors::default(),
            tab_scroll: 0,
            sidebar: Sidebar::default(),
            app_menu,
            app_menu_state: AppMenuState::default(),
            striped: true,
            find: None,
            find_hits: Vec::new(),
            context_menu: None,
            context_menu_state: ContextMenuState::default(),
            wheel_carry: (0.0, 0.0),
            formula_state: TextFieldState::default(),
        }
    }

    /// A workbook with a sample sheet worth a first look: a frozen,
    /// filled header row and a frozen label column, formulas (`SUM`, a
    /// `VLOOKUP` against a rate table, per-row products, month-on-month
    /// growth, `INDEX`/`MATCH` over the year), currency and percentage
    /// number formats, and column widths sized to their contents — every
    /// one of them set through the engine's own commands, the same ones
    /// the menu and palette run.
    ///
    /// `A1:B4` keeps its original cells and formula (`editing.rs`,
    /// `open_save.rs` and others assert on `=SUM(B2:B3)` and its typed
    /// inputs by name), as do the `A6:B8` rate table and the `D1:F2`
    /// lookup chain; the monthly table below them is additive.
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
            // A small discount-rate lookup table, and a chain that reads
            // the sample's own total through a VLOOKUP against it.
            ("A6", "Tier"),
            ("B6", "Discount"),
            ("A7", "Standard"),
            ("B7", "0.05"),
            ("A8", "Premium"),
            ("B8", "0.15"),
            ("D1", "Tier"),
            ("D2", "Premium"),
            ("E1", "Discount"),
            ("E2", "=VLOOKUP(D2,A7:B8,2,FALSE)"),
            ("F1", "Discounted Total"),
            ("F2", "=B4*(1-E2)"),
        ] {
            app.set_cell(cell, input)?;
        }

        // A year of sales: units and price typed in, revenue and growth
        // computed, a totals row, and a best-month lookup beside it.
        const MONTHS: [(&str, u32, f64); 12] = [
            ("Jan", 120, 24.0),
            ("Feb", 135, 24.0),
            ("Mar", 160, 24.5),
            ("Apr", 152, 24.5),
            ("May", 181, 25.0),
            ("Jun", 204, 25.0),
            ("Jul", 197, 25.5),
            ("Aug", 223, 25.5),
            ("Sep", 241, 26.0),
            ("Oct", 236, 26.0),
            ("Nov", 268, 26.5),
            ("Dec", 312, 27.0),
        ];
        for (cell, input) in [
            ("A10", "Month"),
            ("B10", "Units"),
            ("C10", "Price"),
            ("D10", "Revenue"),
            ("E10", "Growth"),
            ("F10", "Over target?"),
            ("H10", "At a glance"),
            ("H11", "Best month"),
            ("I11", "=INDEX(A11:A22,MATCH(MAX(D11:D22),D11:D22,0))"),
            ("H12", "Peak revenue"),
            ("I12", "=MAX(D11:D22)"),
            ("H13", "Avg units"),
            ("I13", "=ROUND(AVERAGE(B11:B22),1)"),
            ("H14", "Target"),
            ("I14", "5000"),
            ("A23", "Total"),
            ("B23", "=SUM(B11:B22)"),
            ("C23", "=D23/B23"),
            ("D23", "=SUM(D11:D22)"),
            ("E23", "=D22/D11-1"),
            ("F23", "=COUNTIF(F11:F22,\"yes\")&\" of 12\""),
        ] {
            app.set_cell(cell, input)?;
        }
        for (i, (month, units, price)) in MONTHS.iter().enumerate() {
            let row = 11 + i;
            app.set_cell(&format!("A{row}"), month)?;
            app.set_cell(&format!("B{row}"), &units.to_string())?;
            app.set_cell(&format!("C{row}"), &price.to_string())?;
            app.set_cell(&format!("D{row}"), &format!("=B{row}*C{row}"))?;
            if row > 11 {
                app.set_cell(&format!("E{row}"), &format!("=D{row}/D{}-1", row - 1))?;
            }
            app.set_cell(
                &format!("F{row}"),
                &format!("=IF(D{row}>=$I$14,\"yes\",\"no\")"),
            )?;
        }

        // Header bands: bold white on a dark fill for the frozen top row,
        // a lighter accent for the monthly table's own header.
        for (range, fill) in [
            ("A1:F1", "#1F4E79"),
            ("A10:F10", "#2E75B6"),
            ("H10:I10", "#2E75B6"),
        ] {
            app.execute("home.bold", json!({ "range": range, "on": true }))?;
            app.execute("home.fillColor", json!({ "range": range, "color": fill }))?;
            app.execute(
                "home.fontColor",
                json!({ "range": range, "color": "#FFFFFF" }),
            )?;
        }
        for range in ["A4:B4", "A6:B6", "A23:F23", "H11:H14"] {
            app.execute("home.bold", json!({ "range": range, "on": true }))?;
        }
        app.execute("home.alignCenter", json!({ "range": "B10:F10" }))?;
        app.execute("home.italic", json!({ "range": "D2", "on": true }))?;
        for (range, format) in [
            ("B2:B4", "Currency"),
            ("F2", "Currency"),
            ("C11:D23", "Currency"),
            ("I12", "Currency"),
            ("I14", "Currency"),
            ("E2", "Percentage"),
            ("E11:E23", "Percentage"),
        ] {
            app.execute(
                "home.numberFormat",
                json!({ "range": range, "format": format }),
            )?;
        }
        // In the engine's own 96-dpi pixels (a default column is 64px);
        // the grid scales them to its own units (see `crate::sheet_view`).
        for (cols, px) in [
            ("A:A", 80),
            ("B:B", 88),
            ("C:C", 64),
            ("D:D", 96),
            ("E:E", 80),
            ("F:F", 136),
            ("G:G", 16),
            ("H:H", 104),
            ("I:I", 88),
        ] {
            app.execute("home.columnWidth", json!({ "cols": cols, "width": px }))?;
        }
        // Row 1 and column A stay put while the rest scrolls.
        app.execute("view.freezePanes", json!({ "cell": "B2" }))?;
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

    /// The engine's displayed text for a cell (formatted value, not the
    /// formula) — `TRUE`/`FALSE` swapped for a checkbox glyph on a cell
    /// `insert.checkbox` formatted (see [`Self::checkbox_glyph`]'s own
    /// doc). `cell_input` doesn't get this treatment: an in-progress
    /// edit needs to see and type the literal `TRUE`/`FALSE`, not a
    /// glyph that isn't a valid boolean literal to type back in.
    pub fn cell_text(&mut self, a1: &str) -> String {
        let Ok(cell) = self.session.execute("cell.get", json!({ "cell": a1 })) else {
            return String::new();
        };
        let text = cell.get("text").and_then(Value::as_str).unwrap_or_default();
        Self::checkbox_glyph(&cell, text).unwrap_or_else(|| text.to_string())
    }

    /// This cell's grid presentation: [`Self::cell_text`]'s display text
    /// plus whatever of its style [`quadraui::CellGrid`] can paint (see
    /// `crate::cell_style`), read off the same `cell.get` call
    /// `cell_text` would otherwise make a second time, so a visible
    /// cell's style costs nothing extra over its text.
    fn grid_cell(&mut self, a1: &str) -> Cell {
        let Ok(cell) = self.session.execute("cell.get", json!({ "cell": a1 })) else {
            return Cell::plain(String::new());
        };
        let text = cell.get("text").and_then(Value::as_str).unwrap_or_default();
        let text = Self::checkbox_glyph(&cell, text).unwrap_or_else(|| text.to_string());
        Cell {
            text,
            style: cell_style::from_engine(cell.get("style")),
        }
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

    /// `text` (`cell.get`'s own `"text"` field) swapped for a Unicode
    /// checkbox glyph when `cell`'s style is GridCraft's own
    /// `"checkbox"` number-format sentinel (`insert.checkbox`'s doc:
    /// "TRUE/FALSE and empty cells draw as checkboxes") — `None` for
    /// every other cell, or a checkbox-formatted one whose value isn't
    /// `TRUE`/`FALSE` (a plain number or text someone typed over it
    /// still shows as itself, same as GridCraft's own egui grid). This
    /// is the terminal-appropriate placeholder #20 asks for in place of
    /// an actual checkbox widget: quadraui's `CellGrid` has no bespoke
    /// one (CLAUDE.md: "use quadraui widgets; don't build widgets
    /// here"), and plain `TRUE`/`FALSE` text is visible but reads as a
    /// boolean cell, not a control to click.
    fn checkbox_glyph(cell: &Value, text: &str) -> Option<String> {
        let is_checkbox = cell
            .get("style")
            .and_then(|s| s.get("num_fmt"))
            .and_then(Value::as_str)
            == Some("checkbox");
        if !is_checkbox {
            return None;
        }
        match text {
            "TRUE" => Some("☑".to_string()),
            "FALSE" => Some("☐".to_string()),
            _ => None,
        }
    }

    pub fn cursor(&self) -> Address {
        let (row, col) = self.grid.cursor;
        Address::new(row as u32, col as u32)
    }

    /// The current selection's top-left corner — a single cell's own
    /// cursor for a one-cell selection, otherwise the same corner
    /// `CalcApp::handle`'s own `UiEvent::ClipboardPaste` arm already
    /// resolves for a Ctrl+V paste. `clipboard::paste_from_system`
    /// targets this, not [`Self::cursor`], so a menu-triggered paste
    /// lands on the same cell Ctrl+V would for the identical selection —
    /// the cursor alone can sit at any corner of a Shift-extended
    /// selection.
    pub fn selection_anchor(&self) -> Address {
        let (top, left, _, _) = self.grid.selection_bounds();
        Address::new(top as u32, left as u32)
    }

    /// Sheet names in workbook order, exactly as the engine has them —
    /// what the tab bar shows.
    pub fn sheet_names(&self) -> Vec<String> {
        sheets::sheet_names(&self.session)
    }

    /// The active sheet's index into `sheet_names`.
    pub fn active_sheet_index(&self) -> usize {
        sheets::active_sheet_index(&self.session)
    }

    /// `sheet.activate`: switch to the sheet at `index`, restoring that
    /// sheet's own remembered cursor/selection/scroll.
    pub fn activate_sheet(&mut self, index: usize) -> anyhow::Result<()> {
        self.switch_sheet("sheet.activate", json!({ "sheet": index }))
    }

    /// `sheet.next`: switch to the next visible sheet (skipping hidden
    /// ones), or stay put if the active sheet is already the last one.
    pub fn next_sheet(&mut self) -> anyhow::Result<()> {
        self.switch_sheet("sheet.next", json!({}))
    }

    /// `sheet.previous`: switch to the previous visible sheet (skipping
    /// hidden ones), or stay put if the active sheet is already the
    /// first one.
    pub fn previous_sheet(&mut self) -> anyhow::Result<()> {
        self.switch_sheet("sheet.previous", json!({}))
    }

    /// Run a sheet-switching command and swap `self.grid`'s cursor/
    /// selection/scroll for whichever sheet it lands on — the one piece
    /// of a sheet switch the engine has no concept of (see
    /// `crate::sheets`'s module doc). `id` must be one of
    /// `sheet.activate`/`sheet.next`/`sheet.previous`; any other command
    /// still runs, but a command that doesn't change the active sheet
    /// just saves and restores the same sheet's own state, a harmless
    /// no-op.
    fn switch_sheet(&mut self, id: &str, params: Value) -> anyhow::Result<()> {
        let leaving = sheets::active_sheet_name(&self.session);
        self.execute(id, params)?;
        if let Some(leaving) = leaving {
            self.sheet_cursors
                .save(&leaving, sheets::CursorMemory::capture(&self.grid));
        }
        if let Some(arriving) = sheets::active_sheet_name(&self.session) {
            self.sheet_cursors.get(&arriving).restore(&mut self.grid);
        }
        Ok(())
    }

    /// Push the UI's own cursor/selection (`CellGrid`, which the engine
    /// never sees) into the engine's `doc().selection` via
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

    /// `formulas.functions`'s own rows for `id` (`crate::function_picker::query_params`
    /// resolves the category, if any, straight from the Formulas-tab id
    /// that opened the picker), turned into a `FunctionPicker`.
    fn open_function_picker(&mut self, id: &'static str) -> anyhow::Result<FunctionPicker> {
        let functions = self.execute("formulas.functions", function_picker::query_params(id))?;
        Ok(FunctionPicker::open(function_picker::parse_entries(
            &functions,
        )))
    }

    /// Types `name` plus `()` into whatever edit is active — through
    /// [`Self::type_key_into_grid`], the same per-character path a
    /// user's own typing takes, so the caret ends up right after the
    /// closing paren exactly like it would after typing the call by
    /// hand. If nothing is being edited yet, opens one on the formula
    /// bar first: seeded with the cursor cell's own raw text when that's
    /// already a formula (so picking a function from an existing one
    /// appends a nested call instead of discarding it), or with a bare
    /// `=` otherwise — same "a name starts the formula" convention
    /// `gridcraft_engine`'s own `formulas.insertFunction` follows for a
    /// supplied `name` (`={name}(`), just typed through the caret
    /// instead of replacing the cell outright.
    fn insert_function_call(&mut self, name: &str, backend: &mut dyn Backend) {
        if self.grid.editing.is_none() {
            let a1 = self.cursor().a1();
            let existing = self.cell_input(&a1);
            let seed = if existing.starts_with('=') {
                existing
            } else {
                "=".to_string()
            };
            self.start_editing(seed);
            self.formula_focused = true;
        }
        for c in format!("{}()", name.to_ascii_uppercase()).chars() {
            self.type_key_into_grid(Key::Char(c), backend);
        }
    }

    /// Open an edit on `self.grid.cursor`, seeded with `text` and the
    /// caret at its end — `CellGrid` has no cell content to seed itself
    /// with (see its module doc), so every edit-opening path (F2, a
    /// plain keypress, a formula-bar click, [`Self::insert_function_call`])
    /// funnels through this one place to build the [`CellEditState`].
    fn start_editing(&mut self, text: String) {
        let cursor = text.len();
        self.grid.editing = Some(CellEditState {
            text,
            cursor,
            selection_anchor: None,
            placeholder: None,
        });
    }

    /// Route one synthetic key into the grid's own [`CellGrid::handle`]
    /// via [`Self::dispatch_to_grid`] and apply whatever
    /// [`CellGridEvent`] it reports — the same per-character path a
    /// user's own typing takes, used where this app types on the grid's
    /// behalf ([`Self::insert_function_call`]) rather than in response
    /// to a real `UiEvent::KeyPressed`.
    fn type_key_into_grid(&mut self, key: Key, backend: &mut dyn Backend) -> Reaction {
        let rect = self.grid_rect(backend);
        let layout = Self::grid_layout(&self.grid, backend, rect);
        let routed = RoutedEvent::Key(KeyEvent {
            key,
            modifiers: Modifiers::default(),
            repeat: false,
        });
        self.dispatch_to_grid(routed, &layout, backend)
    }

    pub fn view(&self) -> &View {
        &self.view
    }

    /// The menu bar's own rect — the viewport's top row, sized by
    /// `resolve_bar_height` (the same metric `quadraui`'s own
    /// `AppMenu` example uses: a fixed in-canvas height on a pixel
    /// backend, one cell on TUI).
    fn menu_bar_rect(backend: &dyn Backend) -> Rect {
        let vp = backend.viewport();
        let h = resolve_bar_height(backend.chrome_line_height()).min(vp.height);
        Rect::new(0.0, 0.0, vp.width, h)
    }

    /// The name box's width: room for a range like `D11:D22` plus the
    /// `fx` marker.
    fn name_box_width(backend: &dyn Backend) -> f32 {
        if Self::is_tui_shaped(backend) {
            14.0
        } else {
            backend.char_width() * 16.0
        }
    }

    /// `(name box, formula field)`: the formula band split at
    /// `name_box_width` (never more than half the band).
    fn split_formula_band(band: Rect, name_box_width: f32) -> (Rect, Rect) {
        let w = name_box_width.min(band.width / 2.0).max(0.0);
        (
            Rect::new(band.x, band.y, w, band.height),
            Rect::new(band.x + w, band.y, band.width - w, band.height),
        )
    }

    /// Recovers the `&'static str` [`CATALOG`] holds for `id`, since a
    /// menu click only ever hands back a borrowed `&str` (`WidgetId::
    /// as_str`), but [`FormatPicker::open`] wants the same `'static`
    /// lifetime [`crate::palette::Entry::id`] already carries for the
    /// palette's own `PaletteOutcome::Pick` path.
    fn catalog_id(id: &str) -> Option<&'static str> {
        CATALOG
            .iter()
            .find(|&&(cid, ..)| cid == id)
            .map(|&(cid, ..)| cid)
    }

    /// This frame's [`Bands`] — formula bar, grid body, status bar —
    /// resolved from `backend`'s own viewport/line-height, offset below
    /// [`Self::menu_bar_rect`] (this app's own fourth chrome row;
    /// `formula_bar::bands` only knows about the other two). Called
    /// identically by `render` (to paint) and `handle` (to hit-test and
    /// to size the grid controller's own layout), so the two can never
    /// disagree on where each band sits.
    fn bands(backend: &dyn Backend) -> Bands {
        let vp = backend.viewport();
        let lh = backend.line_height().max(1.0);
        let menu_h = Self::menu_bar_rect(backend).height;
        let b = formula_bar::bands(vp.width, (vp.height - menu_h).max(0.0), lh);
        Bands {
            formula: Rect::new(
                b.formula.x,
                b.formula.y + menu_h,
                b.formula.width,
                b.formula.height,
            ),
            grid: Rect::new(b.grid.x, b.grid.y + menu_h, b.grid.width, b.grid.height),
            status: Rect::new(
                b.status.x,
                b.status.y + menu_h,
                b.status.width,
                b.status.height,
            ),
        }
    }

    /// The grid body's rect — the viewport minus the formula bar, the
    /// status bar, the sheet-tabs strip, the Name Manager sidebar when
    /// it's open, and the chart panel when the active sheet has charts
    /// to show.
    fn grid_rect(&self, backend: &dyn Backend) -> Rect {
        let grid = Self::carve_tab_strip(self.content_bands(backend)).0;
        Self::carve_chart_panel(grid, self.view.charts.len()).0
    }

    /// The sheet-tabs strip's rect — narrowed the same way `grid_rect`
    /// is when the sidebar is open, so the strip never runs under it.
    /// Not further narrowed for the chart panel: the tabs strip always
    /// spans the full content width, with the chart panel (like the
    /// cell grid above it) carved from what's left above it instead.
    fn tab_bar_rect(&self, backend: &dyn Backend) -> Rect {
        Self::carve_tab_strip(self.content_bands(backend)).1
    }

    /// `(cell grid rect, chart panel rect)`: with no charts to show,
    /// `grid` passes through unchanged and the second rect is `None` —
    /// the common case costs nothing. With at least one, a third of
    /// `grid`'s own height (floored at 6 rows, and never more than
    /// leaves the cell grid itself 4 rows) is carved off the bottom for
    /// [`View::screen`] to push a `Chart` widget into, the same "one
    /// more chrome band" carving `carve_tab_strip` already does for the
    /// sheet-tabs strip.
    fn carve_chart_panel(grid: Rect, chart_count: usize) -> (Rect, Option<Rect>) {
        if chart_count == 0 {
            return (grid, None);
        }
        let panel_h = (grid.height / 3.0)
            .max(6.0)
            .min((grid.height - 4.0).max(0.0));
        if panel_h <= 0.0 {
            return (grid, None);
        }
        let cells_h = grid.height - panel_h;
        let cells = Rect::new(grid.x, grid.y, grid.width, cells_h);
        let panel = Rect::new(grid.x, grid.y + cells_h, grid.width, panel_h);
        (cells, Some(panel))
    }

    /// `Self::bands(backend)` with `grid` narrowed to leave room for
    /// `sidebar_rect` on the right when `self.sidebar` is open —
    /// resolved once here so `grid_rect`/`tab_bar_rect`/`render` can't
    /// disagree on how much width the sidebar actually took.
    fn content_bands(&self, backend: &dyn Backend) -> Bands {
        let mut bands = Self::bands(backend);
        if let Some(sidebar) = self.sidebar_rect(backend) {
            bands.grid.width -= sidebar.width;
        }
        bands
    }

    /// `(grid rect, tab-strip rect)`: `formula_bar::bands` only knows
    /// about the two chrome bands every backend's `Bands` struct has
    /// room for; this app is the only one that needs a third strip, so
    /// it carves it here instead of adding a field there. The strip
    /// takes `bands.status`'s own height off the bottom of `bands.grid`
    /// and sits directly above the status bar — the same "one chrome
    /// row" sizing the formula/status bars already use.
    fn carve_tab_strip(bands: Bands) -> (Rect, Rect) {
        let tabs_h = bands.status.height.min(bands.grid.height);
        let grid_h = bands.grid.height - tabs_h;
        let grid = Rect::new(bands.grid.x, bands.grid.y, bands.grid.width, grid_h);
        let tabs = Rect::new(
            bands.grid.x,
            bands.grid.y + grid_h,
            bands.grid.width,
            tabs_h,
        );
        (grid, tabs)
    }

    /// The Name Manager sidebar's rect, carved off the right edge of
    /// `Self::bands(backend).grid` — the same band `carve_tab_strip`
    /// carves its own strip out of, just the other axis. `None` while
    /// `self.sidebar` is closed, so every caller that narrows around
    /// this (`content_bands`, `render`, `handle`'s own click routing)
    /// leaves the grid exactly as wide as it was before the sidebar
    /// existed.
    fn sidebar_rect(&self, backend: &dyn Backend) -> Option<Rect> {
        if !self.sidebar.is_open() {
            return None;
        }
        let grid = Self::bands(backend).grid;
        let width = Self::resolve_sidebar_width(grid.width, Self::is_tui_shaped(backend));
        Some(Rect::new(
            grid.x + grid.width - width,
            grid.y,
            width,
            grid.height,
        ))
    }

    /// The pure formula behind [`Self::sidebar_rect`]'s width, split
    /// out so it's unit-testable on plain floats — same reasoning as
    /// [`Self::resolve_cell_size`]. A fixed-ish fraction of the grid
    /// band's own width, floored at a size that still fits the
    /// toolbar's title/count/close button and a readable name list,
    /// and never wider than the band itself.
    fn resolve_sidebar_width(available_width: f32, tui_shaped: bool) -> f32 {
        let floor = if tui_shaped { 36.0 } else { 220.0 };
        (available_width * 0.4).max(floor).min(available_width)
    }

    /// The command palette's popup rect: centred, a fixed fraction of
    /// the viewport (mirrors `quadraui-gallery`'s own
    /// `PaletteDemo::popup_rect`), floored at a size the primitive can
    /// still paint a title/query row plus a few items in.
    fn palette_rect(backend: &dyn Backend) -> Rect {
        let vp = backend.viewport();
        let lh = backend.line_height().max(1.0);
        // Centered within the content area below the menu bar, not the
        // full viewport — added once the menu bar introduced a 4th
        // chrome row `bands()` already accounts for; this is the same
        // "below the bar" region.
        let menu_h = Self::menu_bar_rect(backend).height;
        let content_h = (vp.height - menu_h).max(0.0);
        let w = (vp.width * 0.7).max(30.0).min(vp.width);
        let h = (content_h * 0.6).max(lh * 8.0).min(content_h);
        Rect::new((vp.width - w) / 2.0, menu_h + (content_h - h) / 2.0, w, h)
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

    /// `(row_height, col_width)` to write onto `CellGrid::default_row_height`/
    /// `CellGrid::default_col_width`, derived from the backend's own
    /// chrome metrics. Those fields are raw surface-native
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
    /// only places with a `backend` to resolve `CellGrid::layout` against.
    /// Also the one place that retitles the window/terminal
    /// (`backend.window()`), since it is the same "did a command just
    /// run" cue the rest of the view uses.
    fn refresh(&mut self, backend: &mut dyn Backend) {
        // Resolved before `grid_rect` below reads `self.view.charts.len()`
        // to size the cell-grid rect it carves a chart panel out of — a
        // chart inserted this very frame must narrow that rect on the
        // same frame, not lag one behind the panel `View::screen` paints
        // it into once `self.view` is rebuilt further down.
        self.view.charts = chart_view::charts(&mut self.session);
        let (row_height, col_width) = Self::cell_size(backend);
        self.grid.default_row_height = row_height;
        self.grid.default_col_width = col_width;
        // Frozen panes, gridlines and column widths are the active
        // sheet's own (saved with the workbook, set through View/Home
        // commands); zebra striping is this app's.
        if let Some(sheet_view) = SheetView::read(&self.session, col_width) {
            sheet_view.apply(&mut self.grid);
        }
        self.grid.striped = self.striped;
        let rect = self.grid_rect(backend);
        let layout = Self::grid_layout(&self.grid, backend, rect).layout;
        // A trailing space on each visible row's number, so it doesn't
        // run straight into column A's text: the TUI painter
        // right-aligns row numbers flush against the gutter's edge.
        if Self::is_tui_shaped(backend) {
            self.grid.row_labels = layout
                .rows
                .iter()
                .map(|r| (r.index, format!("{} ", r.index + 1)))
                .collect();
        }
        let cells: Vec<Vec<Cell>> = layout
            .rows
            .iter()
            .map(|r| {
                layout
                    .cols
                    .iter()
                    .map(|c| self.grid_cell(&Address::new(r.index as u32, c.index as u32).a1()))
                    .collect()
            })
            .collect();
        // `row_scroll`/`col_scroll` are ranks *past* the frozen prefix,
        // but `loaded_row_start`/`loaded_col_start` are absolute sheet
        // indices (see `CellGrid::cells_window`'s doc) — add the frozen
        // count back so the two conventions agree. Mirrors quadraui's own
        // `examples/common/cell_grid_demo.rs::CellGridDemo::render`.
        self.grid.loaded_row_start =
            self.grid.frozen_rows.min(self.grid.row_count) + self.grid.row_scroll;
        self.grid.loaded_col_start =
            self.grid.frozen_cols.min(self.grid.col_count) + self.grid.col_scroll;
        self.grid.cells = cells;
        let grid = self.grid.clone();

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

        // `selection.stats` reads the engine's own `d.selection.ranges`,
        // not `self.grid`'s own — `sync_selection` is what normally
        // keeps those two in step for palette/clipboard commands (see its
        // own doc), and the status bar needs it just as much, every
        // frame, so it doesn't report last frame's selection or default
        // to the engine's `A1`.
        let _ = self.sync_selection();
        let selection_stats = self
            .execute("selection.stats", json!({}))
            .map(|stats| Self::selection_stats_label(&stats))
            .unwrap_or_default();

        // `TabBar`'s contract (`TabBarPrimitiveLayout::
        // corrected_scroll_offset`'s doc): resolve this frame's scroll
        // offset against the backend's own measurements and write the
        // corrected value back before this same frame paints, rather
        // than waiting for the next one — an `activate_sheet` that lands
        // past the bar's visible range must make that tab visible
        // immediately, not after one more event. `layout.
        // resolved_scroll_offset` is not what computes this: it is only
        // the stored offset clamped to a valid index, not fitted to the
        // active tab.
        let tab_bar_rect = self.tab_bar_rect(backend);
        let provisional_bar = sheets::tab_bar(&self.session, self.tab_scroll);
        self.tab_scroll = backend
            .layout_primitive(tab_bar_rect, &provisional_bar)
            .corrected_scroll_offset;
        let tab_bar = sheets::tab_bar(&self.session, self.tab_scroll);

        // Re-fetched only while open — see `View::sidebar_names`'s own
        // doc for why a closed sidebar leaves this empty instead of
        // paying for the query every frame.
        let sidebar_names = if self.sidebar.is_open() {
            sidebar::fetch_names(&mut self.session)
        } else {
            Vec::new()
        };

        // Moved out rather than recomputed: the early assignment at the
        // top of this function already resolved it against this frame's
        // engine state, and `chart_view::charts` isn't cheap enough (one
        // `sheet.read` per series) to pay for twice a frame.
        let charts = std::mem::take(&mut self.view.charts);

        // An open find panel's matches follow edits, undo and sheet
        // switches, and its "n of m" follows the cursor.
        if self.find.is_some() {
            self.search(false, backend);
        }

        // Re-classify against the engine's current state — e.g. "Undo"
        // greys out the instant there's nothing left to undo. Cheap
        // enough to rebuild every refresh (same cost the palette already
        // pays every time it opens) rather than tracking what changed.
        self.app_menu
            .set_menus(menu::menus_for(&self.session, self.striped));

        self.view = View {
            grid: Some(grid),
            tab_bar: Some(tab_bar),
            charts,
            sidebar_names,
            selection_label,
            selection_stats,
            active_input,
            active_text,
            formula,
            error: self.error.clone(),
            mode: self.mode(),
            tui_shaped: Self::is_tui_shaped(backend),
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

    /// What the keyboard is driving right now — see [`Mode`].
    fn mode(&self) -> Mode {
        if self.find.as_ref().is_some_and(|f| f.focused) {
            Mode::Find
        } else if self.grid.editing.is_some() {
            Mode::Edit
        } else {
            Mode::Ready
        }
    }

    /// Formats `selection.stats`'s result (`{count, numeric, sum, min,
    /// max, average}`, `average` null when `numeric` is `0`) as Excel's
    /// own status-bar summary: `Average: … Count: … Sum: …` once the
    /// selection has at least one number, falling back to just `Count:
    /// …` for a selection that's all text/empty, same as Excel. An empty
    /// selection (`count == 0`) reports nothing, so `status_surface`
    /// paints no segment at all rather than a bare `Count: 0`.
    fn selection_stats_label(stats: &Value) -> String {
        let count = stats.get("count").and_then(Value::as_u64).unwrap_or(0);
        if count == 0 {
            return String::new();
        }
        let numeric = stats.get("numeric").and_then(Value::as_u64).unwrap_or(0);
        if numeric == 0 {
            return format!("Count: {count}");
        }
        let sum = stats.get("sum").and_then(Value::as_f64).unwrap_or(0.0);
        let average = stats.get("average").and_then(Value::as_f64).unwrap_or(0.0);
        format!(
            "Average: {}   Count: {count}   Sum: {}",
            Self::format_stat_number(average),
            Self::format_stat_number(sum)
        )
    }

    /// A whole number with no decimals, otherwise two — this is a status-
    /// bar summary, not a cell value, so it doesn't go through the
    /// engine's cell-format machinery the way `cell_text` does.
    fn format_stat_number(n: f64) -> String {
        if n.fract() == 0.0 && n.abs() < 1e15 {
            format!("{n:.0}")
        } else {
            format!("{n:.2}")
        }
    }
}

impl Default for CalcApp {
    fn default() -> Self {
        Self::new()
    }
}

/// [`EventHost`] impl backing `CalcApp`'s own hand-rolled dispatch into
/// [`CellGrid::handle`] (`CalcApp::dispatch_to_grid`) — mirrors
/// quadraui's own `examples/common/cell_grid_demo.rs::DemoHost`: this app
/// has no hit-test/focus-cycling router of its own for `request_focus`/
/// `capture_pointer`/`release_pointer`/`set_cursor`/`request_redraw` to
/// report back to (every event `dispatch_to_grid` is handed has already
/// been resolved to belong to the grid by `CalcApp::handle`'s own band
/// checks), so only [`Self::emit`] (an extra `UiEvent::CellGrid` a
/// commit's `cx.emit` call attaches alongside its own
/// `CellEditCommitted`) carries anything real.
struct WidgetHost {
    units: Units,
    emitted: Vec<UiEvent>,
}

impl WidgetHost {
    fn new(units: Units) -> Self {
        Self {
            units,
            emitted: Vec::new(),
        }
    }
}

impl EventHost for WidgetHost {
    fn is_focused(&self, _id: &WidgetId, _sub: Option<SubTarget>) -> bool {
        false
    }

    fn request_focus(&mut self, _id: &WidgetId, _sub: Option<SubTarget>) {}

    fn capture_pointer(&mut self, _id: &WidgetId) {}

    fn release_pointer(&mut self) {}

    fn set_cursor(&mut self, _cursor: PointerShape) {}

    fn request_redraw(&mut self) {}

    fn emit(&mut self, event: UiEvent) {
        self.emitted.push(event);
    }

    fn units(&self) -> Units {
        self.units
    }
}

impl View {
    /// The whole screen as `ScreenLayout` surfaces, back to front:
    /// formula bar, grid, chart panel, tab strip, status bar.
    fn screen(&self, bands: Bands, name_box_width: f32, char_width: f32) -> ScreenLayout {
        let mut layout = ScreenLayout::new();
        let (name_box, field) = CalcApp::split_formula_band(bands.formula, name_box_width);
        layout.push(Surface::Widget {
            rect: name_box,
            widget: Box::new(self.name_box()),
        });
        layout.push(Surface::Widget {
            rect: field,
            widget: Box::new(self.formula.clone()),
        });
        let (grid_rect, tab_bar_rect) = CalcApp::carve_tab_strip(bands);
        let (cell_rect, chart_rect) = CalcApp::carve_chart_panel(grid_rect, self.charts.len());
        if let Some(grid) = &self.grid {
            layout.push(Surface::Widget {
                rect: cell_rect,
                widget: Box::new(grid.clone()),
            });
        }
        // Stacked evenly within the carved-off panel — see
        // `CalcApp::carve_chart_panel`'s own doc for why there's a rect
        // here at all exactly when `self.charts` isn't empty.
        if let Some(panel) = chart_rect {
            let each = panel.height / self.charts.len() as f32;
            for (i, chart) in self.charts.iter().enumerate() {
                layout.push(Surface::Widget {
                    rect: Rect::new(panel.x, panel.y + each * i as f32, panel.width, each),
                    widget: Box::new(chart.clone()),
                });
            }
        }
        if let Some(bar) = &self.tab_bar {
            layout.push(Surface::Widget {
                rect: tab_bar_rect,
                widget: Box::new(bar.clone()),
            });
        }
        layout.push(self.status_surface(bands.status, char_width));
        layout
    }

    /// The formula bar's leading name box: the cursor's address (or the
    /// selected range) and an `fx` marker, the way a spreadsheet labels
    /// the field beside it.
    fn name_box(&self) -> StatusBar {
        StatusBar {
            id: WidgetId::new("name-box"),
            left_segments: vec![
                StatusBarSegment {
                    text: format!(" {:<8} ", self.selection_label),
                    fg: Color::rgb(255, 255, 255),
                    bg: Color::rgb(31, 78, 121),
                    bold: true,
                    action_id: None,
                },
                StatusBarSegment {
                    text: " fx ".into(),
                    fg: Color::rgb(160, 190, 220),
                    bg: Color::rgb(24, 40, 60),
                    bold: false,
                    action_id: None,
                },
            ],
            right_segments: vec![],
        }
    }

    /// The status bar: mode badge, selection, the cursor cell's formula
    /// and value (or the last error) on the left; key hints and the
    /// selection summary on the right. The hints only show while
    /// everything fits in `rect` at `char_width` per character — they
    /// are the first thing to give way to an error or a long formula.
    fn status_surface(&self, rect: Rect, char_width: f32) -> Surface {
        let detail = match (&self.error, self.active_input.starts_with('=')) {
            (Some(err), _) => format!(" {err} "),
            (None, true) => format!(" {}  →  {} ", self.active_input, self.active_text),
            (None, false) => format!(" {} ", self.active_input),
        };
        let bar_bg = Color::rgb(33, 66, 110);
        let badge_bg = match (self.error.is_some(), self.mode) {
            (true, _) => Color::rgb(160, 40, 40),
            (false, Mode::Ready) => Color::rgb(20, 45, 80),
            (false, Mode::Edit) => Color::rgb(140, 90, 10),
            (false, Mode::Find) => Color::rgb(30, 110, 70),
        };
        let bar = StatusBar {
            id: WidgetId::new("status"),
            left_segments: vec![
                StatusBarSegment {
                    text: self.mode.badge().into(),
                    fg: Color::rgb(255, 255, 255),
                    bg: badge_bg,
                    bold: true,
                    action_id: None,
                },
                StatusBarSegment {
                    text: format!(" {} ", self.selection_label),
                    fg: Color::rgb(255, 255, 255),
                    bg: Color::rgb(20, 45, 80),
                    bold: true,
                    action_id: None,
                },
                StatusBarSegment {
                    text: detail.clone(),
                    fg: Color::rgb(230, 230, 230),
                    bg: bar_bg,
                    bold: false,
                    action_id: None,
                },
            ],
            right_segments: {
                // Least-important first, most-important (highest
                // priority to stay visible on a narrow bar) last — see
                // `StatusBarSegment`'s own doc on `right_segments`
                // ordering. The selection summary is the one Excel users
                // actually look for here, so it goes last/rightmost; the
                // key hints yield first on a narrow bar.
                let stats = if self.selection_stats.is_empty() {
                    String::new()
                } else {
                    format!(" {} ", self.selection_stats)
                };
                let used = self.mode.badge().chars().count()
                    + self.selection_label.chars().count()
                    + 2
                    + detail.chars().count()
                    + stats.chars().count()
                    + self.mode.hints(self.tui_shaped).chars().count();
                let mut right = Vec::new();
                if used as f32 * char_width.max(1.0) <= rect.width {
                    right.push(StatusBarSegment {
                        text: self.mode.hints(self.tui_shaped).into(),
                        fg: Color::rgb(200, 210, 225),
                        bg: bar_bg,
                        bold: false,
                        action_id: None,
                    });
                }
                if !stats.is_empty() {
                    right.push(StatusBarSegment {
                        text: stats,
                        fg: Color::rgb(255, 255, 255),
                        bg: Color::rgb(20, 45, 80),
                        bold: false,
                        action_id: None,
                    });
                }
                right
            },
        };
        Surface::Widget {
            rect,
            widget: Box::new(bar),
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
        let bands = self.content_bands(backend);
        let mut layout =
            self.view
                .screen(bands, Self::name_box_width(backend), backend.char_width());
        // Docked chrome, same z-order tier as the grid/tab strip
        // `screen` just pushed — not an overlay, so it paints before
        // the palette/format-picker checks below rather than after.
        if let Some(rect) = self.sidebar_rect(backend) {
            let panel = sidebar::toolbar_panel(self.view.sidebar_names.len());
            // The same `layout_primitive` call `handle_sidebar_click`
            // hit-tests against, so the list's rect can never silently
            // drift from what's actually painted.
            let panel_layout = backend.layout_primitive(rect, &panel);
            layout.push(Surface::Widget {
                rect,
                widget: Box::new(panel),
            });
            layout.push(Surface::Widget {
                rect: panel_layout.content_bounds,
                widget: Box::new(sidebar::list_view(
                    &self.view.sidebar_names,
                    self.sidebar.selected(),
                )),
            });
        }
        // The find panel floats over the grid's top-right corner, under
        // any modal overlay below.
        if let Some(bar) = &self.find {
            let group = self.grid_rect(backend);
            let panel = bar.panel_in(group);
            layout.push(Surface::Widget {
                rect: panel.anchored_rect(backend, group),
                widget: Box::new(panel),
            });
        }
        // On top of everything else — the palette is a modal overlay,
        // so it paints last, same as `ScreenLayout`'s own
        // back-to-front push order for every other overlay surface.
        if let Some(palette) = &self.palette {
            layout.push(Surface::Widget {
                rect: Self::palette_rect(backend),
                widget: Box::new(palette.palette()),
            });
        }
        // On top of the palette too (the two are mutually exclusive —
        // see `handle`'s own modal-routing order — but pushed last
        // regardless, same "overlay paints last" rule as the palette
        // itself). Pushed through the same `ScreenLayout` as everything
        // else — see `FormatPicker::render`'s own doc — so one `draw`
        // call below captures it for the accessibility tree and the MCP
        // frame-capture surface too.
        if let Some(picker) = &self.format_picker {
            picker.render(&mut layout, backend);
        }
        // Mutually exclusive with both the palette and the format picker
        // (`handle`'s own modal-routing order again), so "paints last"
        // never actually layers it over either in practice — same
        // `Palette` widget path as `palette` above, since this is the
        // same primitive over a different list.
        if let Some(picker) = &self.function_picker {
            layout.push(Surface::Widget {
                rect: Self::palette_rect(backend),
                widget: Box::new(picker.palette()),
            });
        }
        let menu_bar_rect = Self::menu_bar_rect(backend);
        // `AppMenu::build_menu_bar()` returns the bar's full declarative
        // descriptor — every item, each carrying its *entire* submenu
        // tree regardless of whether a dropdown is actually open — so
        // pushing it is what keeps the menu (not just the grid/status
        // bar) visible to the accessibility tree and to agents over MCP,
        // per this repo's CLAUDE.md: "Paint through ScreenLayout...
        // Don't call backend.draw_* directly." The bar, not the
        // `AppMenu` itself: an `AppMenu` pushed as a widget would also
        // queue its dropdown as a popup, which the runner skips while
        // the dropdown sits on the modal stack anyway (see below).
        layout.push(Surface::Widget {
            rect: menu_bar_rect,
            widget: Box::new(self.app_menu.build_menu_bar()),
        });
        // The grid's context menu, over everything but the menu bar's
        // own dropdown (the two never show at once).
        if let Some((menu, rect)) = &self.context_menu {
            layout.push(Surface::Widget {
                rect: *rect,
                widget: Box::new(menu.clone()),
            });
        }
        let _ = layout.draw(backend);
        // The one thing the bar above can't carry: an open dropdown's
        // actual cascade. `AppMenu::handle_ui_event` puts an open
        // dropdown on the modal stack, and the runner's popup pass
        // paints nothing while a modal is open, so `AppMenu::render`
        // paints it (and repaints the bar itself — identical pixels).
        // Painted last, same "topmost overlay" reasoning as the palette
        // above — an open dropdown must sit over the grid/status bar
        // it's anchored above.
        self.app_menu.render(backend, menu_bar_rect);
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
                    // The same dispatch a menu click takes — see
                    // `run_command_id`.
                    self.run_command_id(id, backend)
                }
                PaletteOutcome::Pick(id) => {
                    self.palette = None;
                    self.format_picker = FormatPicker::open(id);
                    Reaction::Redraw
                }
                PaletteOutcome::PickFunction(id) => {
                    self.palette = None;
                    match self.open_function_picker(id) {
                        Ok(picker) => self.function_picker = Some(picker),
                        Err(e) => self.error = Some(e.to_string()),
                    }
                    self.refresh(backend);
                    Reaction::Redraw
                }
                PaletteOutcome::Ignored => Reaction::Continue,
            };
        }
        // The format picker is modal exactly like the palette above —
        // every event (including a resize, so the grid behind it stays
        // sized correctly once it closes) routes here instead of
        // falling through while it's open.
        if let Some(picker) = self.format_picker.as_mut() {
            if matches!(event, UiEvent::WindowResized { .. }) {
                self.refresh(backend);
                return Reaction::Continue;
            }
            return match picker.handle_event(&event, backend) {
                PickerOutcome::Changed => Reaction::Redraw,
                PickerOutcome::Closed => {
                    self.format_picker = None;
                    Reaction::Redraw
                }
                PickerOutcome::Run(id, params) => {
                    self.format_picker = None;
                    // The selection was already synced when the palette
                    // opened this picker (`PaletteOutcome::Pick` runs
                    // right after `sync_selection`'s own palette-open
                    // call); re-running it here is a no-op in practice,
                    // but keeps this call site self-contained rather
                    // than relying on that ordering.
                    if let Err(e) = self
                        .sync_selection()
                        .and_then(|()| self.execute(id, params))
                    {
                        self.error = Some(e.to_string());
                    }
                    self.refresh(backend);
                    Reaction::Redraw
                }
                PickerOutcome::Ignored => Reaction::Continue,
            };
        }
        // The function picker is modal exactly like the palette and the
        // format picker above — every event routes here instead of the
        // grid while it's open.
        if let Some(picker) = self.function_picker.as_mut() {
            if matches!(event, UiEvent::WindowResized { .. }) {
                self.refresh(backend);
                return Reaction::Continue;
            }
            let UiEvent::KeyPressed { key, modifiers, .. } = &event else {
                return Reaction::Continue;
            };
            return match picker.handle_key(key, modifiers) {
                FunctionPickerOutcome::Changed => Reaction::Redraw,
                FunctionPickerOutcome::Closed => {
                    self.function_picker = None;
                    Reaction::Redraw
                }
                FunctionPickerOutcome::Insert(name) => {
                    self.function_picker = None;
                    self.insert_function_call(&name, backend);
                    self.refresh(backend);
                    Reaction::Redraw
                }
                FunctionPickerOutcome::Ignored => Reaction::Continue,
            };
        }
        // The grid's context menu is modal while open, like the palette.
        if self.context_menu.is_some() {
            return self.handle_context_menu_event(&event, backend);
        }
        // The menu bar takes priority over everything below — Alt+key
        // mnemonics and in-dropdown arrow/Enter navigation must work no
        // matter what else is focused, the same precedence `quadraui`'s
        // own `AppMenu` example gives it. While a dropdown is open
        // this also swallows every mouse event, the same modal
        // interception the palette above gives itself; `Ignored` (the
        // overwhelming majority of events — typing into a cell, grid
        // navigation) falls through to everything below unchanged.
        //
        // Guarded by "outside an active edit" like every other shortcut
        // in this function: `AppMenu::handle_ui_event` has no notion of
        // whether a cell/formula-bar edit is in progress, so an
        // unconditional call would let Alt+<mnemonic letter> open a
        // dropdown mid-edit, leaving that edit's buffer dangling with no
        // commit/cancel path reconciled against the now-open menu. A
        // mouse click on the bar itself is guarded the same way other
        // chrome clicks are below (the tab-strip click check has the
        // same guard) — clicking chrome while editing is a no-op here,
        // not a commit-then-proceed.
        if self.grid.editing.is_none() && !self.formula_focused {
            let menu_bar_rect = Self::menu_bar_rect(backend);
            match self.app_menu.handle_ui_event(
                &mut self.app_menu_state,
                &event,
                backend,
                menu_bar_rect,
            ) {
                Handled::Event(AppMenuEvent::Activated(id)) => {
                    return self.run_command_id(id.as_str(), backend);
                }
                Handled::Event(_) | Handled::Consumed => {
                    self.refresh(backend);
                    return Reaction::Redraw;
                }
                Handled::Ignored => {}
            }
        }
        // Ctrl+F / Ctrl+H (Cmd on macOS) open find / find-and-replace
        // over the sheet — or, already open, take the keyboard back.
        // Outside an active edit, like every other shortcut here.
        if self.grid.editing.is_none()
            && let UiEvent::KeyPressed { key, modifiers, .. } = &event
            && (modifiers.ctrl || modifiers.cmd)
            && !modifiers.shift
            && matches!(key, Key::Char('f' | 'F' | 'h' | 'H'))
        {
            self.open_find(matches!(key, Key::Char('h' | 'H')), backend);
            self.refresh(backend);
            return Reaction::Redraw;
        }
        if self.find.is_some()
            && self.grid.editing.is_none()
            && !self.formula_focused
            && let Some(reaction) = self.route_to_find(&event, backend)
        {
            return reaction;
        }
        // Ctrl+Shift+P (Cmd+Shift+P on macOS), or a bare `:` on a
        // TUI-shaped backend, opens the palette — outside an active
        // edit, so neither shortcut fights with typing a literal `:`
        // or capital `P` into a cell.
        if self.grid.editing.is_none()
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
                // `AppMenu::handle_ui_event` above routes no closed-menu
                // key but an Alt mnemonic, and no open-menu key matches
                // this chord either,
                // so it falls through here `Ignored` even while a
                // dropdown is open (no mnemonic/arrow/Enter/mouse
                // shape matches it). Without this, the menu's own
                // `open_item` would stay `Some` underneath — and, once
                // the palette later closes, interactive again — while
                // the palette is modally open on top of it.
                self.app_menu.dismiss(backend);
                self.palette = Some(CommandPalette::open(&self.session));
                return Reaction::Redraw;
            }
        }
        // Ctrl+F3 (Cmd+F3 on macOS) toggles the Name Manager sidebar —
        // see `crate::sidebar`'s module doc for why that's GridCraft's
        // own shortcut for `formulas.nameManager` rather than one this
        // app picked on its own. Same "outside an active edit" guard as
        // every shortcut above.
        if self.grid.editing.is_none()
            && let UiEvent::KeyPressed { key, modifiers, .. } = &event
            && (modifiers.ctrl || modifiers.cmd)
            && matches!(key, Key::Named(NamedKey::F(3)))
        {
            self.sidebar.toggle();
            self.refresh(backend);
            return Reaction::Redraw;
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
            && Self::split_formula_band(Self::bands(backend).formula, Self::name_box_width(backend))
                .1
                .contains(*position)
        {
            if self.grid.editing.is_none() {
                let a1 = self.cursor().a1();
                let text = self.cell_input(&a1);
                self.start_editing(text);
            }
            self.formula_focused = true;
            // The press itself places the caret (and starts a drag-select)
            // through the field's own pointer handling.
            self.route_to_formula_field(&event, backend);
            self.refresh(backend);
            return Reaction::Redraw;
        }
        // A drag that started in the formula bar keeps extending its
        // selection until the button comes up, wherever the pointer goes.
        if self.formula_focused
            && self.formula_state.dragging
            && matches!(event, UiEvent::MouseMoved { .. } | UiEvent::MouseUp { .. })
        {
            self.route_to_formula_field(&event, backend);
            self.refresh(backend);
            return Reaction::Redraw;
        }
        // A click on the sheet-tabs strip switches sheets, resolved
        // against the same `TabBar` `View::screen` just painted so the
        // hit always matches what's on screen this frame. Guarded by
        // "outside an active edit" like every other shortcut below, so a
        // stray click there doesn't abandon an in-progress edit.
        if self.grid.editing.is_none()
            && let UiEvent::MouseDown {
                button: MouseButton::Left,
                position,
                ..
            } = &event
        {
            let rect = self.tab_bar_rect(backend);
            if rect.contains(*position) {
                if let Some(bar) = self.view.tab_bar.clone() {
                    let local_x = position.x - rect.x;
                    let local_y = position.y - rect.y;
                    if let TabBarHit::Tab(i) = backend
                        .layout_primitive(rect, &bar)
                        .layout
                        .hit_test(local_x, local_y)
                        && let Err(e) = self.activate_sheet(i)
                    {
                        self.error = Some(e.to_string());
                    }
                }
                self.refresh(backend);
                return Reaction::Redraw;
            }
        }
        // A click inside the Name Manager sidebar — the toolbar's close
        // button, or a row in the name list — routed through the same
        // `SidebarPanel`/`ListView` layouts `render` just painted (see
        // `handle_sidebar_click`'s own doc). Guarded by "outside an
        // active edit" and checked before the grid's own dispatch below,
        // same reasoning as the sheet-tabs click above: without this, a
        // click here would fall through to `self.grid.handle` and be
        // mis-resolved against a grid column that isn't actually under
        // the cursor once the sidebar has narrowed the grid's own rect.
        if self.grid.editing.is_none()
            && let UiEvent::MouseDown {
                button: MouseButton::Left,
                position,
                ..
            } = &event
            && let Some(rect) = self.sidebar_rect(backend)
            && rect.contains(*position)
        {
            self.handle_sidebar_click(*position, rect, backend);
            self.refresh(backend);
            return Reaction::Redraw;
        }
        // Right-click on the grid (or Shift+F10 / the Menu key) opens
        // its context menu — outside an active edit, so it never
        // abandons one.
        if self.grid.editing.is_none() && !self.formula_focused {
            if let UiEvent::MouseDown {
                button: MouseButton::Right,
                position,
                ..
            } = &event
                && self.grid_rect(backend).contains(*position)
            {
                self.open_context_menu_at_pointer(*position, backend);
                self.refresh(backend);
                return Reaction::Redraw;
            }
            if let UiEvent::KeyPressed { key, modifiers, .. } = &event
                && (matches!(key, Key::Named(NamedKey::Menu))
                    || (modifiers.shift && matches!(key, Key::Named(NamedKey::F(10)))))
            {
                self.open_context_menu_at_cursor(backend);
                self.refresh(backend);
                return Reaction::Redraw;
            }
        }
        // The wheel scrolls the grid under the pointer without moving the
        // cursor, the way a spreadsheet does.
        if let UiEvent::Scroll {
            delta, position, ..
        } = &event
            && self.grid_rect(backend).contains(*position)
        {
            self.scroll_grid(delta.x, delta.y);
            self.refresh(backend);
            return Reaction::Redraw;
        }
        // Double-click a cell to edit it in place, like F2.
        if self.grid.editing.is_none()
            && let UiEvent::DoubleClick { position, .. } = &event
        {
            let rect = self.grid_rect(backend);
            if rect.contains(*position) {
                let layout = Self::grid_layout(&self.grid, backend, rect);
                if let CellGridHit::Cell(row, col) = layout.hit_test(position.x, position.y) {
                    self.grid.cursor = (row, col);
                    self.grid.selection_anchor = None;
                    let text = self.cell_input(&Address::new(row as u32, col as u32).a1());
                    self.start_editing(text);
                }
                self.refresh(backend);
                return Reaction::Redraw;
            }
        }
        // `q`/Escape quit only outside an active edit — while editing, `q`
        // is a character to insert and Escape cancels the edit instead
        // (`CellGrid::handle` routes both to the edit buffer while
        // `editing` is `Some`, same as quadraui's own `cell_grid_demo`).
        if self.grid.editing.is_none()
            && let UiEvent::KeyPressed { key, .. } = &event
            && matches!(key, Key::Char('q') | Key::Named(NamedKey::Escape))
        {
            return Reaction::Exit;
        }
        // Ctrl+S (Cmd+S on macOS) saves — also only outside an active
        // edit, so the shortcut never fights with typing a literal "s"
        // into a cell.
        if self.grid.editing.is_none()
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
        if self.grid.editing.is_none()
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
        // Ctrl+PageUp/PageDown (Cmd on macOS) switch sheets through
        // `sheet.previous`/`sheet.next` — GridCraft's own catalog
        // shortcuts for these ids (`gridcraft_engine::cmd::sheet`).
        // Checked before `self.grid.handle` further down, whose own
        // `PageUp`/`PageDown` arms page the grid regardless of
        // modifiers; same "outside an active edit" guard as every other
        // shortcut above.
        if self.grid.editing.is_none()
            && let UiEvent::KeyPressed { key, modifiers, .. } = &event
            && (modifiers.ctrl || modifiers.cmd)
        {
            type SheetSwitch = fn(&mut CalcApp) -> anyhow::Result<()>;
            let switch: Option<SheetSwitch> = if matches!(key, Key::Named(NamedKey::PageUp)) {
                Some(CalcApp::previous_sheet)
            } else if matches!(key, Key::Named(NamedKey::PageDown)) {
                Some(CalcApp::next_sheet)
            } else {
                None
            };
            if let Some(switch) = switch {
                if let Err(e) = switch(self) {
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
        // keypress before it ever reaches `handle` as a `KeyPressed` — it
        // arrives below as its own `UiEvent::ClipboardPaste` arm instead.
        if self.grid.editing.is_none()
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
        // `CellGrid::handle` has no notion of `WindowResized` (it owns no
        // viewport-size state of its own, only scroll/cursor), and the
        // conversion to `RoutedEvent` below has no arm for it either, so
        // `refresh` below would never run. Without this, `render` keeps
        // painting the cached `View::grid`, whose loaded window was sized
        // for the old viewport — enlarging the terminal or window leaves
        // the newly revealed rows/columns blank until the next cursor
        // move. Refresh explicitly instead.
        if let UiEvent::WindowResized { .. } = &event {
            self.refresh(backend);
            return Reaction::Redraw;
        }
        // Ctrl+V (Cmd on macOS): quadraui's runtime intercepts the native
        // paste keypress before it ever reaches `handle` as a `KeyPressed`
        // — it arrives here as `UiEvent::ClipboardPaste` instead, with no
        // `RoutedEvent` counterpart `CellGrid::handle` would recognize, so
        // this app resolves it directly. While an edit is active the
        // pasted text is inserted into its buffer, the same
        // `TextField::apply_edit_op` bridge typing a character uses
        // (`Self::apply_edit_op`); with no edit active it's handed to
        // `edit.paste` at the selection's top-left corner, same anchor
        // convention a menu/palette paste (`clipboard::paste_from_system`)
        // resolves for an identical selection.
        if let UiEvent::ClipboardPaste(text) = &event {
            if self.grid.editing.is_some() {
                self.apply_edit_op(EditOp::InsertText(text.clone()));
                self.refresh(backend);
                return Reaction::Redraw;
            }
            if self.grid.row_count > 0 && self.grid.col_count > 0 {
                let (top, left, ..) = self.grid.selection_bounds();
                let anchor = Address::new(top as u32, left as u32).a1();
                if let Err(e) = clipboard::paste(self, &anchor, text) {
                    self.error = Some(e.to_string());
                }
                self.refresh(backend);
                return Reaction::Redraw;
            }
            return Reaction::Continue;
        }
        // Alt+<letter> is reserved for a menu-bar mnemonic — none exist
        // yet, but the `RoutedEvent` conversion below has no notion of
        // modifiers at all for a plain `Key::Char`, so one with `alt` set
        // would otherwise start an edit and insert that bare letter into
        // the active cell (e.g. Alt+A on an idle cell silently replacing
        // its content with "a"). Every other shortcut guard above already
        // checks `modifiers.ctrl || modifiers.cmd` before letting a
        // keypress fall through to typing; this is the same guard for
        // `alt`, swallowing the keypress instead of mistyping it. Outside
        // an active edit only, like every guard above — while editing,
        // Alt+<letter> reaches the edit buffer unchanged, same as any
        // other character.
        //
        // Narrowed to ASCII alphanumerics with no Ctrl held, because
        // `alt` is not only a mnemonic modifier on every backend this
        // crate runs on:
        // - On macOS, Option is the *composition* modifier (no Alt/Option
        //   menu mnemonics exist there at all — Cocoa menus use Cmd), so
        //   Option+<letter> arrives as `Key::Char` with `alt: true` and a
        //   composed non-ASCII character (e.g. Option+A -> 'å'). Only
        //   ASCII alphanumerics are plausible mnemonics, so a composed
        //   character always falls through to the grid untouched.
        // - On Windows, AltGr is reported as Ctrl+Alt, and AltGr-composed
        //   characters (e.g. '@', '{', '€' on European layouts) still
        //   arrive as a normal `Key::Char`. Requiring `!modifiers.ctrl`
        //   lets those reach the grid instead of being swallowed.
        if self.grid.editing.is_none()
            && let UiEvent::KeyPressed { key, modifiers, .. } = &event
            && modifiers.alt
            && !modifiers.ctrl
            && matches!(key, Key::Char(c) if c.is_ascii_alphanumeric())
        {
            return Reaction::Continue;
        }
        // Every other event the grid might care about — a key (whether
        // the formula bar or the grid itself visually owns the caret;
        // `CellGrid::handle` branches on `CellGrid::editing` alone, so
        // typing/committing/cancelling is identical either way, same as
        // quadraui's own `examples/common/cell_grid_demo.rs`), a click or
        // drag inside the grid body — converted to the `RoutedEvent`
        // shape `Interactive::handle` expects and dispatched through
        // `Self::dispatch_to_grid`. Anything else (a click elsewhere that
        // no earlier check claimed) is left alone.
        let rect = self.grid_rect(backend);
        let layout = Self::grid_layout(&self.grid, backend, rect);
        let routed = match &event {
            UiEvent::MouseDown {
                button: MouseButton::Left,
                position,
                modifiers,
                ..
            } if rect.contains(*position) => Some(RoutedEvent::Pointer {
                kind: PointerKind::Down(MouseButton::Left),
                sub: None,
                local: Point::new(position.x - rect.x, position.y - rect.y),
                mods: *modifiers,
            }),
            UiEvent::MouseMoved {
                position,
                buttons: ButtonMask { left: true, .. },
            } => Some(RoutedEvent::Pointer {
                kind: PointerKind::Move,
                sub: None,
                local: Point::new(position.x - rect.x, position.y - rect.y),
                mods: Modifiers::default(),
            }),
            UiEvent::MouseUp {
                button: MouseButton::Left,
                position,
                ..
            } => Some(RoutedEvent::Pointer {
                kind: PointerKind::Up(MouseButton::Left),
                sub: None,
                local: Point::new(position.x - rect.x, position.y - rect.y),
                mods: Modifiers::default(),
            }),
            UiEvent::KeyPressed {
                key,
                modifiers,
                repeat,
            } => Some(RoutedEvent::Key(KeyEvent {
                key: key.clone(),
                modifiers: *modifiers,
                repeat: *repeat,
            })),
            _ => None,
        };
        match routed {
            Some(routed) => {
                let reaction = self.dispatch_to_grid(routed, &layout, backend);
                if matches!(reaction, Reaction::Redraw) {
                    self.refresh(backend);
                }
                reaction
            }
            None => Reaction::Continue,
        }
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
    /// The unit system `backend` lays primitives out in — what
    /// [`CellGrid::handle`]'s host reports through
    /// [`EventHost::units`]. Resolved from the same chrome row height
    /// `DrawsPrimitives::layout_primitive` hands every widget, so it
    /// always agrees with [`Self::grid_layout`]. Mirrors quadraui's own
    /// `examples/common/cell_grid_demo.rs::layout_ctx`.
    fn units(backend: &dyn Backend) -> Units {
        Self::layout_ctx(backend).units()
    }

    /// A [`LayoutCtx`] matching the one `DrawsPrimitives::layout_primitive`
    /// builds — rebuilt here because quadraui keeps that one crate-private.
    fn layout_ctx(backend: &dyn Backend) -> LayoutCtx<'_> {
        LayoutCtx::new(
            LayoutCtx::resolve_row_height(backend.chrome_line_height(), &backend.style()),
            backend.char_width(),
            backend.nerd_fonts_enabled(),
        )
    }

    /// `grid`'s layout at `rect`, resolved through the backend's own
    /// `layout_primitive` — the same ctx (measurer, style, row-height
    /// floor) the `Surface::Widget` `View::screen` pushes is painted
    /// with, so a click or a viewport window computed here always
    /// matches what's on screen.
    fn grid_layout(grid: &CellGrid, backend: &dyn Backend, rect: Rect) -> CellGridPrimitiveLayout {
        backend.layout_primitive(rect, grid)
    }

    /// Route one already-converted [`RoutedEvent`] into `self.grid`'s own
    /// [`CellGrid::handle`] and apply whatever [`CellGridEvent`] it
    /// reports (plus every extra event [`WidgetHost::emit`] collected — see
    /// that type's own doc for why nothing else it carries matters here).
    /// Mirrors quadraui's own
    /// `examples/common/cell_grid_demo.rs::CellGridDemo::dispatch_to_grid`.
    fn dispatch_to_grid(
        &mut self,
        routed: RoutedEvent,
        layout: &CellGridPrimitiveLayout,
        backend: &dyn Backend,
    ) -> Reaction {
        let mut host = WidgetHost::new(Self::units(backend));
        let handled = {
            let mut cx = EventCx::new(self.grid.id, &mut host);
            self.grid
                .handle(&mut self.grid_state, &routed, layout, &mut cx)
        };
        let mut reaction = match handled {
            Handled::Event(event) => self.apply_grid_event(event),
            Handled::Consumed => Reaction::Redraw,
            Handled::Ignored => Reaction::Continue,
        };
        // A copy/cut out of the in-progress edit's selection.
        self.copy_emitted_text(&host.emitted, backend);
        let grid_id = self.grid.id;
        for extra in host.emitted {
            if let UiEvent::CellGrid(id, event) = extra
                && id == grid_id
            {
                reaction = reaction.merge(self.apply_grid_event(event));
            }
        }
        reaction
    }

    /// Apply one [`CellGridEvent`] from [`Self::dispatch_to_grid`]'s own
    /// `CellGrid::handle` call. `Interactive::handle` only ever has
    /// `&self` (that trait's own doc), so every navigation/edit outcome
    /// — a plain arrow, a click, a drag, F2, a commit, a cancel — comes
    /// back as an event instead of being applied in place; this writes
    /// it onto `self.grid`'s own fields, and a commit is also this app's
    /// cue to write the cell through the engine, the one way the UI ever
    /// changes a workbook.
    fn apply_grid_event(&mut self, event: CellGridEvent) -> Reaction {
        match event {
            CellGridEvent::Navigated {
                cursor,
                selection_anchor,
                row_scroll,
                col_scroll,
            } => {
                self.grid.cursor = cursor;
                self.grid.selection_anchor = selection_anchor;
                self.grid.row_scroll = row_scroll;
                self.grid.col_scroll = col_scroll;
                Reaction::Redraw
            }
            // F2: seed the inline editor with this cell's current raw
            // input — `CellGrid` has no cell content to seed it with
            // itself (see its module doc).
            CellGridEvent::CellEditRequested { row, col } => {
                let a1 = Address::new(row as u32, col as u32).a1();
                let text = self.cell_input(&a1);
                self.start_editing(text);
                Reaction::Redraw
            }
            // A fresh edit started (a plain keypress with none active
            // yet), or the in-progress one's text buffer changed
            // (insert/delete/select) — either way `edit` is what
            // `CellGrid::editing` becomes next frame.
            CellGridEvent::CellEditStarted { edit, .. }
            | CellGridEvent::CellEditChanged { edit, .. } => {
                self.grid.editing = Some(edit);
                Reaction::Redraw
            }
            // Enter/Tab/an arrow key committed the in-progress edit — hand
            // the raw text to the engine's own `cell.set`.
            CellGridEvent::CellEditCommitted { row, col, text } => {
                let a1 = Address::new(row as u32, col as u32).a1();
                if let Err(e) = self.set_cell(&a1, &text) {
                    self.error = Some(e.to_string());
                }
                self.grid.editing = None;
                self.formula_focused = false;
                self.formula_state = TextFieldState::default();
                Reaction::Redraw
            }
            // Escape cancelled the edit — whether it was typed through
            // the grid or the formula bar, neither owns the (now gone)
            // caret any more.
            CellGridEvent::CellEditCancelled { .. } => {
                self.grid.editing = None;
                self.formula_focused = false;
                self.formula_state = TextFieldState::default();
                Reaction::Redraw
            }
            // `CellsRequested` needs no action of its own: `Self::refresh`
            // always rebuilds `self.grid.cells` fresh for the current
            // `row_scroll`/`col_scroll` on the very next frame, never
            // trying to avoid that recompute the way a host windowing a
            // genuinely expensive data source might.
            _ => Reaction::Redraw,
        }
    }

    /// Open the find panel — with the replace row when `replace` — or,
    /// already open, give it the keyboard back. Any open context menu or
    /// menu dropdown closes first.
    fn open_find(&mut self, replace: bool, backend: &mut dyn Backend) {
        self.app_menu.dismiss(backend);
        match &mut self.find {
            Some(bar) => bar.refocus(replace),
            None => self.find = Some(FindBar::open(replace, String::new())),
        }
        self.search(false, backend);
    }

    /// Re-run the search for the panel's current query and options,
    /// jumping to the nearest match when `jump` (a query edit), and
    /// update the panel's match count.
    fn search(&mut self, jump: bool, backend: &mut dyn Backend) {
        let Some(params) = self.find.as_ref().map(FindBar::find_all_params) else {
            return;
        };
        let sheet = sheets::active_sheet_name(&self.session).unwrap_or_default();
        self.find_hits = match params {
            // `edit.find` errors on no match ("We couldn't find…"); that's
            // an empty list here, not an error to show.
            Some(params) => self
                .execute("edit.find", params)
                .map(|r| find::matches(&r, &sheet))
                .unwrap_or_default(),
            None => Vec::new(),
        };
        if jump && let Some(hit) = find::nearest(&self.find_hits, self.grid.cursor) {
            self.go_to_cell(hit, backend);
        }
        self.update_match_info();
    }

    fn update_match_info(&mut self) {
        let cursor = self.grid.cursor;
        if let Some(bar) = &mut self.find {
            bar.panel.match_info = bar.note.clone().unwrap_or_else(|| {
                find::match_info(&self.find_hits, cursor, bar.panel.query.is_empty())
            });
            bar.panel.rebuild_hit_regions();
        }
    }

    /// Step to the next (`forward`) or previous match.
    fn find_step(&mut self, forward: bool, backend: &mut dyn Backend) {
        if let Some(hit) = find::step(&self.find_hits, self.grid.cursor, forward) {
            self.go_to_cell(hit, backend);
        }
        self.update_match_info();
    }

    /// Replace the match under the cursor (then step to the next one), or
    /// every match when `all`, through `edit.replace`.
    fn find_replace(&mut self, all: bool, backend: &mut dyn Backend) {
        let Some(bar) = &self.find else { return };
        if bar.panel.query.is_empty() {
            return;
        }
        let params = bar.replace_params(all);
        if !all && !self.find_hits.contains(&self.grid.cursor) {
            self.find_step(true, backend);
            return;
        }
        let result = self
            .sync_selection()
            .and_then(|()| self.execute("edit.replace", params));
        match result {
            Ok(r) => {
                let replaced = r.get("replaced").and_then(Value::as_u64).unwrap_or(0);
                self.search(false, backend);
                if all {
                    if let Some(bar) = &mut self.find {
                        bar.note = Some(format!("Replaced {replaced}"));
                    }
                    self.update_match_info();
                } else {
                    self.find_step(true, backend);
                }
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    /// Move the cursor to `cell` (collapsing any selection) and scroll it
    /// into view — where a find step lands.
    fn go_to_cell(&mut self, cell: (usize, usize), backend: &dyn Backend) {
        self.grid.cursor = cell;
        self.grid.selection_anchor = None;
        self.reveal(cell, backend);
    }

    /// Scroll so `cell` is on screen. A cell in a frozen row/column is
    /// always on screen along that axis. `CellGrid` resolves this itself
    /// for its own navigation, but keeps that private, so this walks the
    /// scroll offset against the same layout `render` paints.
    fn reveal(&mut self, (row, col): (usize, usize), backend: &dyn Backend) {
        let rect = self.grid_rect(backend);
        let (fr, fc) = (self.grid.frozen_rows, self.grid.frozen_cols);
        if row >= fr && row < fr + self.grid.row_scroll {
            self.grid.row_scroll = row - fr;
        }
        if col >= fc && col < fc + self.grid.col_scroll {
            self.grid.col_scroll = col - fc;
        }
        // Bounded: each step brings the target at least one row/column
        // closer, and a sheet is at most `row_count`/`col_count` away.
        for _ in 0..(self.grid.row_count + self.grid.col_count) {
            let layout = Self::grid_layout(&self.grid, backend, rect).layout;
            let fully_visible = |spans: &[quadraui::ResolvedSpan], index: usize, extent: f32| {
                spans
                    .iter()
                    .any(|s| s.index == index && s.start + s.size <= extent + 0.01)
            };
            let row_ok = row < fr
                || fully_visible(&layout.rows, row, layout.viewport_height)
                || layout.rows.len() <= layout.frozen_rows + 1;
            let col_ok = col < fc
                || fully_visible(&layout.cols, col, layout.viewport_width)
                || layout.cols.len() <= layout.frozen_cols + 1;
            if row_ok && col_ok {
                break;
            }
            if !row_ok {
                self.grid.row_scroll += 1;
            }
            if !col_ok {
                self.grid.col_scroll += 1;
            }
        }
    }

    /// The find panel's box this frame: top-right of the grid.
    fn find_rect(&self, backend: &dyn Backend) -> Option<Rect> {
        let bar = self.find.as_ref()?;
        let group = self.grid_rect(backend);
        Some(bar.panel_in(group).anchored_rect(backend, group))
    }

    /// Route `event` to the open find panel when it's the panel's:
    /// a press inside its box (which also gives it the keyboard), and
    /// every key and paste while it has the keyboard. A press anywhere
    /// else hands the keyboard back to the grid and falls through; with
    /// the grid holding the keyboard, Escape closes the panel instead of
    /// quitting. `None` for an event the panel doesn't take.
    fn route_to_find(&mut self, event: &UiEvent, backend: &mut dyn Backend) -> Option<Reaction> {
        let rect = self.find_rect(backend)?;
        let focused = self.find.as_ref().is_some_and(|f| f.focused);
        let routed = match event {
            UiEvent::MouseDown {
                button: MouseButton::Left,
                position,
                modifiers,
                ..
            } if rect.contains(*position) => {
                if let Some(bar) = &mut self.find {
                    bar.focused = true;
                }
                RoutedEvent::Pointer {
                    kind: PointerKind::Down(MouseButton::Left),
                    sub: None,
                    local: Point::new(position.x - rect.x, position.y - rect.y),
                    mods: *modifiers,
                }
            }
            UiEvent::MouseDown { .. } => {
                if let Some(bar) = &mut self.find {
                    bar.focused = false;
                }
                return None;
            }
            UiEvent::WindowResized { .. } => return None,
            UiEvent::KeyPressed { key, .. }
                if !focused
                    && self.grid.editing.is_none()
                    && matches!(key, Key::Named(NamedKey::Escape)) =>
            {
                self.close_find();
                self.refresh(backend);
                return Some(Reaction::Redraw);
            }
            _ if !focused => return None,
            UiEvent::KeyPressed {
                key,
                modifiers,
                repeat,
            } => RoutedEvent::Key(KeyEvent {
                key: key.clone(),
                modifiers: *modifiers,
                repeat: *repeat,
            }),
            UiEvent::ClipboardPaste(text) => RoutedEvent::Text(text.clone()),
            UiEvent::CharTyped(c) => RoutedEvent::Text(c.to_string()),
            _ => return None,
        };
        let bar = self.find.as_ref()?;
        let panel = bar.panel_in(self.grid_rect(backend));
        let layout = backend.layout_primitive(rect, &panel);
        let mut host = WidgetHost::new(Self::units(backend));
        let handled = {
            let mut cx = EventCx::new(panel.id, &mut host);
            panel.handle(&mut (), &routed, &layout, &mut cx)
        };
        self.copy_emitted_text(&host.emitted, backend);
        let reaction = match handled {
            Handled::Event(e) => self.handle_find_event(e, backend),
            Handled::Consumed => Reaction::Redraw,
            Handled::Ignored => Reaction::Continue,
        };
        self.refresh(backend);
        Some(reaction)
    }

    fn handle_find_event(
        &mut self,
        event: FindReplaceEvent,
        backend: &mut dyn Backend,
    ) -> Reaction {
        if let Some(bar) = &mut self.find {
            bar.note = None;
        }
        match &event {
            FindReplaceEvent::Close => {
                self.close_find();
                return Reaction::Redraw;
            }
            FindReplaceEvent::NextMatch => self.find_step(true, backend),
            FindReplaceEvent::PrevMatch => self.find_step(false, backend),
            FindReplaceEvent::ReplaceCurrent => self.find_replace(false, backend),
            FindReplaceEvent::ReplaceAll => self.find_replace(true, backend),
            FindReplaceEvent::FieldEdited { .. } | FindReplaceEvent::Toggled { .. } => {}
        }
        let requery = match &event {
            FindReplaceEvent::FieldEdited { field, text, .. } => {
                *field == FindReplaceField::Find
                    && self.find.as_ref().is_some_and(|f| f.panel.query != *text)
            }
            FindReplaceEvent::Toggled { .. } => true,
            _ => false,
        };
        if let Some(bar) = &mut self.find {
            bar.panel.apply_event(&event);
        }
        if requery {
            self.search(true, backend);
        }
        Reaction::Redraw
    }

    fn close_find(&mut self) {
        self.find = None;
        self.find_hits.clear();
    }

    /// Hand any text a widget copied (a selection in a field, Ctrl+C) to
    /// the system clipboard.
    fn copy_emitted_text(&self, emitted: &[UiEvent], backend: &dyn Backend) {
        for event in emitted {
            if let UiEvent::TextCopied(text) = event {
                backend.services().clipboard().write_text(text);
            }
        }
    }

    /// Route a pointer event on the formula bar through its
    /// [`TextField`]'s own handling — click to place the caret, drag (or
    /// Shift+click) to select — and write the resulting caret, selection
    /// and text back onto the in-progress edit both the bar and the cell
    /// show.
    fn route_to_formula_field(&mut self, event: &UiEvent, backend: &mut dyn Backend) {
        let Some(edit) = self.grid.editing.clone() else {
            return;
        };
        let rect =
            Self::split_formula_band(Self::bands(backend).formula, Self::name_box_width(backend)).1;
        let local = |p: Point| Point::new(p.x - rect.x, p.y - rect.y);
        let routed = match event {
            UiEvent::MouseDown {
                button: MouseButton::Left,
                position,
                modifiers,
                ..
            } => RoutedEvent::Pointer {
                kind: PointerKind::Down(MouseButton::Left),
                sub: None,
                local: local(*position),
                mods: *modifiers,
            },
            UiEvent::MouseMoved { position, .. } => RoutedEvent::Pointer {
                kind: PointerKind::Move,
                sub: None,
                local: local(*position),
                mods: Modifiers::default(),
            },
            UiEvent::MouseUp {
                button: MouseButton::Left,
                position,
                ..
            } => RoutedEvent::Pointer {
                kind: PointerKind::Up(MouseButton::Left),
                sub: None,
                local: local(*position),
                mods: Modifiers::default(),
            },
            _ => return,
        };
        let field = formula_bar::build_field(&edit.text, &self.grid, true);
        let layout = backend.layout_primitive(rect, &field);
        self.formula_state.cursor = Some(edit.cursor);
        self.formula_state.anchor = edit.selection_anchor;
        let mut host = WidgetHost::new(Self::units(backend));
        let handled = {
            let mut cx = EventCx::new(field.id, &mut host);
            field.handle(&mut self.formula_state, &routed, &layout, &mut cx)
        };
        let text = match handled {
            Handled::Event(TextFieldAction::Changed(text)) => text,
            _ => edit.text.clone(),
        };
        let cursor = self
            .formula_state
            .cursor
            .unwrap_or(edit.cursor)
            .min(text.len());
        let selection_anchor = self
            .formula_state
            .anchor
            .filter(|&a| a != cursor && a <= text.len());
        self.grid.editing = Some(CellEditState {
            text,
            cursor,
            selection_anchor,
            placeholder: edit.placeholder,
        });
    }

    /// Scroll the grid by a wheel `delta` (positive `y` = toward the top),
    /// three rows per notch, carrying any fraction over to the next
    /// event. The cursor stays where it is.
    fn scroll_grid(&mut self, dx: f32, dy: f32) {
        const ROWS_PER_NOTCH: f32 = 3.0;
        let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
        // Positive `y` scrolls toward the top; positive `x` toward the
        // right (`quadraui::ScrollDelta`).
        let rows = self.wheel_carry.1 + quadraui::wheel_delta(finite(dy), ROWS_PER_NOTCH);
        let cols = self.wheel_carry.0 + finite(dx);
        let (whole_rows, whole_cols) = (rows.trunc(), cols.trunc());
        self.wheel_carry = (cols - whole_cols, rows - whole_rows);
        let max_row = self
            .grid
            .row_count
            .saturating_sub(self.grid.frozen_rows)
            .saturating_sub(1);
        let max_col = self
            .grid
            .col_count
            .saturating_sub(self.grid.frozen_cols)
            .saturating_sub(1);
        let step = |at: usize, by: f32, max: usize| -> usize {
            ((at as f32 + by).max(0.0) as usize).min(max)
        };
        self.grid.row_scroll = step(self.grid.row_scroll, whole_rows, max_row);
        self.grid.col_scroll = step(self.grid.col_scroll, whole_cols, max_col);
    }

    /// The grid's right-click menu: clipboard, row/column structure,
    /// clearing and formatting, and find — each item a catalog id
    /// [`Self::run_command_id`] runs exactly as the menu bar would.
    fn context_menu_items(&self) -> Vec<ContextMenuItem> {
        let item = |id: &str, label: &str, detail: Option<&str>| ContextMenuItem {
            id: Some(WidgetId::new(id)),
            label: StyledText::plain(label),
            detail: detail.map(StyledText::plain),
            ..Default::default()
        };
        let sep = ContextMenuItem::default;
        vec![
            item("edit.cut", "Cut", Some("Ctrl+X")),
            item("edit.copy", "Copy", Some("Ctrl+C")),
            item("edit.paste", "Paste", Some("Ctrl+V")),
            sep(),
            item("home.insertRows", "Insert Row", None),
            item("home.deleteRows", "Delete Row", None),
            item("home.insertColumns", "Insert Column", None),
            item("home.deleteColumns", "Delete Column", None),
            sep(),
            item("edit.clearContents", "Clear Contents", Some("Del")),
            item("home.bold", "Bold", None),
            item("home.numberFormat", "Number Format…", None),
            sep(),
            item("edit.find", "Find…", Some("Ctrl+F")),
        ]
    }

    /// Open the context menu with its top-left at `at`, nudged to stay on
    /// screen.
    fn open_context_menu(&mut self, at: Point, backend: &mut dyn Backend) {
        self.app_menu.dismiss(backend);
        let mut menu = ContextMenu {
            id: WidgetId::new("quadracalc-context-menu"),
            items: self.context_menu_items(),
            selected_idx: 0,
            bg: None,
            placement: Default::default(),
            glyphs: Default::default(),
        };
        menu.selected_idx = menu.first_selectable();
        let vp = backend.viewport();
        let ctx = Self::layout_ctx(backend);
        let size = Primitive::measure(&menu, &ctx, Size::new(vp.width, vp.height));
        let w = size.width.unwrap_or(vp.width / 3.0).min(vp.width);
        let h = size.height.unwrap_or(vp.height / 2.0).min(vp.height);
        // Whole cells on a terminal: the menu paints snapped to the cell
        // grid, so its hit-test rect must be snapped the same way.
        let at = if Self::is_tui_shaped(backend) {
            Point::new(at.x.floor(), at.y.floor())
        } else {
            at
        };
        let x = at.x.min(vp.width - w).max(0.0);
        let y = at.y.min(vp.height - h).max(0.0);
        self.context_menu = Some((menu, Rect::new(x, y, w, h)));
        self.context_menu_state = ContextMenuState::default();
    }

    /// Right-click: a click outside the current selection moves the
    /// cursor there first (one inside it keeps the selection, so the menu
    /// acts on the whole range), then the menu opens at the pointer.
    fn open_context_menu_at_pointer(&mut self, at: Point, backend: &mut dyn Backend) {
        let rect = self.grid_rect(backend);
        let layout = Self::grid_layout(&self.grid, backend, rect);
        if let CellGridHit::Cell(row, col) = layout.hit_test(at.x, at.y) {
            let (top, left, bottom, right) = self.grid.selection_bounds();
            if !(top..=bottom).contains(&row) || !(left..=right).contains(&col) {
                self.grid.cursor = (row, col);
                self.grid.selection_anchor = None;
            }
        }
        self.open_context_menu(at, backend);
    }

    /// Shift+F10 / the Menu key: the menu opens at the cursor cell.
    fn open_context_menu_at_cursor(&mut self, backend: &mut dyn Backend) {
        let rect = self.grid_rect(backend);
        let layout = Self::grid_layout(&self.grid, backend, rect).layout;
        let (row, col) = self.grid.cursor;
        let y = layout
            .rows
            .iter()
            .find(|s| s.index == row)
            .map_or(0.0, |s| s.start + s.size);
        let x = layout
            .cols
            .iter()
            .find(|s| s.index == col)
            .map_or(0.0, |s| s.start);
        self.open_context_menu(Point::new(rect.x + x, rect.y + y), backend);
    }

    /// Every event while the context menu is open: keys and pointer
    /// gestures go to the menu, a press outside it dismisses it.
    fn handle_context_menu_event(
        &mut self,
        event: &UiEvent,
        backend: &mut dyn Backend,
    ) -> Reaction {
        let Some((menu, rect)) = self.context_menu.clone() else {
            return Reaction::Continue;
        };
        let local = |p: Point| Point::new(p.x - rect.x, p.y - rect.y);
        let routed = match event {
            UiEvent::WindowResized { .. } => {
                self.context_menu = None;
                self.refresh(backend);
                return Reaction::Redraw;
            }
            UiEvent::MouseDown { position, .. } if !rect.contains(*position) => {
                self.context_menu = None;
                return Reaction::Redraw;
            }
            UiEvent::MouseDown {
                button: MouseButton::Left,
                position,
                modifiers,
                ..
            } => RoutedEvent::Pointer {
                kind: PointerKind::Down(MouseButton::Left),
                sub: None,
                local: local(*position),
                mods: *modifiers,
            },
            UiEvent::MouseMoved { position, .. } if rect.contains(*position) => {
                RoutedEvent::Pointer {
                    kind: PointerKind::Move,
                    sub: None,
                    local: local(*position),
                    mods: Modifiers::default(),
                }
            }
            UiEvent::KeyPressed {
                key,
                modifiers,
                repeat,
            } => RoutedEvent::Key(KeyEvent {
                key: key.clone(),
                modifiers: *modifiers,
                repeat: *repeat,
            }),
            _ => return Reaction::Continue,
        };
        let layout = backend.layout_primitive(rect, &menu);
        let mut host = WidgetHost::new(Self::units(backend));
        let handled = {
            let mut cx = EventCx::new(menu.id, &mut host);
            menu.handle(&mut self.context_menu_state, &routed, &layout, &mut cx)
        };
        match handled {
            Handled::Event(ContextMenuEvent::Activated { item }) => {
                self.context_menu = None;
                self.run_command_id(item.as_str(), backend)
            }
            Handled::Event(ContextMenuEvent::SelectionChanged { idx }) => {
                if let Some((menu, _)) = &mut self.context_menu {
                    menu.selected_idx = idx;
                }
                Reaction::Redraw
            }
            Handled::Event(ContextMenuEvent::Dismissed | ContextMenuEvent::SubmenuClosed) => {
                self.context_menu = None;
                Reaction::Redraw
            }
            Handled::Event(_) | Handled::Consumed => Reaction::Redraw,
            Handled::Ignored => Reaction::Continue,
        }
    }

    /// Run one command a menu item, a context-menu item or the palette
    /// names: the few ids this app handles itself (quit, find/replace,
    /// banded rows), a parameter picker for the ids that need one, and
    /// every other catalog id through the engine.
    fn run_command_id(&mut self, id_str: &str, backend: &mut dyn Backend) -> Reaction {
        if id_str == menu::QUIT {
            return Reaction::Exit;
        }
        if id_str == menu::BANDED_ROWS {
            self.striped = !self.striped;
            self.refresh(backend);
            return Reaction::Redraw;
        }
        if id_str == "edit.find" || id_str == "edit.replace" {
            self.open_find(id_str == "edit.replace", backend);
            self.refresh(backend);
            return Reaction::Redraw;
        }
        // Same "open a picker instead of running it" branch the
        // palette's own `PaletteOutcome::Pick` uses — a menu
        // click on Font/Font Size/Fill Color/Font Color/Borders/
        // Draw Border/Number Format needs the same parameter
        // dialog, just reached from the menu instead of the
        // palette. `catalog_id` recovers the `&'static str` the
        // catalog's own id table holds, since `WidgetId` only
        // ever gives back a borrowed `&str` and `FormatPicker::
        // open` wants the former.
        if format_picker::has_picker(id_str) {
            match Self::catalog_id(id_str) {
                Some(cid) => self.format_picker = FormatPicker::open(cid),
                // Unreachable today — `menu::tab_items` only ever
                // builds items from `CATALOG` itself, so every id
                // it hands back resolves here by construction.
                // Surfaced rather than silently dropped in case a
                // future catalog id rename ever desyncs the two.
                None => {
                    self.error = Some(format!("no catalog entry for \"{id_str}\""));
                }
            }
            self.refresh(backend);
            return Reaction::Redraw;
        }
        // Same "open a picker instead of running it" branch
        // the palette's own `PaletteOutcome::PickFunction`
        // uses — the seven Function Library category ids
        // are `cmd!(query ...)` wrappers whose result would
        // be discarded by the generic arm below, and
        // `formulas.insertFunction` only pushes a
        // `UiRequest::Dialog` nothing in this crate reads.
        // `function_picker::has_picker` and
        // `open_function_picker` already exist for exactly
        // this; see their own docs.
        if function_picker::has_picker(id_str) {
            match Self::catalog_id(id_str) {
                Some(cid) => match self.open_function_picker(cid) {
                    Ok(picker) => self.function_picker = Some(picker),
                    Err(e) => self.error = Some(e.to_string()),
                },
                // Unreachable today, for the same reason as
                // the `format_picker::has_picker` arm above.
                None => {
                    self.error = Some(format!("no catalog entry for \"{id_str}\""));
                }
            }
            self.refresh(backend);
            return Reaction::Redraw;
        }
        // `sync_selection` before running a catalog command
        // against no explicit range, same as the palette run
        // above — except for the ids this crate wraps in its
        // own helper instead of a bare `execute`: "Paste" needs
        // the system clipboard (see `menu::tab_items`'s and
        // `clipboard::paste_from_system`'s own docs), "Copy"/
        // "Cut" need `clipboard::copy`/`cut`'s mirror onto the
        // system clipboard (the same one `CalcApp::handle`'s
        // own Ctrl+C/X dispatch uses — a bare `execute` here
        // would silently stop populating the OS clipboard), and
        // "Save" needs `CalcApp::save`'s "nothing to save to
        // yet" handling (the same one Ctrl+S's dispatch uses).
        // Keeping all three in step with the keyboard path
        // matters most on macOS: a native `NSMenu` key
        // equivalent (see `menu::accelerator`'s own doc) bypasses
        // `handle`'s `KeyPressed` arms entirely and lands here
        // instead, so this arm is that shortcut's only path to
        // the engine on that platform.
        let result = if id_str == "edit.paste" {
            clipboard::paste_from_system(self, backend)
        } else if id_str == "edit.copy" {
            clipboard::copy(self, backend)
        } else if id_str == "edit.cut" {
            clipboard::cut(self, backend)
        } else if id_str == "file.save" {
            self.save()
        } else {
            self.sync_selection()
                .and_then(|()| self.execute(id_str, json!({})))
                .map(|_| ())
        };
        if let Err(e) = result {
            self.error = Some(e.to_string());
        }
        self.refresh(backend);
        Reaction::Redraw
    }

    /// Apply `op` to the in-progress cell edit's text buffer — used for
    /// `UiEvent::ClipboardPaste` while editing, the one text-buffer
    /// mutation that reaches this app directly rather than through
    /// `CellGrid::handle`/`Self::dispatch_to_grid` (quadraui's runtime
    /// resolves a native paste keypress to `UiEvent::ClipboardPaste`
    /// before `handle` ever sees it as a `KeyPressed`, and `RoutedEvent`
    /// has no counterpart for it — see `handle`'s own `ClipboardPaste`
    /// arm). Bridges through a scratch [`TextField`] and its own
    /// `apply_edit_op`, the same flat byte-offset editor `CellGrid`'s
    /// internal `apply_edit` uses, since `CellGrid` exposes no public
    /// edit-buffer-mutation entry point of its own.
    fn apply_edit_op(&mut self, op: EditOp) {
        let Some(edit) = self.grid.editing.take() else {
            return;
        };
        let mut field = TextField::new(WidgetId::new("quadracalc-grid:editor"))
            .with_value(edit.text)
            .with_cursor(Some(edit.cursor))
            .with_selection_anchor(edit.selection_anchor);
        field.apply_edit_op(op);
        self.grid.editing = Some(CellEditState {
            text: field.value,
            cursor: field.cursor.unwrap_or(0),
            selection_anchor: field.selection_anchor,
            placeholder: edit.placeholder,
        });
    }

    /// Resolves a click already known to land inside `rect` (`handle`'s
    /// own `self.sidebar_rect(backend)` check) against the same
    /// `SidebarPanel`/`ListView` layouts `render` paints this panel
    /// with, and applies it: the toolbar's close button closes the
    /// sidebar, a content row selects it. Anything else (the toolbar's
    /// title/count labels, the gap past the last row) is a no-op.
    fn handle_sidebar_click(&mut self, position: Point, rect: Rect, backend: &dyn Backend) {
        let panel = sidebar::toolbar_panel(self.view.sidebar_names.len());
        let layout = backend.layout_primitive(rect, &panel);
        match layout.hit_test(position.x, position.y) {
            SidebarPanelHit::ToolbarButton(id) if id == sidebar::close_button_id() => {
                self.sidebar.close();
            }
            SidebarPanelHit::Content { x, y } => {
                let list = sidebar::list_view(&self.view.sidebar_names, self.sidebar.selected());
                // `layout_primitive`, not a bare `line_height`-only
                // `ListView::layout` call: `line_height` is the *editor*
                // font metric, while the list paints rows at the
                // backend's own resolved row pitch (a whole cell on TUI;
                // `Style::row_height` floored at the chrome font on
                // GTK/macOS/Win) and reserves a scrollbar column once it
                // overflows — the same ctx the `Surface::Widget` `render`
                // pushes for it is painted with.
                let area = Rect::new(
                    0.0,
                    0.0,
                    layout.content_bounds.width,
                    layout.content_bounds.height,
                );
                let list_layout = backend.layout_primitive(area, &list).layout;
                if let ListViewHit::Item(idx) = list_layout.hit_test(x, y) {
                    self.sidebar.select(idx, self.view.sidebar_names.len());
                }
            }
            SidebarPanelHit::ToolbarButton(_)
            | SidebarPanelHit::ToolbarEmpty
            | SidebarPanelHit::Empty => {}
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

    /// #20: `insert.checkbox` already changed what the cell displays
    /// (`TRUE`/`FALSE`, same as any boolean cell) before this glyph
    /// swap existed — this asserts the swap itself, not that the
    /// command is visible at all.
    #[test]
    fn a_checkbox_formatted_cell_shows_a_glyph_not_true_false() {
        let mut app = CalcApp::new();
        app.execute("insert.checkbox", json!({"range": "A1"}))
            .unwrap();
        assert_eq!(app.cell_text("A1"), "☐");
        app.execute("cell.toggleCheckbox", json!({"cell": "A1"}))
            .unwrap();
        assert_eq!(app.cell_text("A1"), "☑");
    }

    /// A plain boolean cell — no checkbox formatting — keeps showing
    /// the literal `TRUE`/`FALSE` text, confirming the glyph swap is
    /// gated on the cell's own style, not just its boolean value.
    #[test]
    fn a_plain_boolean_cell_is_unaffected() {
        let mut app = CalcApp::new();
        app.set_cell("A1", "TRUE").unwrap();
        assert_eq!(app.cell_text("A1"), "TRUE");
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
        assert_eq!(app.grid.selection_anchor, None);
    }

    #[test]
    fn selecting_numeric_cells_shows_average_count_and_sum() {
        let mut app = CalcApp::with_sample().unwrap();
        let mut b = backend();
        app.setup(&mut b);
        // B2:B3 — the sample's two numeric cells, 1200.50 and 9.
        app.handle(key(NamedKey::Down, Modifiers::default()), &mut b);
        app.handle(key(NamedKey::Right, Modifiers::default()), &mut b);
        app.handle(key(NamedKey::Down, shift()), &mut b);
        assert_eq!(app.view().selection_label, "B2:B3");
        assert_eq!(
            app.view().selection_stats,
            "Average: 604.75   Count: 2   Sum: 1209.50"
        );
    }

    #[test]
    fn selecting_text_only_cells_falls_back_to_just_a_count() {
        let mut app = CalcApp::with_sample().unwrap();
        let mut b = backend();
        app.setup(&mut b);
        // A2:A3 — the sample's two text cells, "Hosting" and "Domains".
        app.handle(key(NamedKey::Down, Modifiers::default()), &mut b);
        app.handle(key(NamedKey::Down, shift()), &mut b);
        assert_eq!(app.view().selection_label, "A2:A3");
        assert_eq!(app.view().selection_stats, "Count: 2");
    }

    #[test]
    fn selecting_an_empty_cell_shows_no_stats_at_all() {
        let mut app = CalcApp::new();
        let mut b = backend();
        app.setup(&mut b);
        assert_eq!(app.view().selection_label, "A1");
        assert_eq!(app.view().selection_stats, "");
    }

    #[test]
    fn sample_header_row_renders_bold_and_filled_but_the_row_below_does_not() {
        let mut app = CalcApp::with_sample().unwrap();
        let mut b = backend();
        app.setup(&mut b);
        let grid = app.view().grid.as_ref().unwrap();

        // (0, 0) and (0, 1) are A1/B1 — `home.bold`/`home.fillColor`'s
        // target range in `with_sample` — with no scroll yet.
        let a1 = &grid.cells[0][0];
        let b1 = &grid.cells[0][1];
        assert_eq!(a1.text, "Item");
        assert_eq!(a1.style.bold, Some(true));
        assert!(a1.style.bg.is_some(), "A1 should carry the header fill");
        assert_eq!(
            b1.style.bold,
            Some(true),
            "fillColor/bold's range is A1:B1, not just A1"
        );
        assert_eq!(b1.style.bg, a1.style.bg);

        // A2, one row down, was never formatted and stays plain.
        let a2 = &grid.cells[1][0];
        assert_eq!(a2.text, "Hosting");
        assert_eq!(a2.style.bold, Some(false));
        assert_eq!(a2.style.bg, None);
    }

    #[test]
    fn window_resized_refreshes_the_loaded_window() {
        let mut app = CalcApp::new();
        let mut b = backend();
        app.setup(&mut b);
        let rows_before = app.view().grid.as_ref().unwrap().cells.len();

        // Grow the viewport well past the original 20 rows and re-dispatch
        // through `handle`, the way a real resize reaches the app —
        // without an explicit `WindowResized` arm, the `RoutedEvent`
        // conversion ahead of `CellGrid::handle` has no arm for it either
        // and `refresh` never runs, so the cached `View` stays sized for
        // the old, smaller viewport.
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

    #[test]
    fn alt_letter_is_swallowed_instead_of_typed_into_the_active_cell() {
        let mut app = CalcApp::with_sample().unwrap();
        let mut b = backend();
        app.setup(&mut b);
        let before = app.cell_text("A1");

        let alt_a = UiEvent::KeyPressed {
            key: Key::Char('a'),
            modifiers: Modifiers {
                alt: true,
                ..Default::default()
            },
            repeat: false,
        };
        let reaction = app.handle(alt_a, &mut b);

        assert_eq!(
            reaction,
            Reaction::Continue,
            "Alt+letter should be swallowed, not start an edit"
        );
        assert!(
            app.grid.editing.is_none(),
            "Alt+letter must not start an in-cell edit"
        );
        assert_eq!(
            app.cell_text("A1"),
            before,
            "Alt+letter must not clobber the active cell's content"
        );
    }

    #[test]
    fn option_composed_character_still_reaches_the_active_cell() {
        // macOS Option-composition: Option+A arrives as `Key::Char('å')`
        // with `alt: true`, since Cocoa has no Alt/Option menu mnemonics
        // at all. 'å' is not ASCII alphanumeric, so it must not be
        // swallowed by the mnemonic guard.
        let mut app = CalcApp::with_sample().unwrap();
        let mut b = backend();
        app.setup(&mut b);

        let option_a = UiEvent::KeyPressed {
            key: Key::Char('å'),
            modifiers: Modifiers {
                alt: true,
                ..Default::default()
            },
            repeat: false,
        };
        let reaction = app.handle(option_a, &mut b);

        assert_eq!(
            reaction,
            Reaction::Redraw,
            "an Option-composed character must start an edit, not be swallowed"
        );
        assert!(
            app.grid.editing.is_some(),
            "Option+A must start editing the active cell on macOS"
        );
    }

    // No end-to-end test for the AltGr (Ctrl+Alt) carve-out: `Key::Char`
    // with `ctrl: true` already falls to `CellGrid::handle_key_idle`'s own
    // `jump = modifiers.ctrl || modifiers.cmd` guard, which treats *any*
    // `ctrl` as a navigation modifier and skips `begin_typing` regardless
    // of this app's own mnemonic guard — so an end-to-end `app.handle`
    // test can't yet observe the `!modifiers.ctrl` condition doing
    // anything, and would pass or fail for a reason unrelated to this
    // file. That conflation lives in quadraui's `primitives/cell_grid.rs`,
    // out of scope here. `!modifiers.ctrl` is kept anyway, both because
    // it's the narrow, correct condition and so a future quadraui fix
    // doesn't silently regress through this guard.
}
