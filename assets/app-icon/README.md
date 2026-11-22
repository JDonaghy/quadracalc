# quadracalc app icon

A 2x2 grid ("quadra") with one selected cell and its fill handle, on a full-bleed
app-colour tile. Original geometry written by hand for this repository, deliberately
not modelled on any vendor's spreadsheet logo (no letters, no "X", no overlapping
panels) — see `CLAUDE.md`'s naming rule (don't use GridCraft's or ArtCraft's name or
logos as branding).

## Palette

Exactly three colours:

| Colour | Hex | Used for |
|---|---|---|
| Ink | `#101114` | Line work and contour |
| Paper | `#f4f1e8` | The card background |
| App colour (quadracalc blue) | `#2f5fa6` | The full-bleed tile and the selected cell |

## Geometry

A 512 x 512 viewBox, clipped to a rounded square with `rx=112`. Windows and Linux use
the full tile. The macOS icons (`.icns`, `quadracalc-macos-512.png`) sit on Apple's
grid: an 824/1024 body with a transparent margin.

## Files

| File | Use |
|---|---|
| `quadracalc.svg` | Canonical artwork; also `hicolor/scalable` |
| `quadracalc-1024.png` | 1024 px render of the full tile |
| `quadracalc-macos-512.png` | Runtime Dock icon on macOS, set via `RunConfig::with_app_icon` in `quadracalc-mac` |
| `quadracalc.icns` | macOS bundle icon (`CFBundleIconFile`), for future `.app` packaging |
| `quadracalc.ico` | Windows exe icon, 16-256 px |
| `hicolor/<size>/apps/io.github.jdonaghy.quadracalc.png` | Linux icon theme, 16-512 px |
| `hicolor/scalable/apps/io.github.jdonaghy.quadracalc.svg` | Linux scalable icon |
| `LICENSE.txt` | MIT OR Apache-2.0 |

The app ID is `io.github.jdonaghy.quadracalc`, matching the reverse-domain form
`quadraui::shell::ShellConfig::icon_name`'s own doc comment suggests for a GitHub-hosted
app with no registered domain.

## Regenerate

The raster/bundle files here are drawn directly (not rasterized from the SVG — no SVG
rasterizer was available when they were generated) by a small PIL script that mirrors
`quadracalc.svg`'s geometry at 4x supersampling before downscaling. If you change the
geometry, keep both in sync by hand, or write a proper `resvg`/`rsvg-convert`-based
regeneration step once one of those tools is available in the build environment.
