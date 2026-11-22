//! Command palette over GridCraft's own command catalog
//! (`gridcraft_engine::catalog::CATALOG`): id, ribbon tab, group and
//! label for every placement GridCraft's own UI would show. Opened with
//! Ctrl+Shift+P (or `:` on a TUI-shaped backend — see `app.rs`), it
//! fuzzy-filters by label and tab and runs the selected row through
//! `Session::execute` against the current selection.
//!
//! This builds and drives quadraui's own [`Palette`] primitive; it does
//! not paint anything itself (`CommandPalette::palette` just returns the
//! primitive's declarative struct for `app.rs` to push through
//! `ScreenLayout`, same as every other surface).
//!
//! A catalog entry whose command needs a parameter this app can't supply
//! yet (a cell range picker aside — every optional `range` already
//! defaults to the current selection, see `gridcraft_engine::cmd::target_ranges`)
//! is shown disabled with a short reason, same for an id the catalog
//! lists but no command implements (`catalog.rs`'s own doc: "a catalog
//! entry counts as done when a command with its id is registered").
//! Two exceptions, both shown enabled and routed to a dialog host
//! instead of a plain `Outcome::Run`: an id `crate::format_picker::has_picker`
//! knows (`Outcome::Pick`) opens that module's choice dialog to collect
//! the parameter, and an id `crate::function_picker::has_picker` knows
//! (`Outcome::PickFunction`) opens the Formulas tab's searchable
//! function list instead of running `formulas.insertFunction`/a category
//! id generically, which would otherwise just discard whatever it
//! returned.

use gridcraft_engine::{Session, catalog::CATALOG, find_command};
use quadraui::{
    Color, Key, Modifiers, NamedKey, Palette, PaletteItem, PaletteMode, StyledText, WidgetId,
    fuzzy_score, next_char_boundary, prev_char_boundary,
};

/// Colour for a disabled row's label and reason — muted, distinct from
/// the normal row's default foreground.
fn muted() -> Color {
    Color::rgb(130, 130, 130)
}

/// One catalog placement (an id can appear under more than one ribbon
/// tab, e.g. `data.sort` under both Home and Data — each placement is
/// its own row, matching GridCraft's own ribbon), plus whether this app
/// can run it right now.
#[derive(Debug, Clone)]
pub struct Entry {
    pub id: &'static str,
    pub tab: &'static str,
    pub group: &'static str,
    pub label: &'static str,
    pub enabled: bool,
    /// Why `enabled` is `false`. Always `Some` when `enabled` is `false`,
    /// always `None` when it's `true`.
    pub disabled_reason: Option<String>,
    /// Whether running this entry opens a [`crate::format_picker::FormatPicker`]
    /// (`Outcome::Pick`) instead of running the command directly
    /// (`Outcome::Run`). Only ever `true` when `enabled` is too — see
    /// `classify`.
    pub picker: bool,
}

/// One row per `CATALOG` entry, each classified against `session`: no
/// registered command, a precondition the engine itself enforces (no
/// workbook open, nothing to undo, …), or a required parameter this app
/// has no way to supply yet all disable the row with a reason; anything
/// else can run as-is.
pub fn entries(session: &Session) -> Vec<Entry> {
    CATALOG
        .iter()
        .map(|&(id, tab, group, label)| {
            let (enabled, disabled_reason) = classify(session, id);
            Entry {
                id,
                tab,
                group,
                label,
                enabled,
                disabled_reason,
                picker: enabled && crate::format_picker::has_picker(id),
            }
        })
        .collect()
}

fn classify(session: &Session, id: &str) -> (bool, Option<String>) {
    let Some(spec) = find_command(id) else {
        return (false, Some("not implemented by this engine".to_string()));
    };
    let info = spec.info(session);
    if !info.enabled {
        return (false, info.disabled_reason);
    }
    // `crate::format_picker` can supply this command's own required
    // parameter through its own dialog host, so a required field here
    // doesn't disable the row the way `unsuppliable_params` below would.
    if crate::format_picker::has_picker(id) {
        return (true, None);
    }
    // The opposite override: a row `unsuppliable_params` would leave
    // enabled (every param really is optional) but that this app still
    // has nothing to show for — see `no_terminal_surface`'s own doc.
    if let Some(reason) = no_terminal_surface(id) {
        return (false, Some(reason.to_string()));
    }
    if let Some(fields) = unsuppliable_params(spec.params) {
        return (false, Some(format!("needs {fields}")));
    }
    (true, None)
}

/// Why `id` is disabled when it creates engine-side state this app's
/// `CellGrid`-only terminal view has no surface for at all — `None` for
/// every other id. Unlike `insert.table`/`insert.pivotTable`/
/// `insert.checkbox` (plain cell content and formatting `CellGrid`
/// already paints) or `insert.chart`/`insert.pivotChart` (wired to a
/// `Surface::Chart` panel — `crate::chart_view`), each of these creates
/// a *floating* object anchored at a point rather than inside a cell
/// (a `Shape`/`Slicer`) or print-only state no view here ever renders
/// (`pageLayout.headerFooter`), and quadraui has no floating-object
/// widget to paint one with (CLAUDE.md: "If quadracalc needs a widget
/// quadraui lacks, say so" — belongs upstream, not built here).
/// Disabling honestly beats leaving a menu item that runs and changes
/// nothing the user can see (#20).
fn no_terminal_surface(id: &str) -> Option<&'static str> {
    match id {
        "insert.textBox" | "insert.wordArt" | "insert.slicer" => {
            Some("no terminal surface yet: creates a floating object this view doesn't paint")
        }
        "pageLayout.headerFooter" => {
            Some("no terminal surface yet: print-only, never shown in the grid view")
        }
        _ => None,
    }
}

/// `None` when `params` (a `CommandSpec::params` doc string, e.g.
/// `"{range?, on?: bool}"`) describes no parameter this app can't
/// already fill in by itself — every field is optional, since an
/// omitted `range`/`cell` already falls back to the current selection
/// (`gridcraft_engine::cmd::target_ranges`). Otherwise, the
/// comma-separated names of the required top-level fields.
///
/// `params` is free-form documentation, not a real schema, so this is a
/// heuristic: it only looks at the first balanced `{…}` group and splits
/// on top-level commas. Good enough to tell "nothing required" apart
/// from "something required" for every command in `CATALOG` today;
/// anything it can't parse (no brace group at all) is treated as
/// "nothing required" rather than disabling the row, since a doc string
/// this heuristic can't read is far more likely to be a `{}`-free command
/// than a required field it failed to spot.
fn unsuppliable_params(params: &str) -> Option<String> {
    let open = params.find('{')?;
    let close = matching_brace(params, open)?;
    let inner = params[open + 1..close].trim();
    if inner.is_empty() {
        return None;
    }
    let required: Vec<&str> = split_top_level(inner)
        .into_iter()
        .filter_map(|field| {
            let field = field.trim();
            if field.is_empty() {
                return None;
            }
            let name_end = field
                .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(field.len());
            let optional = field[name_end..].starts_with('?');
            if optional {
                None
            } else if name_end == 0 {
                Some(field)
            } else {
                Some(&field[..name_end])
            }
        })
        .collect();
    if required.is_empty() {
        None
    } else {
        Some(required.join(", "))
    }
}

/// Index of the `}` (or `]`/`)`, bracket type is not enforced — good
/// enough for well-formed doc strings) that balances the opening
/// delimiter at `open`, skipping over nested brackets and anything
/// inside a `"…"` run.
fn matching_brace(s: &str, open: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_quote = false;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'"' => in_quote = !in_quote,
            b'{' | b'[' | b'(' if !in_quote => depth += 1,
            b'}' | b']' | b')' if !in_quote => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Splits `s` on commas that sit at bracket depth 0 and outside any
/// `"…"` run — so `"range?: \"A1:B2\" | by?: rows|cols"` and
/// `"columns?: [\"A\",\"C\"]"` each stay one field instead of splitting
/// on the commas their own value lists carry.
fn split_top_level(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut in_quote = false;
    let mut start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'"' => in_quote = !in_quote,
            b'{' | b'[' | b'(' if !in_quote => depth += 1,
            b'}' | b']' | b')' if !in_quote => depth -= 1,
            b',' if !in_quote && depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// What a key handled by [`CommandPalette::handle_key`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Query, cursor or selection changed — redraw, palette stays open.
    Changed,
    /// The highlighted, enabled row was confirmed: run this id (against
    /// the current selection, with no extra params) and close the
    /// palette.
    Run(&'static str),
    /// The highlighted row needs a parameter `crate::format_picker`
    /// supplies: close the palette and open a `FormatPicker` for this
    /// id instead of running it directly.
    Pick(&'static str),
    /// The highlighted row is one `crate::function_picker::has_picker`
    /// claims (`formulas.insertFunction` or a Function Library category):
    /// close the palette and open a `FunctionPicker` instead of running
    /// the id directly — running it generically would just discard
    /// whatever `formulas.functions`/`formulas.insertFunction` returned.
    PickFunction(&'static str),
    /// Escape: close the palette with no further action.
    Closed,
    /// The key meant nothing to the palette (including Enter on a
    /// disabled row, or on an empty filtered list).
    Ignored,
}

/// Open, query-filtered state for the command palette overlay.
/// `app.rs` holds one of these in `Some` while the palette is open, gets
/// it from [`CommandPalette::open`], feeds every key to
/// [`CommandPalette::handle_key`], and renders it with
/// [`CommandPalette::palette`].
pub struct CommandPalette {
    entries: Vec<Entry>,
    query: String,
    /// Byte offset of the query's edit caret.
    cursor: usize,
    /// `(index into entries, match byte positions in that entry's label)`,
    /// in descending score order — the palette's displayed rows.
    filtered: Vec<(usize, Vec<usize>)>,
    selected: usize,
}

impl CommandPalette {
    /// Snapshot `session`'s catalog classification and show every row
    /// (empty query = no filtering), selecting the first one.
    pub fn open(session: &Session) -> Self {
        let mut palette = Self {
            entries: entries(session),
            query: String::new(),
            cursor: 0,
            filtered: Vec::new(),
            selected: 0,
        };
        palette.refilter();
        palette
    }

    /// The current query text (for tests and for the odd caller that
    /// wants to show it outside the rendered `Palette`).
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The currently visible rows, in display order — what
    /// [`Self::palette`] renders and [`Self::handle_key`]'s Enter/arrows
    /// act on.
    pub fn filtered(&self) -> Vec<&Entry> {
        self.filtered
            .iter()
            .map(|&(idx, _)| &self.entries[idx])
            .collect()
    }

    fn refilter(&mut self) {
        let query = self.query.to_lowercase();
        let mut scored: Vec<(i32, usize, Vec<usize>)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(idx, entry)| {
                let haystack =
                    format!("{} {} {}", entry.label, entry.tab, entry.group).to_lowercase();
                let (score, _) = fuzzy_score(&haystack, &query)?;
                let positions = fuzzy_score(&entry.label.to_lowercase(), &query)
                    .map(|(_, positions)| positions)
                    .unwrap_or_default();
                Some((score, idx, positions))
            })
            .collect();
        // Higher score first; ties keep the catalog's own order (a
        // stable sort key on the original index) rather than whatever
        // order `filter_map` happened to produce.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        self.filtered = scored.into_iter().map(|(_, idx, pos)| (idx, pos)).collect();
        self.selected = 0;
    }

    /// Feed one key event to the palette. The caller (`app.rs`) routes
    /// every `KeyPressed` here while the palette is open and ignores
    /// everything else, matching the primitive's own "intercept all
    /// input while modal" contract.
    pub fn handle_key(&mut self, key: &Key, modifiers: &Modifiers) -> Outcome {
        match key {
            Key::Named(NamedKey::Escape) => Outcome::Closed,
            Key::Named(NamedKey::Enter) => {
                let Some(&(idx, _)) = self.filtered.get(self.selected) else {
                    return Outcome::Ignored;
                };
                let entry = &self.entries[idx];
                if !entry.enabled {
                    Outcome::Ignored
                } else if entry.picker {
                    Outcome::Pick(entry.id)
                } else if crate::function_picker::has_picker(entry.id) {
                    Outcome::PickFunction(entry.id)
                } else {
                    Outcome::Run(entry.id)
                }
            }
            Key::Named(NamedKey::Up) => {
                if self.selected > 0 {
                    self.selected -= 1;
                    Outcome::Changed
                } else {
                    Outcome::Ignored
                }
            }
            Key::Named(NamedKey::Down) => {
                if self.selected + 1 < self.filtered.len() {
                    self.selected += 1;
                    Outcome::Changed
                } else {
                    Outcome::Ignored
                }
            }
            Key::Named(NamedKey::Backspace) => {
                if self.cursor == 0 {
                    return Outcome::Ignored;
                }
                let prev = prev_char_boundary(&self.query, self.cursor);
                self.query.replace_range(prev..self.cursor, "");
                self.cursor = prev;
                self.refilter();
                Outcome::Changed
            }
            Key::Named(NamedKey::Delete) => {
                if self.cursor >= self.query.len() {
                    return Outcome::Ignored;
                }
                let next = next_char_boundary(&self.query, self.cursor);
                self.query.replace_range(self.cursor..next, "");
                self.refilter();
                Outcome::Changed
            }
            Key::Named(NamedKey::Left) => {
                if self.cursor == 0 {
                    return Outcome::Ignored;
                }
                self.cursor = prev_char_boundary(&self.query, self.cursor);
                Outcome::Changed
            }
            Key::Named(NamedKey::Right) => {
                if self.cursor >= self.query.len() {
                    return Outcome::Ignored;
                }
                self.cursor = next_char_boundary(&self.query, self.cursor);
                Outcome::Changed
            }
            Key::Named(NamedKey::Home) => {
                if self.cursor == 0 {
                    return Outcome::Ignored;
                }
                self.cursor = 0;
                Outcome::Changed
            }
            Key::Named(NamedKey::End) => {
                if self.cursor >= self.query.len() {
                    return Outcome::Ignored;
                }
                self.cursor = self.query.len();
                Outcome::Changed
            }
            // Only a plain (or shifted, for uppercase) character edits
            // the query — a Ctrl/Cmd chord reaching here is some other
            // shortcut the palette doesn't understand, not a character
            // to insert.
            Key::Char(c) if !modifiers.ctrl && !modifiers.cmd => {
                self.query.insert(self.cursor, *c);
                self.cursor += c.len_utf8();
                self.refilter();
                Outcome::Changed
            }
            _ => Outcome::Ignored,
        }
    }

    /// Build this frame's [`Palette`] primitive for `app.rs` to push
    /// through `ScreenLayout`. A disabled row's label and detail (its
    /// reason, in place of the usual tab/group breadcrumb) paint muted.
    pub fn palette(&self) -> Palette {
        let items = self
            .filtered
            .iter()
            .map(|(idx, positions)| {
                let entry = &self.entries[*idx];
                let (text, detail) = if entry.enabled {
                    (
                        StyledText::plain(entry.label),
                        StyledText::plain(format!("{} › {}", entry.tab, entry.group)),
                    )
                } else {
                    (
                        StyledText::colored(entry.label, muted()),
                        StyledText::colored(
                            entry.disabled_reason.clone().unwrap_or_default(),
                            muted(),
                        ),
                    )
                };
                PaletteItem {
                    text,
                    detail: Some(detail),
                    icon: None,
                    match_positions: positions.clone(),
                    depth: 0,
                    expandable: false,
                    expanded: false,
                }
            })
            .collect();
        Palette {
            id: WidgetId::new("quadracalc-command-palette"),
            title: "Commands".to_string(),
            query: self.query.clone(),
            query_cursor: self.cursor,
            items,
            selected_idx: self.selected,
            scroll_offset: 0,
            total_count: self.entries.len(),
            has_focus: true,
            show_query: true,
            create_label: None,
            preview: None,
            mode: PaletteMode::List,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let mut s = Session::new();
        s.new_workbook();
        s
    }

    #[test]
    fn unsuppliable_params_accepts_every_field_optional() {
        assert_eq!(unsuppliable_params("{range?, on?: bool}"), None);
        assert_eq!(unsuppliable_params("{}"), None);
        assert_eq!(unsuppliable_params("{range?}"), None);
    }

    #[test]
    fn unsuppliable_params_names_a_required_field() {
        assert_eq!(
            unsuppliable_params("{path} | {name, base64}"),
            Some("path".to_string())
        );
        assert_eq!(
            unsuppliable_params("{id, latex}"),
            Some("id, latex".to_string())
        );
    }

    #[test]
    fn unsuppliable_params_does_not_split_inside_nested_brackets() {
        // The comma inside `[...]` and inside the quoted alternation must
        // not be mistaken for a field separator.
        assert_eq!(
            unsuppliable_params("{range?, columns?: [\"A\",\"C\"], header?: bool}"),
            None
        );
    }

    #[test]
    fn catalog_loads_every_entry_and_classifies_a_simple_command_as_runnable() {
        let s = session();
        let all = entries(&s);
        assert_eq!(all.len(), CATALOG.len());
        let bold = all
            .iter()
            .find(|e| e.id == "home.bold")
            .expect("home.bold is in the catalog");
        assert!(bold.enabled, "{:?}", bold.disabled_reason);
    }

    #[test]
    fn a_command_needing_a_param_the_app_cannot_supply_is_disabled_with_a_reason() {
        let s = session();
        let all = entries(&s);
        let open = all
            .iter()
            .find(|e| e.id == "file.open")
            .expect("file.open is in the catalog");
        assert!(!open.enabled);
        assert!(
            open.disabled_reason.as_deref().unwrap().contains("path"),
            "{:?}",
            open.disabled_reason
        );
    }

    /// #20: before `no_terminal_surface` existed, every one of these
    /// classified as runnable — each catalog entry's own `params` doc
    /// has every field optional — even though running one creates a
    /// floating object or print-only state this app's grid view never
    /// paints, so clicking the Insert menu item did nothing visible.
    #[test]
    fn floating_object_and_print_only_commands_are_disabled_with_a_reason() {
        let s = session();
        let all = entries(&s);
        for id in [
            "insert.textBox",
            "insert.wordArt",
            "insert.slicer",
            "pageLayout.headerFooter",
        ] {
            let entry = all.iter().find(|e| e.id == id).expect(id);
            assert!(!entry.enabled, "{id} should be disabled");
            assert!(
                entry
                    .disabled_reason
                    .as_deref()
                    .unwrap()
                    .contains("no terminal surface"),
                "{id}: {:?}",
                entry.disabled_reason
            );
        }
    }

    /// Chart/PivotChart/Table/PivotTable/Checkbox all paint through
    /// `CellGrid` or the chart panel (`crate::chart_view`) — none of
    /// them are the floating-object/print-only shape
    /// `no_terminal_surface` disables, so they stay runnable.
    #[test]
    fn commands_with_a_grid_or_chart_render_path_stay_enabled() {
        let s = session();
        let all = entries(&s);
        for id in [
            "insert.table",
            "insert.pivotTable",
            "insert.chart",
            "insert.pivotChart",
            "insert.checkbox",
        ] {
            let entry = all.iter().find(|e| e.id == id).expect(id);
            assert!(entry.enabled, "{id}: {:?}", entry.disabled_reason);
        }
    }

    #[test]
    fn fuzzy_query_narrows_to_matching_labels() {
        let s = session();
        let mut palette = CommandPalette::open(&s);
        assert_eq!(palette.filtered().len(), CATALOG.len());

        for c in "bold".chars() {
            assert_eq!(
                palette.handle_key(&Key::Char(c), &Modifiers::default()),
                Outcome::Changed
            );
        }
        let labels: Vec<&str> = palette.filtered().iter().map(|e| e.label).collect();
        assert!(labels.contains(&"Bold"), "{labels:?}");
        assert!(
            labels.len() < CATALOG.len(),
            "filtering should narrow the list"
        );
    }

    #[test]
    fn enter_on_the_selected_runnable_row_returns_its_id() {
        let s = session();
        let mut palette = CommandPalette::open(&s);
        for c in "bold".chars() {
            palette.handle_key(&Key::Char(c), &Modifiers::default());
        }
        assert_eq!(
            palette.handle_key(&Key::Named(NamedKey::Enter), &Modifiers::default()),
            Outcome::Run("home.bold")
        );
    }

    #[test]
    fn escape_closes_without_running_anything() {
        let s = session();
        let mut palette = CommandPalette::open(&s);
        assert_eq!(
            palette.handle_key(&Key::Named(NamedKey::Escape), &Modifiers::default()),
            Outcome::Closed
        );
    }
}
