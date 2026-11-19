# quadracalc: rules for workers

quadracalc is a quadraui frontend for ArtCraft's GridCraft spreadsheet engine. It runs as a TUI (local or over SSH) and as native GTK, macOS and Windows windows from one core crate.

## Architecture (don't change it without an issue saying so)
- **`quadracalc-core` holds everything.** It's backend-neutral, implements quadraui `AppLogic`, and owns the GridCraft `Session` and all view state. The four binaries stay a few lines each: build the app and hand it to `quadraui::{tui,gtk,macos,win}::run`.
- **The engine is used unchanged and only through its command surface.** Every change to a workbook goes through `Session::execute(id, params)` with GridCraft's own command ids (`cell.set`, `cell.get`, `range.setValues`, …). Don't reach into engine internals, and never reimplement something a command already does. The engine is pinned by full SHA in the root `Cargo.toml`; never fork or vendor it. A bug in the engine is an upstream issue for storytold/gridcraft.
- **`render(&self)` paints from `View`.** Reading the engine needs `&mut Session`, so the app refreshes `View` after every command and `render` only reads it. Keep it that way: no engine calls from `render`.
- **Paint through `ScreenLayout`.** Push `Surface`s, then `draw`. Don't call `backend.draw_*` directly. This is what keeps the app visible to quadraui's accessibility tree and to agents over MCP.
- **Use quadraui widgets; don't build widgets here.** If quadracalc needs a widget quadraui lacks, say so in your final message. It belongs upstream in quadraui.

## Naming and licence
- Don't call the app, a crate or a binary "GridCraft", and don't use ArtCraft's name or logos as branding. Describe it as "a quadraui frontend for the GridCraft engine".
- MIT OR Apache-2.0. Keep `third_party/gridcraft/` (their licence and NOTICE) intact.

## Testing
- Core logic gets unit tests in `quadracalc-core`, and they must not need a display.
- Behaviour visible on screen gets a black-box test driving the TUI through quadraui's `TuiDriver`, asserting on rendered output.
- Run `cargo test -p quadracalc-core` and `cargo clippy --workspace --exclude quadracalc-gtk -- -D warnings` before you finish. On Linux, include `quadracalc-gtk`.

## Process
- `gh` is on the deny-list; use plain `git`.
- Commit and push before your final message, even if unfinished.
- Don't edit README.md or CHANGELOG; the coordinator owns docs.
