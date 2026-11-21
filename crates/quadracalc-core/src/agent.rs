//! Driving quadracalc from an AI agent over MCP.
//!
//! Wires [`quadraui_mcp`] — the MCP server quadraui itself exposes over a
//! running app's backend-neutral [`quadraui::control`] channel — into
//! [`CalcApp`], and registers three "Level 2" app commands (see
//! `quadraui::control`'s "Levels" doc section) alongside the five
//! built-in Level 1 tools (`snapshot`, `act`, `key`, `type`,
//! `screenshot`) every `quadraui-mcp` server already answers for any
//! `AppLogic` app:
//!
//! - `set_cell(cell, input)` — GridCraft's `cell.set`, the same command
//!   a keyboard edit runs.
//! - `get_range(range)` — reads a range's values and formulas (two
//!   `sheet.read` calls).
//! - `run_command(id, params)` — passes straight through to
//!   `gridcraft_engine::Session::execute`, so an agent can reach any
//!   command the engine knows about without this crate adding a
//!   dedicated wrapper for each one.
//!
//! Every command above only calls [`CalcApp`]'s own public methods
//! (`execute`, `set_cell`) — never the engine directly — so an
//! agent-run command changes the workbook exactly the way a keyboard
//! edit does. The one difference from a keyboard edit: `CalcApp::handle`
//! has a `&mut dyn Backend` to refresh [`crate::View`] with on the spot,
//! but `quadraui::AppLogic::run_command` (what a control-channel
//! `Command` request calls) does not. [`CalcApp`] works around that by
//! marking the view stale and refreshing it from `AppLogic::tick`, the
//! next hook that *does* get a backend — see that impl's own comment.
//! The agent-visible effect is a one-frame delay (the same kind of delay
//! `quadraui-mcp`'s own round-trip test polls `snapshot` to ride out)
//! rather than no repaint at all.

use quadraui::control::{CommandError, CommandRegistry};
use serde_json::{Value, json};

use crate::app::CalcApp;

/// Builds the registry [`CalcApp`] stashes as a field and runs through
/// [`quadraui::AppLogic::run_command`] — see that trait method's own doc
/// for the "swap the field out, call, swap it back" dance every call
/// site (including [`CalcApp`]'s) uses so a registered closure here can
/// take `&mut CalcApp` without aliasing the field that holds it.
pub(crate) fn registry() -> CommandRegistry<CalcApp> {
    let mut commands = CommandRegistry::new();
    commands.register(
        "set_cell",
        "Enters text, a number or a formula into one cell — GridCraft's \
         cell.set, the same command a keyboard edit runs.",
        json!({
            "type": "object",
            "properties": {
                "cell": {
                    "type": "string",
                    "description": "An A1 cell reference, e.g. \"B2\".",
                },
                "input": {
                    "type": "string",
                    "description": "Text, a number, or a formula starting with \"=\".",
                },
            },
            "required": ["cell", "input"],
            "additionalProperties": false,
        }),
        |app: &mut CalcApp, args: Value| {
            let cell = string_arg(&args, "cell")?;
            let input = string_arg(&args, "input")?;
            app.set_cell(&cell, &input)
                .map_err(|e| CommandError::Failed(e.to_string()))?;
            Ok(json!({"ok": true}))
        },
    );
    commands.register(
        "get_range",
        "Reads a range's displayed values and formulas (GridCraft's \
         sheet.read, called once for each).",
        json!({
            "type": "object",
            "properties": {
                "range": {
                    "type": "string",
                    "description": "An A1 range, e.g. \"A1:C10\". Defaults to the \
                        sheet's used range.",
                },
            },
            "additionalProperties": false,
        }),
        |app: &mut CalcApp, args: Value| {
            let range = args.get("range").and_then(Value::as_str);
            let mut params = json!({});
            if let Some(range) = range {
                params["range"] = json!(range);
            }
            let values = app
                .execute("sheet.read", params.clone())
                .map_err(|e| CommandError::Failed(e.to_string()))?;
            params["formulas"] = json!(true);
            let formulas = app
                .execute("sheet.read", params)
                .map_err(|e| CommandError::Failed(e.to_string()))?;
            Ok(json!({
                "range": values.get("range").cloned().unwrap_or(Value::Null),
                "values": values.get("values").cloned().unwrap_or_else(|| json!([])),
                "formulas": formulas.get("values").cloned().unwrap_or_else(|| json!([])),
            }))
        },
    );
    commands.register(
        "run_command",
        "Runs one GridCraft engine command by id, passing params through \
         unchanged (Session::execute's own command surface — cell.set, \
         range.setValues, edit.undo, …).",
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "A GridCraft command id, e.g. \"cell.set\".",
                },
                "params": {
                    "description": "The command's own params object. Defaults to {}.",
                },
            },
            "required": ["id"],
            "additionalProperties": false,
        }),
        |app: &mut CalcApp, args: Value| {
            let id = string_arg(&args, "id")?;
            let params = args.get("params").cloned().unwrap_or_else(|| json!({}));
            app.execute(&id, params)
                .map_err(|e| CommandError::Failed(e.to_string()))
        },
    );
    commands
}

fn string_arg(args: &Value, name: &str) -> Result<String, CommandError> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| CommandError::InvalidArgs(format!("\"{name}\" is required")))
}

/// Where quadracalc serves `quadraui-mcp`: `$QUADRACALC_MCP` if set and
/// parseable (see [`quadraui_mcp::Transport::parse`] for the accepted
/// spec strings — `"stdio"`, `"unix:<path>"`, `"tcp:<host>:<port>"`),
/// otherwise a per-process Unix-domain socket under the OS temp dir on
/// Unix, or a fixed loopback TCP port on Windows (no `Transport::Unix`
/// variant exists on a non-Unix target — see that type's own doc).
/// Mirrors `quadraui`'s own `tui_mcp_demo` example.
pub fn mcp_transport() -> quadraui_mcp::Transport {
    if let Some(t) = std::env::var("QUADRACALC_MCP")
        .ok()
        .and_then(|spec| quadraui_mcp::Transport::parse(&spec))
    {
        return t;
    }
    #[cfg(unix)]
    {
        quadraui_mcp::Transport::Unix(
            std::env::temp_dir().join(format!("quadracalc-mcp-{}.sock", std::process::id())),
        )
    }
    #[cfg(not(unix))]
    {
        quadraui_mcp::Transport::Tcp("127.0.0.1:7777".parse().expect("valid loopback address"))
    }
}

/// Spawns `quadraui-mcp` on a background thread against `handle`/
/// `transport`, logging any transport error to stderr. Every binary
/// (TUI, GTK, macOS, Windows) calls this the same way, right after its
/// own runner's `attach_control`: the server has to run off the thread
/// that owns the event loop, the same reason `quadraui`'s own
/// `tui_mcp_demo` example spawns it rather than calling
/// [`quadraui_mcp::serve`] inline.
pub fn spawn_mcp_server(
    handle: quadraui::control::ControlHandle,
    transport: quadraui_mcp::Transport,
) {
    std::thread::spawn(move || {
        if let Err(e) = quadraui_mcp::serve(handle, transport) {
            eprintln!("quadracalc: quadraui-mcp server exited: {e}");
        }
    });
}
