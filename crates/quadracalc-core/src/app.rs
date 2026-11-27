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
use quadraui::primitive::{LayoutCtx, Units};
use quadraui::primitives::cell_grid::CellEditState;
use quadraui::{
    AppLogic, AppMenu, AppMenuEvent, AppMenuState, Backend, ButtonMask, Cell, CellGrid,
    CellGridEvent, CellGridPrimitiveLayout, CellGridState, CellStyle, Chart, Color,
    DrawsPrimitives, EditOp, EventCx, EventHost, Handled, Interactive, Key, KeyEvent, ListViewHit,
    Modifiers, MouseButton, NamedKey, Point, PointerKind, PointerShape, Reaction, Rect,
    RoutedEvent, ScreenLayout, SidebarPanelHit, StatusBar, StatusBarSegment, Surface, TabBar,
    TabBarHit, TextField, UiEvent, WidgetId, resolve_bar_height,
};
use serde_json::{Value, json};

use crate::address::Address;
use crate::agent;
use crate::chart_view;
use crate::clipboard;
use crate::format_picker::{self, FormatPicker, PickerOutcome};
use crate::formula_bar::{self, Bands};
use crate::function_picker::{self, FunctionPicker, Outcome as FunctionPickerOutcome};
use crate::menu;
use crate::palette::{CommandPalette, Outcome as PaletteOutcome};
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
    /// The active sheet's charts (`crate::chart_view::charts`), resolved
    /// fresh on every `refresh` — empty when it has none, which is also
    /// what tells [`CalcApp::grid_rect`] and [`View::screen`] not to
    /// carve out a chart panel at all. See `crate::chart_view`'s module
    /// doc for why this is a quadraui [`Chart`] already resolved to
    /// plain numbers, not the engine's own `Chart` model.
    pub charts: Vec<Chart>,
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
        }
    }

    /// A workbook with a small sample, so a first launch shows a live
    /// formula — and enough range (headers, a lookup table, a formula
    /// chain) to be worth a screenshot of the ribbon-style surface this
    /// app adds over the engine's bare grid: a bold+filled header row, an
    /// explicit currency number format (not just the `$`-prefix
    /// autodetect `B2`'s literal input already gets), and a `SUM` feeding
    /// a `VLOOKUP` against a small rate table. `A1:B4` keeps its original
    /// cells and formula (`editing.rs`, `open_save.rs` and others assert
    /// on `=SUM(B2:B3)` and its typed inputs by name); everything new
    /// lives in the previously-empty columns/rows around it.
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
        // Bold, filled header row — the Home-tab commands the format
        // picker now supplies a parameter for (see `format_picker.rs`).
        app.execute("home.bold", json!({ "range": "A1:B1", "on": true }))?;
        app.execute(
            "home.fillColor",
            json!({ "range": "A1:B1", "color": "#D9E8F5" }),
        )?;
        // An explicit currency number format on the cost column, rather
        // than relying only on `B2`'s typed `$` prefix.
        app.execute(
            "home.numberFormat",
            json!({ "range": "B2:B4", "format": "Currency" }),
        )?;
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
    /// plus the `Style` fields [`quadraui::CellGrid`] itself can paint —
    /// bold and an explicit solid fill colour, read off the same
    /// `cell.get` call `cell_text` would otherwise make a second time,
    /// so a visible cell's style costs nothing extra over its text.
    ///
    /// Only [`gridcraft_engine::model::Color::Rgb`] (what
    /// `crate::format_picker`'s colour choices and a plain hex literal
    /// in a loaded file both produce) resolves to a paintable colour;
    /// `Auto` and a theme-tinted `Theme(..)` both come back `None`
    /// (inherit the grid's own default) — resolving a theme tint needs
    /// the workbook's theme palette, which has no `Session::execute`
    /// query to read it back through, only ribbon commands that set it.
    fn grid_cell(&mut self, a1: &str) -> Cell {
        let Ok(cell) = self.session.execute("cell.get", json!({ "cell": a1 })) else {
            return Cell::plain(String::new());
        };
        let text = cell.get("text").and_then(Value::as_str).unwrap_or_default();
        let text = Self::checkbox_glyph(&cell, text).unwrap_or_else(|| text.to_string());

        let style = cell.get("style");
        let bold = style
            .and_then(|s| s.get("font"))
            .and_then(|f| f.get("bold"))
            .and_then(Value::as_bool);
        let bg = style
            .and_then(|s| s.get("fill"))
            .filter(|fill| fill.get("pattern").and_then(Value::as_str) == Some("Solid"))
            .and_then(|fill| fill.get("fg"))
            .and_then(|c| c.get("Rgb"))
            .and_then(Value::as_u64)
            .map(|rgb| Color::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8));

        Cell {
            text,
            style: CellStyle {
                bold,
                bg,
                ..CellStyle::default()
            },
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
        let units = Self::units(backend);
        let routed = RoutedEvent::Key(KeyEvent {
            key,
            modifiers: Modifiers::default(),
            repeat: false,
        });
        self.dispatch_to_grid(routed, &layout, units)
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
        let rect = self.grid_rect(backend);
        let layout = Self::grid_layout(&self.grid, backend, rect).layout;
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

        // Re-classify against the engine's current state — e.g. "Undo"
        // greys out the instant there's nothing left to undo. Cheap
        // enough to rebuild every refresh (same cost the palette already
        // pays every time it opens) rather than tracking what changed.
        self.app_menu.set_menus(menu::menus(&self.session));

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
struct GridHost {
    units: Units,
    emitted: Vec<UiEvent>,
}

impl GridHost {
    fn new(units: Units) -> Self {
        Self {
            units,
            emitted: Vec::new(),
        }
    }
}

impl EventHost for GridHost {
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
    fn screen(&self, bands: Bands) -> ScreenLayout {
        let mut layout = ScreenLayout::new();
        layout.push(Surface::Widget {
            rect: bands.formula,
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
            right_segments: {
                // Least-important first, most-important (highest
                // priority to stay visible on a narrow bar) last — see
                // `StatusBarSegment`'s own doc on `right_segments`
                // ordering. The selection summary is the one Excel users
                // actually look for here, so it goes last/rightmost;
                // the branding stays but yields first on a narrow bar.
                let mut right = vec![StatusBarSegment {
                    text: " quadracalc · GridCraft engine · q quits ".into(),
                    fg: Color::rgb(200, 210, 225),
                    bg: bar_bg,
                    bold: false,
                    action_id: None,
                }];
                if !self.selection_stats.is_empty() {
                    right.push(StatusBarSegment {
                        text: format!(" {} ", self.selection_stats),
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
        let mut layout = self.view.screen(bands);
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
                    // Same "Paste" special case the menu's own
                    // `Activated` dispatch uses — see
                    // `clipboard::paste_from_system`'s own doc for why
                    // this id can't go through a generic `execute`.
                    let result = if id == "edit.paste" {
                        clipboard::paste_from_system(self, backend)
                    } else {
                        // The palette was classified and is now run
                        // against the UI's own selection, not
                        // necessarily the one the engine still has
                        // cached from the last `cell.set`/`edit.undo` —
                        // see `sync_selection`'s own doc.
                        self.sync_selection()
                            .and_then(|()| self.execute(id, json!({})))
                            .map(|_| ())
                    };
                    if let Err(e) = result {
                        self.error = Some(e.to_string());
                    }
                    self.refresh(backend);
                    Reaction::Redraw
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
                Handled::Event(AppMenuEvent::Activated(id)) if id.as_str() == menu::QUIT => {
                    return Reaction::Exit;
                }
                Handled::Event(AppMenuEvent::Activated(id)) => {
                    let id_str = id.as_str();
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
                    return Reaction::Redraw;
                }
                Handled::Event(_) | Handled::Consumed => {
                    self.refresh(backend);
                    return Reaction::Redraw;
                }
                Handled::Ignored => {}
            }
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
            && Self::bands(backend).formula.contains(*position)
        {
            if self.grid.editing.is_none() {
                let a1 = self.cursor().a1();
                let text = self.cell_input(&a1);
                self.start_editing(text);
            }
            self.formula_focused = true;
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
        let units = Self::units(backend);
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
                let reaction = self.dispatch_to_grid(routed, &layout, units);
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
        LayoutCtx::new(
            LayoutCtx::resolve_row_height(backend.chrome_line_height(), &backend.style()),
            backend.char_width(),
            backend.nerd_fonts_enabled(),
        )
        .units()
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
    /// reports (plus every extra event [`GridHost::emit`] collected — see
    /// that type's own doc for why nothing else it carries matters here).
    /// Mirrors quadraui's own
    /// `examples/common/cell_grid_demo.rs::CellGridDemo::dispatch_to_grid`.
    fn dispatch_to_grid(
        &mut self,
        routed: RoutedEvent,
        layout: &CellGridPrimitiveLayout,
        units: Units,
    ) -> Reaction {
        let mut host = GridHost::new(units);
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
                Reaction::Redraw
            }
            // Escape cancelled the edit — whether it was typed through
            // the grid or the formula bar, neither owns the (now gone)
            // caret any more.
            CellGridEvent::CellEditCancelled { .. } => {
                self.grid.editing = None;
                self.formula_focused = false;
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
