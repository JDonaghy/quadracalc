//! Backend-neutral core of quadracalc: a quadraui frontend for the GridCraft
//! spreadsheet engine.
//!
//! The engine is used unchanged. Every action goes through
//! [`gridcraft_engine::Session::execute`] with GridCraft's own command ids
//! (`cell.set`, `cell.get`, …), the same surface its egui app, CLI and MCP
//! server use. This crate owns view state (cursor, scroll) and builds quadraui
//! surfaces; the per-backend binaries only hand [`CalcApp`] to a runner.

mod address;
pub mod agent;
mod app;
mod chart_view;
mod clipboard;
pub mod format_picker;
mod formula_bar;
pub mod palette;
mod sheets;
mod sidebar;

pub use address::{Address, column_label};
pub use app::{CalcApp, View};
