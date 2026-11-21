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
