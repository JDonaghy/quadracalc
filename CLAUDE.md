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
- Behaviour visible on screen gets a black-box test written once, generic over `quadraui::testing::ConformanceDriver`, in `quadracalc-core/tests/conformance/mod.rs` — then run against both `TuiDriver` (`quadracalc-core/tests/conformance_tui.rs`) and `GtkDriver` (`quadracalc-gtk/tests/conformance_gtk.rs`), not a TUI-only test. Locate with `click_text`/`type_text`/`press_named`, assert with `screen_has` — never a literal `click(x, y)`, since TUI cells and GTK pixels are different units.
- A `TuiDriver`-only test is fine only when the behaviour is genuinely TUI-specific (character-grid string matching, a TUI-only shortcut) or GTK can't render the surface under test yet — say which, in the test's own doc comment. The older per-file `TuiDriver` tests already in `editing.rs`/`clipboard.rs`/`sheets.rs`/etc. aren't retroactively required to move; this is the bar for new screen-visible behaviour going forward.
- `MacDriver`/`WinDriver` exist too (same `ConformanceDriver` shape) but aren't required per-PR — each only compiles on its own platform, so adding one is its own issue dispatched to a machine that can build it, not a blocker for a feature PR.
- Run `cargo test -p quadracalc-core`, `cargo test -p quadracalc-gtk`, and `cargo clippy --workspace --exclude quadracalc-gtk -- -D warnings` before you finish. On Linux, include `quadracalc-gtk` in the clippy run too.

## Process
- `gh` is on the deny-list; use plain `git`.
- Commit and push before your final message, even if unfinished.
- Don't edit README.md or CHANGELOG; the coordinator owns docs.
