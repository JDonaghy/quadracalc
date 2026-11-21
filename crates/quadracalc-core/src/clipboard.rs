//! Clipboard and fill: Ctrl+C/X/V (Cmd on macOS) through the engine's own
//! `edit.copy`/`edit.cut`/`edit.paste`, and Ctrl+D/Ctrl+R through
//! `edit.fillDown`/`edit.fillRight` — see `CalcApp::handle`'s own
//! shortcut dispatch, which is the only caller of everything here.
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
//! - `paste` reads the system clipboard first. `edit.paste` treats a
//!   `text` param that differs from what the engine's own clipboard
//!   already holds as plain TSV pasted from outside and fills the range
//!   from the cursor; text that matches (or no system clipboard text at
//!   all, e.g. nothing was ever copied) falls through to the engine's own
//!   richer internal clipboard instead — see `edit.paste`'s own doc.
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
    if let Some(text) = result.get("text").and_then(Value::as_str) {
        backend.services().clipboard().write_text(text);
    }
    Ok(())
}

/// Ctrl+V: `edit.paste` at the cursor, with whatever plain text the
/// system clipboard holds right now (if any) passed through as `text` —
/// see the module doc for how the engine picks between that and its own
/// internal clipboard.
pub(crate) fn paste(app: &mut CalcApp, backend: &mut dyn Backend) -> anyhow::Result<()> {
    app.sync_selection()?;
    let mut params = json!({});
    if let Some(text) = backend.services().clipboard().read_text() {
        params["text"] = Value::String(text);
    }
    app.execute("edit.paste", params)?;
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
