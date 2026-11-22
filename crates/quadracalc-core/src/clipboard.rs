//! Clipboard and fill: Ctrl+C/X/V (Cmd on macOS) through the engine's own
//! `edit.copy`/`edit.cut`/`edit.paste`, and Ctrl+D/Ctrl+R through
//! `edit.fillDown`/`edit.fillRight`. `copy`/`cut`/`fill_down`/
//! `fill_right` are called from `CalcApp::handle`'s own shortcut
//! dispatch directly; `paste` is called from
//! `CalcApp::apply_controller_event` instead, since quadraui's runtime
//! intercepts the paste keypress before `handle` ever sees it (see
//! `paste`'s own doc below).
//!
//! Reference adjustment (copy shifts relative references, cut doesn't),
//! TSV parsing and the actual cell writes are the engine's job
//! (`gridcraft_engine::cmd::edit`'s clipboard/fill modules) — this module
//! only bridges the UI's selection and the host's system clipboard onto
//! that command surface:
//!
//! - `copy`/`cut` sync the UI selection into the engine first (same
//!   reason `CalcApp::sync_selection` exists at all: a palette run or a
//!   shortcut has no explicit `range` param, so the engine falls back to
//!   whatever it last had selected), then mirror the copied/cut plain
//!   text onto the system clipboard (`Backend::services().clipboard()`,
//!   quadraui#2729) so it round-trips into another app.
//! - `paste` takes the anchor and text quadraui's own runtime already
//!   resolved before `AppLogic::handle` ever ran: the native paste
//!   keypress (Ctrl/Cmd+V) is intercepted upstream (`runtime::
//!   preprocess_event`'s step 6) and delivered as `UiEvent::
//!   ClipboardPaste(text)`, which `CellGridController::handle` turns into
//!   `CellGridControllerEvent::PasteRequested { anchor_row, anchor_col,
//!   text }` anchored at the selection's top-left corner — see
//!   `CalcApp::apply_controller_event`, the only caller of `paste` here.
//!   `edit.paste` treats a `text` param that differs from what the
//!   engine's own clipboard already holds as plain TSV pasted from
//!   outside and fills the range from `at`; text that matches (an in-app
//!   copy round-tripped through the system clipboard) falls through to
//!   the engine's own richer internal clipboard instead — see
//!   `edit.paste`'s own doc.
//! - `fill_down`/`fill_right` touch no clipboard at all, just the
//!   selection.
//! - `paste_from_system` (menu/palette Paste, which has no already-
//!   resolved `UiEvent::ClipboardPaste`) reads the system clipboard
//!   itself and falls back to a bare `edit.paste` — the engine's own
//!   internal clipboard — when that read fails, see its own doc.

use quadraui::Backend;
use serde_json::{Value, json};

use crate::app::CalcApp;

/// Ctrl+C: `edit.copy`, then mirror its plain text onto the system
/// clipboard.
pub(crate) fn copy(app: &mut CalcApp, backend: &mut dyn Backend) -> anyhow::Result<()> {
    run_and_mirror(app, backend, "edit.copy")
}

/// Ctrl+X: `edit.cut`, then mirror its plain text onto the system
/// clipboard the same as `copy`. The source cell isn't cleared yet —
/// `edit.cut` only arms the engine's clipboard; the move happens when
/// `paste` lands it elsewhere.
pub(crate) fn cut(app: &mut CalcApp, backend: &mut dyn Backend) -> anyhow::Result<()> {
    run_and_mirror(app, backend, "edit.cut")
}

fn run_and_mirror(app: &mut CalcApp, backend: &mut dyn Backend, id: &str) -> anyhow::Result<()> {
    app.sync_selection()?;
    let result = app.execute(id, json!({}))?;
    if let Some(text) = result.get("text").and_then(Value::as_str)
        && !text.is_empty()
    {
        backend.services().clipboard().write_text(text);
    }
    Ok(())
}

/// Ctrl+V: `edit.paste` at `anchor` (an A1 cell reference) with `text` —
/// both already resolved by quadraui before this ever runs, see the
/// module doc.
pub(crate) fn paste(app: &mut CalcApp, anchor: &str, text: &str) -> anyhow::Result<()> {
    app.execute("edit.paste", json!({ "at": anchor, "text": text }))?;
    Ok(())
}

/// Edit > Paste from the menu bar or the command palette: unlike Ctrl+V
/// (see the module doc), there's no anchor/text quadraui's runtime has
/// already resolved, so this reads the system clipboard directly and
/// pastes at the current selection's top-left corner — the same anchor
/// Ctrl+V's own already-resolved `PasteRequested` event uses, not the
/// raw cursor (see [`CalcApp::selection_anchor`]'s own doc for why those
/// two can differ). `palette::classify` marks `edit.paste` enabled
/// unconditionally for exactly this reason — see that module's own doc.
///
/// `backend.services().clipboard().read_text()` fails whenever there's no
/// real OS clipboard to read from — most notably a TUI over SSH with no
/// local desktop session, where quadraui's `arboard`-only read has no
/// OSC-52/native-tool fallback. When that happens this falls back to a
/// bare `edit.paste` with no `text` param at all (never an empty-string
/// `text`, which the engine would treat as literal external TSV and
/// overwrite the selection with blank cells), landing on the engine's
/// own internal `Session.clipboard` armed by a prior in-app
/// `edit.copy`/`edit.cut` — restoring the exact copy-then-paste round
/// trip that worked before this function read the system clipboard at
/// all. A blank-selection `copy` (see `run_and_mirror`) never touches the
/// OS clipboard but still arms that internal clipboard, so this fallback
/// is checked only when the system clipboard has nothing, never instead
/// of it.
pub(crate) fn paste_from_system(
    app: &mut CalcApp,
    backend: &mut dyn Backend,
) -> anyhow::Result<()> {
    let anchor = app.selection_anchor().a1();
    match backend.services().clipboard().read_text() {
        Some(text) => paste(app, &anchor, &text),
        None => {
            app.execute("edit.paste", json!({ "at": anchor }))?;
            Ok(())
        }
    }
}

/// Ctrl+D: `edit.fillDown` — fills the selection from its top row (or,
/// with a single cell selected, from the row above).
pub(crate) fn fill_down(app: &mut CalcApp, _backend: &mut dyn Backend) -> anyhow::Result<()> {
    app.sync_selection()?;
    app.execute("edit.fillDown", json!({}))?;
    Ok(())
}

/// Ctrl+R: `edit.fillRight` — fills the selection from its left column
/// (or, with a single cell selected, from the column to its left).
pub(crate) fn fill_right(app: &mut CalcApp, _backend: &mut dyn Backend) -> anyhow::Result<()> {
    app.sync_selection()?;
    app.execute("edit.fillRight", json!({}))?;
    Ok(())
}
