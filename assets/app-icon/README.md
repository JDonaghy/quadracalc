# quadracalc app icon

Original artwork: four grid cells on a full-bleed tile, with the bottom-right
cell picked out as the active cell and a small fill handle at its corner.
Deliberately not modelled on any vendor's spreadsheet logo (no letters, no
"X", no overlapping panels) and distinct from GridCraft's own icon (see
`third_party/gridcraft/`) — this is quadracalc's own identity as a frontend,
not a reskin of the engine it drives.

## Palette

| Colour | Hex | Used for |
|---|---|---|
| Accent blue | `#2f6fb0` | The full-bleed tile |
| Paper | `#f3f6fa` | The three idle cells |
| Active amber | `#e8a33d` | The active cell |
| Ink | `#10243a` | The fill handle |

## Geometry

A 512 × 512 viewBox, clipped to a rounded square with `rx=112` — same
convention as GridCraft's own icon. macOS's `quadracalc-macos-512.png` and
`quadracalc.icns` render from a widened viewBox instead (`-62.136 -62.136
636.272 636.272`) so the artwork sits on Apple's 824/1024 icon grid with a
transparent margin, rather than filling the full tile the way Linux/Windows
icons would.

## Files

| File | Use |
|---|---|
| `quadracalc.svg` | Canonical artwork |
| `quadracalc-1024.png` | 1024 px render of the full tile |
| `quadracalc-macos-512.png` | Runtime Dock/app-switcher icon on macOS (`quadracalc-mac` embeds this via `RunConfig::with_app_icon`) |
| `quadracalc.icns` | macOS bundle icon (`CFBundleIconFile`, for a future `.app` bundle/packaging step) |
| `LICENSE.txt` | MIT OR Apache-2.0, same as the rest of this repository |

Linux (`hicolor` theme icons) and Windows (`.ico`) aren't generated yet —
`quadracalc-gtk` and `quadracalc-win` don't consume an app icon yet either;
add them alongside whichever issue wires those backends up.

## Regenerate

Edit `quadracalc.svg`, then re-render with `rsvg-convert` (`brew install
librsvg`) and, for the `.icns`, macOS's own `iconutil`:

```sh
DIR=assets/app-icon
SVG="$DIR/quadracalc.svg"
MAC=$(mktemp)
sed 's/viewBox="0 0 512 512"/viewBox="-62.136 -62.136 636.272 636.272"/' "$SVG" > "$MAC"

rsvg-convert -w 1024 -h 1024 "$SVG" -o "$DIR/quadracalc-1024.png"
rsvg-convert -w 512 -h 512 "$MAC" -o "$DIR/quadracalc-macos-512.png"

SET=$(mktemp -d)/quadracalc.iconset
mkdir -p "$SET"
for s in 16 32 128 256 512; do
  rsvg-convert -w "$s" -h "$s" "$MAC" -o "$SET/icon_${s}x${s}.png"
  rsvg-convert -w $((s * 2)) -h $((s * 2)) "$MAC" -o "$SET/icon_${s}x${s}@2x.png"
done
iconutil -c icns -o "$DIR/quadracalc.icns" "$SET"
```
