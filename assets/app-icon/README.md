# VectorCraft app icon

**Creature:** a regal engraved dragon, VectorCraft's mascot.

**Style:** an engraving-style portrait in the Crafting Apps owl template (the FilmCraft owl): a tight
head-and-shoulders portrait on a full-bleed field, no frame or roundel, with the body running off the tile.

**Palette** (exactly three colours):

| Colour | Hex | Used for |
|---|---|---|
| Ink | `#0b0b0c` | line work and contour |
| Paper | `#efe9dc` | the figure |
| VectorCraft colour | `#e8573f` | the full-bleed field |

**Tile:** `viewBox="0 0 512 512"`, a rounded square with `rx=112`, clipped. The macOS files add Apple's
transparent margin (tile = 824/1024 of the canvas); Windows and Linux use the tile edge to edge.

**Provenance:** the owner's original ArtCraft drawing, vectorised. `vectorcraft.svg` is the canonical
artwork. Licence: see `LICENSE.txt`.

## Files

| File | What | Used by |
|---|---|---|
| `vectorcraft.svg` | master vector (traced at 2048 px, about 700 KB) | source for everything below |
| `vectorcraft-small.svg` | lighter vector (traced at 1024 px, about 370 KB) | `hicolor/scalable` |
| `vectorcraft-1024.png` | full tile, 1024 px | docs, store listings |
| `vectorcraft-macos-512.png` | tile with Apple margin, 512 px | runtime Dock icon (`apps/vectorcraft/src/main.rs`) |
| `vectorcraft.icns` | macOS icon set, 16 to 1024 px | `cargo xtask bundle` (`CFBundleIconFile`) |
| `vectorcraft.ico` | Windows icon, 16 to 256 px | `apps/vectorcraft/build.rs` (embedded in the `.exe`) |
| `hicolor/<size>/apps/ai.storyteller.vectorcraft.png` | Linux theme icons, 16 to 512 px; the 256 one is also the runtime icon on Windows and Linux, the 128 one the brand mark in the app bar and About box (`crates/ui-egui/src/brand.rs`) | `packaging/linux/ai.storyteller.vectorcraft.desktop` |
| `hicolor/scalable/apps/ai.storyteller.vectorcraft.svg` | Linux scalable icon | as above |

## How it reaches each OS

- **macOS:** the app sets the Dock / app-switcher icon at runtime (`ViewportBuilder::with_icon`); the
  `.app` from `cargo xtask bundle` carries `vectorcraft.icns`.
- **Windows:** `build.rs` embeds `vectorcraft.ico` and VERSIONINFO with `winresource` (taskbar, Start
  menu, Explorer, Alt-Tab); the runtime icon covers the title bar.
- **Linux:** install `packaging/linux/ai.storyteller.vectorcraft.desktop` to `share/applications/` and
  `hicolor/` to `share/icons/`. The app sets its Wayland app ID / X11 class to
  `ai.storyteller.vectorcraft`, so the dock matches the window to the launcher.

## Regenerate

Edit or replace `vectorcraft.svg` (and `vectorcraft-small.svg`), then run `packaging/icons.sh`
(needs `resvg`; `iconutil` on macOS for the `.icns`). It writes every PNG, the `.ico` (via
`cargo xtask ico`) and the `.icns`.

## StitchCraft

`stitchcraft.svg` is StitchCraft's icon: a needle threaded with a loop of running stitches, drawn by
hand for StitchCraft in the same three-colour scheme (ink `#0b0b0c`, paper `#efe9dc`, and the
StitchCraft colour `#2f7d78` for the field) on the same 512 tile. `packaging/icons.sh stitchcraft`
renders `stitchcraft-1024.png`, `stitchcraft-macos-512.png` (runtime Dock icon,
`apps/stitchcraft/src/main.rs`), the `hicolor/…/ai.storyteller.stitchcraft.png` sizes (the 128 px one
is the brand mark, the 256 px one the runtime window icon), `stitchcraft.ico` (`apps/stitchcraft/build.rs`
and the MSI) and `stitchcraft.icns` (`cargo xtask bundle --app stitchcraft`); the scalable icon is
`stitchcraft.svg` itself.
Licence: MIT OR Apache-2.0, by the VectorCraft contributors (not covered by `LICENSE.txt`, which is
the dragon's).
