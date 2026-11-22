//! Classic drop-down menu bar, built from GridCraft's own command
//! catalog (`gridcraft_engine::catalog::CATALOG`) rather than a
//! hand-written list — one [`MenuDef`] per ribbon tab, grouped and
//! labelled exactly as [`crate::palette`] already classifies them, so
//! the menu can never offer a command this build can't actually run.
//!
//! Scoped to the tabs a terminal spreadsheet session reaches for first;
//! [`TABS`] is the only place that needs editing to surface more of the
//! catalog later. `File` gets one item with no catalog counterpart —
//! "Quit" — appended by [`menus`] itself.
//!
//! `Home` alone carries ~90 catalog placements across eight groups
//! (Clipboard, Font, Alignment, Number, Styles, Cells, Editing, Add-ins)
//! — too dense for one flat dropdown — so [`home_items`] wraps each
//! group in its own [`ContextMenuItem::submenu`] instead of
//! [`tab_items`]'s flat separator-between-groups shape every other tab
//! still uses. `Undo`/`Redo` have no `CATALOG` placement of their own
//! (ribbon parity puts them on Excel's Quick Access Toolbar, not a
//! ribbon tab) but are still the first things a terminal user reaches
//! for, so [`home_items`] also prepends them straight from
//! `gridcraft_engine::find_command` rather than `palette::entries`.
//!
//! Every item whose keyboard shortcut already works in `CalcApp::handle`
//! (Save, Undo, Redo, Cut, Copy, Paste, Fill Down, Fill Right) gets a
//! [`ContextMenuItem::key_equivalent`] via [`shortcut_for`], rendered
//! platform-correctly (`⌘S` on macOS, `Ctrl+S` elsewhere) by
//! `render_accelerator` — the same shortcut, just a visible hint, since
//! none of this crate's shortcuts go through quadraui's own accelerator
//! registry (see `app.rs`'s own `KeyPressed` guards).

use gridcraft_engine::{Session, find_command};
use quadraui::{
    Accelerator, AcceleratorId, AcceleratorScope, ContextMenuItem, KeyBinding, MenuDef, StyledText,
    WidgetId,
};

use crate::palette::{self, Entry};

/// Ribbon tabs surfaced here, in display order. Contextual tabs (Page
/// Layout, Review, Draw, Automate, Developer, and the chart/table/pivot
/// tabs that only exist once their object is selected) are left out of
/// this first pass rather than shown mostly disabled.
const TABS: &[(&str, &str)] = &[
    ("File", "&File"),
    ("Home", "&Home"),
    ("Insert", "&Insert"),
    ("Formulas", "For&mulas"),
    ("Data", "&Data"),
    ("View", "&View"),
];

/// Sentinel id for the one menu item with no catalog counterpart —
/// handled directly by `CalcApp::handle`, never passed to
/// `Session::execute`.
pub const QUIT: &str = "quadracalc.quit";

/// Build this frame's menu bar from `session`'s current classification
/// (`palette::entries`) — the same enabled/disabled-with-reason logic
/// the command palette already uses, so e.g. "Undo" greys out the
/// moment there's nothing left to undo.
pub fn menus(session: &Session) -> Vec<MenuDef> {
    let entries = palette::entries(session);
    let mut menus: Vec<MenuDef> = TABS
        .iter()
        .map(|&(tab, label)| MenuDef {
            id: WidgetId::new(format!("menu:{}", tab.to_lowercase())),
            label: label.to_string(),
            disabled: false,
            items: if tab == "Home" {
                home_items(session, &entries)
            } else {
                tab_items(&entries, tab)
            },
        })
        .collect();

    if let Some(file) = menus.first_mut() {
        // Guarded rather than unconditional: today's catalog always
        // gives File real entries, but a future reclassification that
        // ever left it empty would otherwise render a leading separator
        // with nothing above it.
        if !file.items.is_empty() {
            file.items.push(separator());
        }
        file.items.push(action(QUIT, "Quit"));
    }
    menus
}

/// One tab's items, catalog order, with a separator between groups.
fn tab_items(entries: &[Entry], tab: &str) -> Vec<ContextMenuItem> {
    let mut items = Vec::new();
    let mut last_group: Option<&str> = None;
    for e in entries.iter().filter(|e| e.tab == tab) {
        if last_group.is_some_and(|g| g != e.group) {
            items.push(separator());
        }
        last_group = Some(e.group);
        items.push(classified_action(e));
    }
    items
}

/// Home's own dropdown: `Undo`/`Redo` (see this module's own top doc for
/// why they're built here rather than from `palette::entries`) above a
/// separator, then every Home catalog entry grouped into one
/// [`ContextMenuItem::submenu`] per `Entry::group`, in the order each
/// group is first encountered — catalog order, same as [`tab_items`]'s
/// flat shape preserves for every other tab.
fn home_items(session: &Session, entries: &[Entry]) -> Vec<ContextMenuItem> {
    let mut items = undo_redo_items(session);
    items.push(separator());

    let mut groups: Vec<(&str, Vec<ContextMenuItem>)> = Vec::new();
    for e in entries.iter().filter(|e| e.tab == "Home") {
        let item = classified_action(e);
        match groups.iter_mut().find(|(group, _)| *group == e.group) {
            Some((_, group_items)) => group_items.push(item),
            None => groups.push((e.group, vec![item])),
        }
    }
    items.extend(
        groups
            .into_iter()
            .map(|(group, group_items)| group_submenu(group, group_items)),
    );
    items
}

/// `Undo`/`Redo` items built straight from `gridcraft_engine::find_command`
/// instead of `palette::entries`, since neither id has a `CATALOG`
/// placement (this module's own top doc explains why) for `classify` to
/// classify. Enabled state mirrors `CommandSpec::info` exactly the way
/// every catalog-backed item already does.
fn undo_redo_items(session: &Session) -> Vec<ContextMenuItem> {
    [
        ("edit.undo", "Undo", KeyBinding::Undo),
        // Not `KeyBinding::Redo`: that renders "Ctrl+Shift+Z", but
        // `CalcApp::handle` only dispatches redo on `Ctrl/Cmd+Y` (and its
        // undo arm, just above in `app.rs`, matches `Ctrl/Cmd+Z` while
        // ignoring Shift — so "Ctrl+Shift+Z" would actually undo).
        (
            "edit.redo",
            "Redo",
            KeyBinding::Literal("Ctrl+Y".to_string()),
        ),
    ]
    .into_iter()
    .map(|(id, label, binding)| {
        let mut item = action(id, label);
        item.key_equivalent = Some(accelerator(id, binding));
        // Unreachable today — both ids are registered by
        // `gridcraft_engine::cmd::edit` unconditionally — but surfaced as
        // "stays enabled" rather than silently dropped if a future
        // engine build ever stopped registering one of them.
        if let Some(spec) = find_command(id) {
            let info = spec.info(session);
            if !info.enabled {
                item.disabled = true;
                item.detail = info.disabled_reason.map(StyledText::plain);
            }
        }
        item
    })
    .collect()
}

/// One catalog entry's menu item: its label, and — when `e.id` is one of
/// the ids [`shortcut_for`] knows — the accelerator hint for its already-
/// working keyboard shortcut. Disabled the same way every catalog-backed
/// item already was before submenus existed.
fn classified_action(e: &Entry) -> ContextMenuItem {
    let mut item = action(e.id, e.label);
    if let Some(binding) = shortcut_for(e.id) {
        item.key_equivalent = Some(accelerator(e.id, binding));
    }
    // `palette::entries` already classifies "Paste" (`edit.paste`) and
    // every `format_picker`-backed id as enabled — see that module's own
    // doc for why — so this needs no override of its own;
    // `CalcApp::handle`'s menu dispatch special-cases both the same way
    // the palette's own dispatch does.
    if !e.enabled {
        item.disabled = true;
        item.detail = e.disabled_reason.as_deref().map(StyledText::plain);
    }
    item
}

/// An [`Accelerator`] for `id`'s shortcut, which `CalcApp::handle` already
/// dispatches straight off raw `KeyPressed` events rather than through
/// quadraui's own accelerator registry — so this never changes what the
/// registry itself does. It still drives more than a rendered hint on
/// macOS, though: `quadraui::macos::menu_bar_install` installs every
/// leaf's `key_equivalent` as a *real* `NSMenu` key equivalent, so AppKit
/// matches it before `handle` ever sees the keypress and delivers
/// `MenuEvent::Activated` instead. `CalcApp::handle`'s own `Activated` arm
/// special-cases exactly the ids that need more than a bare `execute` —
/// Copy, Cut, Paste, Save — the same way the keyboard dispatch does, so
/// that reroute stays behaviourally equivalent.
fn accelerator(id: &str, binding: KeyBinding) -> Accelerator {
    Accelerator {
        id: AcceleratorId::new(id),
        binding,
        scope: AcceleratorScope::Global,
        label: None,
    }
}

/// The [`KeyBinding`] for every catalog id `CalcApp::handle` already
/// dispatches a working keyboard shortcut for — `None` for everything
/// else, which leaves [`ContextMenuItem::key_equivalent`] unset. Fill
/// Down/Fill Right have no universal `KeyBinding` variant, so they're
/// spelled out as [`KeyBinding::Literal`]s matching `app.rs`'s own
/// `modifiers.ctrl || modifiers.cmd` guard for `d`/`r`.
fn shortcut_for(id: &str) -> Option<KeyBinding> {
    match id {
        "file.save" => Some(KeyBinding::Save),
        "edit.cut" => Some(KeyBinding::Cut),
        "edit.copy" => Some(KeyBinding::Copy),
        "edit.paste" => Some(KeyBinding::Paste),
        "edit.fillDown" => Some(KeyBinding::Literal("Ctrl+D".to_string())),
        "edit.fillRight" => Some(KeyBinding::Literal("Ctrl+R".to_string())),
        _ => None,
    }
}

fn action(id: &str, label: &str) -> ContextMenuItem {
    ContextMenuItem {
        id: Some(WidgetId::new(id)),
        label: StyledText::plain(label),
        ..Default::default()
    }
}

/// A submenu-parent item for one `Entry::group`. Needs its own `id` (a
/// synthetic one, distinct from every catalog id) even though clicking it
/// never reaches `CalcApp::handle` as `MenuEvent::Activated` — `quadraui`'s
/// `MenuSystem` opens the nested popup instead — because
/// `ContextMenuItem::is_separator` (and the TUI rasteriser's hit-testing)
/// treats a `None` id as a non-interactive separator, which a submenu
/// parent plainly isn't.
fn group_submenu(group: &str, items: Vec<ContextMenuItem>) -> ContextMenuItem {
    ContextMenuItem {
        id: Some(WidgetId::new(format!(
            "menu:home:{}",
            group.to_lowercase().replace([' ', '-'], "_")
        ))),
        label: StyledText::plain(group),
        submenu: Some(items),
        ..Default::default()
    }
}

fn separator() -> ContextMenuItem {
    ContextMenuItem::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use quadraui::{Platform, render_binding};

    /// Depth-first search through `items` and every nested
    /// `ContextMenuItem::submenu`, for assertions below that no longer
    /// know (or care) which submenu an id landed in — `home_items` groups
    /// catalog entries into submenus by `Entry::group`, so a flat
    /// `items.iter().find(...)` the way pre-submenu tests used no longer
    /// reaches most Home ids.
    fn find_recursive<'a>(items: &'a [ContextMenuItem], id: &str) -> Option<&'a ContextMenuItem> {
        for item in items {
            if item.id == Some(WidgetId::new(id)) {
                return Some(item);
            }
            if let Some(sub) = &item.submenu
                && let Some(found) = find_recursive(sub, id)
            {
                return Some(found);
            }
        }
        None
    }

    #[test]
    fn file_menu_ends_in_a_separator_then_quit() {
        let mut session = Session::new();
        session.new_workbook();
        let file = &menus(&session)[0];
        assert_eq!(file.label, "&File");
        let last = file.items.last().unwrap();
        assert_eq!(last.id, Some(WidgetId::new(QUIT)));
        let separator = &file.items[file.items.len() - 2];
        assert_eq!(separator.id, None);
    }

    #[test]
    fn edit_paste_is_enabled() {
        let mut session = Session::new();
        session.new_workbook();
        let home = menus(&session)
            .into_iter()
            .find(|m| m.label == "&Home")
            .unwrap();
        let paste = find_recursive(&home.items, "edit.paste").unwrap();
        assert!(!paste.disabled);
    }

    #[test]
    fn every_tab_is_non_empty_on_a_fresh_workbook() {
        let mut session = Session::new();
        session.new_workbook();
        for menu in menus(&session) {
            assert!(!menu.items.is_empty(), "{} has no items", menu.label);
        }
    }

    /// Home's catalog items (everything after `Undo`/`Redo` and their
    /// separator) are grouped into one `submenu` per `Entry::group`
    /// instead of one flat list — the density issue #49 is about — and a
    /// familiar group ("Font") still has its own familiar item ("Bold")
    /// nested inside, not sitting at the top level.
    #[test]
    fn home_catalog_items_are_grouped_into_submenus() {
        let mut session = Session::new();
        session.new_workbook();
        let home = menus(&session)
            .into_iter()
            .find(|m| m.label == "&Home")
            .unwrap();

        let font_group = home
            .items
            .iter()
            .find(|i| i.label == StyledText::plain("Font"))
            .expect("a \"Font\" submenu item");
        assert!(
            font_group.submenu.is_some(),
            "the \"Font\" group should be a submenu parent, not a flat leaf item"
        );
        assert!(
            home.items
                .iter()
                .all(|i| i.label != StyledText::plain("Bold")),
            "\"Bold\" should be nested inside its group's submenu, not sitting flat in Home"
        );
        let bold = font_group
            .submenu
            .as_ref()
            .unwrap()
            .iter()
            .find(|i| i.id == Some(WidgetId::new("home.bold")));
        assert!(bold.is_some(), "\"Bold\" should be inside the Font submenu");
    }

    /// `Undo`/`Redo` have no `CATALOG` placement, so they're built by
    /// `undo_redo_items` directly rather than from `palette::entries` —
    /// confirm they still show up, enabled, ahead of the grouped
    /// submenus on a fresh workbook with nothing yet to undo/redo... a
    /// fresh workbook does have nothing to undo, so assert the shape
    /// (present, with an accelerator) instead of enabled state.
    ///
    /// Asserts the *rendered* string, not just that some accelerator was
    /// set: `CalcApp::handle` only dispatches redo on Ctrl/Cmd+Y (its undo
    /// arm matches Ctrl/Cmd+Z and ignores Shift), so `KeyBinding::Redo`
    /// (which renders "Ctrl+Shift+Z") would advertise a combo that
    /// actually performs undo — the bug this test is here to catch.
    #[test]
    fn home_menu_has_undo_and_redo_with_shortcut_hints() {
        let mut session = Session::new();
        session.new_workbook();
        let home = menus(&session)
            .into_iter()
            .find(|m| m.label == "&Home")
            .unwrap();
        let undo = find_recursive(&home.items, "edit.undo").unwrap();
        assert_eq!(
            render_binding(
                &undo.key_equivalent.as_ref().unwrap().binding,
                Platform::Tui
            ),
            "Ctrl+Z"
        );
        let redo = find_recursive(&home.items, "edit.redo").unwrap();
        assert_eq!(
            render_binding(
                &redo.key_equivalent.as_ref().unwrap().binding,
                Platform::Tui
            ),
            "Ctrl+Y"
        );
    }

    /// Every id #49 lists as already having a working keyboard shortcut
    /// shows a `key_equivalent` hint in the dropdown now, wherever it
    /// landed after grouping.
    #[test]
    fn every_shortcut_bearing_item_has_a_key_equivalent() {
        let mut session = Session::new();
        session.new_workbook();
        let home = menus(&session)
            .into_iter()
            .find(|m| m.label == "&Home")
            .unwrap();
        let file = menus(&session)
            .into_iter()
            .find(|m| m.label == "&File")
            .unwrap();
        for id in [
            "edit.undo",
            "edit.redo",
            "edit.cut",
            "edit.copy",
            "edit.paste",
            "edit.fillDown",
            "edit.fillRight",
        ] {
            let item = find_recursive(&home.items, id)
                .unwrap_or_else(|| panic!("{id} should be somewhere in the Home menu"));
            assert!(
                item.key_equivalent.is_some(),
                "{id} should have a key_equivalent hint"
            );
        }
        let save = find_recursive(&file.items, "file.save").unwrap();
        assert!(
            save.key_equivalent.is_some(),
            "Save should have a key_equivalent hint"
        );
    }
}
