# quadracalc

A spreadsheet that runs in your terminal, over SSH, and as a native window on
macOS, Windows and Linux, from one codebase.

quadracalc is a [quadraui](https://github.com/JDonaghy/quadraui) frontend for
the **GridCraft engine** from [ArtCraft](https://github.com/storytold). It uses
GridCraft's spreadsheet engine unchanged (formulas, recalculation, number
formats, xlsx import/export, undo) and drives it through the same command
surface GridCraft's own app, CLI and MCP server use:
`Session::execute("cell.set", …)`.

> quadracalc is an independent community project. It is not made or endorsed by
> ArtCraft, and "GridCraft" and "ArtCraft" are ArtCraft's trademarks.

## Status

Early. The workspace links the engine and shows a live sheet with a stand-in
grid. The real spreadsheet grid (cell cursor, range selection, frozen panes,
in-cell editing) is being built in quadraui as the CellGrid primitive
([quadraui#2732](https://github.com/JDonaghy/quadraui/issues/2732)).

## Run

```sh
cargo run -p quadracalc                         # terminal (works over SSH)
cargo run -p quadracalc-gtk                     # GTK4 window (Linux)
cargo run -p quadracalc-mac                     # native macOS window
cargo run -p quadracalc-win                     # native Windows window
```

## Layout

| Crate | What it is |
|---|---|
| `quadracalc-core` | Everything that matters: the GridCraft `Session`, view state, and the quadraui surfaces. Backend-neutral. |
| `quadracalc` | Terminal binary: a few lines that hand the core to `quadraui::tui::run`. |
| `quadracalc-gtk`, `quadracalc-mac`, `quadracalc-win` | The same, for each native backend. |

## Licence

MIT OR Apache-2.0, at your option, the same as quadraui and GridCraft.
GridCraft's own licence and NOTICE files are in `third_party/gridcraft/` and
ship with release artifacts.
