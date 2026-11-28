//! Formulas tab: "Insert Function…" and the Function Library category
//! buttons (Financial, Logical, Text, Date & Time, Lookup & Reference,
//! Math & Trig, More Functions).
//!
//! Running any of those ids the way every other palette row runs —
//! `palette.rs`'s plain `execute(id, {})` — is a no-op this app can
//! observe: `formulas.insertFunction` with no `name` only queues a
//! `UiRequest::Dialog` this app never drains (`app.rs`'s `save` is the
//! one place that reads `session.ui_requests` at all, and only to check
//! for a `"saveAs"` kind), and every category id just runs
//! `formulas.functions` and discards the list it returns. #21: a
//! searchable function list is exactly the kind of thing an Excel power
//! user would try first from this menu, so this builds a minimal one
//! instead of leaving the item silently do nothing.
//!
//! Shaped like `palette.rs`'s own [`crate::palette::CommandPalette`]:
//! holds fuzzy-filtered state over a fixed list, reusing
//! `quadraui::fuzzy_score` the same way, and exposes a quadraui
//! [`Palette`] for `app.rs` to push through `ScreenLayout` — no bespoke
//! widget, per `CLAUDE.md`'s "use quadraui widgets; don't build widgets
//! here". The list itself is `formulas.functions`'s own return value
//! (`{category?, search?} → [{name, category, signature, description}]`)
//! — `app.rs` runs that command through `Session::execute` and hands the
//! parsed result to [`FunctionPicker::open`], so this module never reads
//! `gridcraft_engine` function data directly, only the command surface.
//!
//! Picking a row doesn't run a command: it types the function's name
//! plus `()` into whatever edit is active (`app.rs` opens one on the
//! formula bar first if none is in progress yet), the same keys a user
//! would type, so the caret lands right after the closing paren exactly
//! like it would after typing it by hand.

use quadraui::{
    Key, Modifiers, NamedKey, Palette, PaletteItem, PaletteMode, StyledText, WidgetId, fuzzy_score,
    next_char_boundary, prev_char_boundary,
};
use serde_json::{Value, json};

/// One `formulas.functions` row: name, category, call signature and
/// one-line description — everything GridCraft's own function catalog
/// returns for a function.
#[derive(Debug, Clone)]
pub struct FunctionEntry {
    pub name: String,
    pub category: String,
    pub signature: String,
    pub description: String,
}

/// `formulas.functions`'s own `category` param for the Formulas-tab id
/// that opens this picker pre-filtered to one category — `None` for
/// `formulas.insertFunction` itself (opens the full catalog) and for
/// every id this module doesn't claim at all (see [`has_picker`]).
fn category_param(id: &str) -> Option<&'static str> {
    match id {
        "formulas.financial" => Some("Financial"),
        "formulas.logical" => Some("Logical"),
        "formulas.text" => Some("Text"),
        "formulas.dateTime" => Some("DateTime"),
        "formulas.lookupReference" => Some("Lookup"),
        "formulas.mathTrig" => Some("MathTrig"),
        // Matches `formulas.moreFunctions`'s own default category (the
        // engine's `cmd/extra.rs`: `category.unwrap_or("Statistical")`)
        // since this picker has no sub-menu of its own to pick
        // Engineering/Information/Database/Compatibility/Web/Cube with
        // instead — typing further narrows to any of those by name.
        "formulas.moreFunctions" => Some("Statistical"),
        _ => None,
    }
}

/// Whether `id` is one of the Formulas-tab ids this module opens a
/// picker for, in place of `palette.rs` running it generically.
pub fn has_picker(id: &str) -> bool {
    id == "formulas.insertFunction" || category_param(id).is_some()
}

/// `formulas.functions`'s own params for opening the picker at `id` —
/// `app.rs` passes this straight to `execute`.
pub fn query_params(id: &str) -> Value {
    match category_param(id) {
        Some(category) => json!({ "category": category }),
        None => json!({}),
    }
}

/// Parse `formulas.functions`'s own return value into this module's
/// entries. A row missing a `name` (shouldn't happen — the command's own
/// doc is the contract) is skipped rather than panicking; every other
/// field defaults to empty text.
pub fn parse_entries(functions: &Value) -> Vec<FunctionEntry> {
    functions
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| {
            let name = f.get("name")?.as_str()?.to_string();
            let field = |key: &str| {
                f.get(key)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            Some(FunctionEntry {
                name,
                category: field("category"),
                signature: field("signature"),
                description: field("description"),
            })
        })
        .collect()
}

/// What a key handled by [`FunctionPicker::handle_key`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Query, cursor or selection changed — redraw, picker stays open.
    Changed,
    /// Enter on the selected row: type this function's name plus `()`
    /// into the active edit and close the picker.
    Insert(String),
    /// Escape: close the picker with no further action.
    Closed,
    /// The key meant nothing to the picker (including Enter on an empty
    /// filtered list).
    Ignored,
}

/// Open, query-filtered state for the function picker overlay. `app.rs`
/// holds one of these in `Some` while it's open, feeds it every key, and
/// renders it with [`FunctionPicker::palette`] — the same shape
/// `crate::palette::CommandPalette` follows.
pub struct FunctionPicker {
    entries: Vec<FunctionEntry>,
    query: String,
    /// Byte offset of the query's edit caret.
    cursor: usize,
    /// `(index into entries, match byte positions in that entry's name)`,
    /// in descending score order — the picker's displayed rows.
    filtered: Vec<(usize, Vec<usize>)>,
    selected: usize,
}

impl FunctionPicker {
    /// Show every one of `entries` (already filtered to one category by
    /// whatever `formulas.functions` call `app.rs` made, per
    /// [`query_params`]) with no further query filtering yet, selecting
    /// the first row.
    pub fn open(entries: Vec<FunctionEntry>) -> Self {
        let mut picker = Self {
            entries,
            query: String::new(),
            cursor: 0,
            filtered: Vec::new(),
            selected: 0,
        };
        picker.refilter();
        picker
    }

    /// The current query text (for tests).
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The currently visible rows, in display order — what
    /// [`Self::palette`] renders and [`Self::handle_key`]'s Enter/arrows
    /// act on.
    pub fn filtered(&self) -> Vec<&FunctionEntry> {
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
                let haystack = format!("{} {} {}", entry.name, entry.category, entry.description)
                    .to_lowercase();
                let (score, _) = fuzzy_score(&haystack, &query)?;
                let positions = fuzzy_score(&entry.name.to_lowercase(), &query)
                    .map(|(_, positions)| positions)
                    .unwrap_or_default();
                Some((score, idx, positions))
            })
            .collect();
        // Higher score first; ties keep the catalog's own (alphabetical,
        // per `gridcraft_engine::cmd::formulas::function_list`) order
        // rather than whatever order `filter_map` happened to produce.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        self.filtered = scored.into_iter().map(|(_, idx, pos)| (idx, pos)).collect();
        self.selected = 0;
    }

    /// Feed one key event to the picker. The caller (`app.rs`) routes
    /// every `KeyPressed` here while the picker is open and ignores
    /// everything else, matching the palette's own "intercept all input
    /// while modal" contract.
    pub fn handle_key(&mut self, key: &Key, modifiers: &Modifiers) -> Outcome {
        match key {
            Key::Named(NamedKey::Escape) => Outcome::Closed,
            Key::Named(NamedKey::Enter) => match self.filtered.get(self.selected) {
                Some(&(idx, _)) => Outcome::Insert(self.entries[idx].name.clone()),
                None => Outcome::Ignored,
            },
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
            // shortcut the picker doesn't understand, not a character to
            // insert. Matches `CommandPalette::handle_key`'s own guard.
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
    /// through `ScreenLayout`. Each row's detail is the function's
    /// signature and description, standing in for `CommandPalette`'s own
    /// tab/group breadcrumb.
    pub fn palette(&self) -> Palette {
        let items = self
            .filtered
            .iter()
            .map(|(idx, positions)| {
                let entry = &self.entries[*idx];
                PaletteItem {
                    text: StyledText::plain(entry.name.clone()),
                    detail: Some(StyledText::plain(format!(
                        "{} — {}",
                        entry.signature, entry.description
                    ))),
                    icon: None,
                    match_positions: positions.clone(),
                    depth: 0,
                    expandable: false,
                    expanded: false,
                }
            })
            .collect();
        Palette {
            id: WidgetId::new("quadracalc-function-picker"),
            title: "Insert Function".to_string(),
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
            show_mode_badge: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, category: &str) -> FunctionEntry {
        FunctionEntry {
            name: name.to_string(),
            category: category.to_string(),
            signature: format!("{name}(…)"),
            description: format!("{name} description"),
        }
    }

    #[test]
    fn has_picker_matches_insert_function_and_every_category() {
        for id in [
            "formulas.insertFunction",
            "formulas.financial",
            "formulas.logical",
            "formulas.text",
            "formulas.dateTime",
            "formulas.lookupReference",
            "formulas.mathTrig",
            "formulas.moreFunctions",
        ] {
            assert!(has_picker(id), "{id} should open the function picker");
        }
        assert!(!has_picker("formulas.nameManager"));
        assert!(!has_picker("home.bold"));
    }

    #[test]
    fn query_params_carries_the_category_except_for_insert_function() {
        assert_eq!(query_params("formulas.insertFunction"), json!({}));
        assert_eq!(
            query_params("formulas.financial"),
            json!({ "category": "Financial" })
        );
        assert_eq!(
            query_params("formulas.moreFunctions"),
            json!({ "category": "Statistical" })
        );
    }

    #[test]
    fn parse_entries_reads_every_field_and_skips_a_nameless_row() {
        let v = json!([
            {"name": "SUM", "category": "MathTrig", "signature": "SUM(…)", "description": "Adds."},
            {"category": "Logical"},
        ]);
        let parsed = parse_entries(&v);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "SUM");
        assert_eq!(parsed[0].category, "MathTrig");
        assert_eq!(parsed[0].description, "Adds.");
    }

    #[test]
    fn opening_shows_every_entry_unfiltered() {
        let picker =
            FunctionPicker::open(vec![entry("SUM", "MathTrig"), entry("VLOOKUP", "Lookup")]);
        assert_eq!(picker.filtered().len(), 2);
        assert_eq!(picker.query(), "");
    }

    #[test]
    fn typing_narrows_by_fuzzy_match_on_name() {
        let mut picker = FunctionPicker::open(vec![
            entry("SUM", "MathTrig"),
            entry("VLOOKUP", "Lookup"),
            entry("SUMIF", "MathTrig"),
        ]);
        for c in "sum".chars() {
            assert_eq!(
                picker.handle_key(&Key::Char(c), &Modifiers::default()),
                Outcome::Changed
            );
        }
        let names: Vec<&str> = picker.filtered().iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"SUM"));
        assert!(names.contains(&"SUMIF"));
        assert!(!names.contains(&"VLOOKUP"));
    }

    #[test]
    fn enter_on_the_selected_row_inserts_its_name() {
        let mut picker = FunctionPicker::open(vec![entry("SUM", "MathTrig")]);
        assert_eq!(
            picker.handle_key(&Key::Named(NamedKey::Enter), &Modifiers::default()),
            Outcome::Insert("SUM".to_string())
        );
    }

    #[test]
    fn escape_closes_without_inserting_anything() {
        let mut picker = FunctionPicker::open(vec![entry("SUM", "MathTrig")]);
        assert_eq!(
            picker.handle_key(&Key::Named(NamedKey::Escape), &Modifiers::default()),
            Outcome::Closed
        );
    }

    #[test]
    fn enter_on_an_empty_filtered_list_is_ignored() {
        let mut picker = FunctionPicker::open(vec![entry("SUM", "MathTrig")]);
        picker.handle_key(&Key::Char('z'), &Modifiers::default());
        picker.handle_key(&Key::Char('z'), &Modifiers::default());
        assert!(picker.filtered().is_empty());
        assert_eq!(
            picker.handle_key(&Key::Named(NamedKey::Enter), &Modifiers::default()),
            Outcome::Ignored
        );
    }
}
